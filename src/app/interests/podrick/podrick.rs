//! podrick — the Discord bot for a server I'm in.
//!
//! Job 1 (here): announce a lift in a channel when one is published on
//! benjisponge.com. Job 2 (later): manage database records in response to
//! posts in another channel, seeded from that channel's history.
//!
//! Runs as its own Railway service from the same image as the site
//! (`docs/podrick.md`). It reads the site's public API for message content and
//! owns the `podrick_*` tables directly; it never writes a fitness table.
//!
//! Deliberately REST-only — no gateway, no serenity. See `discord.rs`.

use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use benjisponge::data::Data;

mod announce;
mod db;
mod discord;

// The permanent-path format is a public URL contract shared with /lifting and
// the diary. Podrick reuses the implementation rather than restating it; it
// needs only `public_path` and `utc_timestamp` from the module.
#[allow(dead_code)]
#[path = "../lifting/archive/eastern.rs"]
mod eastern;

use announce::{Announcer, TickReport};
use discord::Discord;

const DEFAULT_API: &str = "https://benjisponge.com";
const DEFAULT_INTERVAL_SECONDS: u64 = 60;
const TOKEN_VAR: &str = "DISCORD_BOT_TOKEN";
const LIFT_CHANNEL_VAR: &str = "PODRICK_LIFT_CHANNEL_ID";

const USAGE: &str = "\
podrick — Discord bot for benjisponge.com

USAGE
  podrick <COMMAND> [FLAGS]        (or: cargo run --bin podrick -- <COMMAND>)

COMMANDS
  run                   poll forever, announcing new lifts (the deployed mode)
  once                  run a single pass and exit

FLAGS
  --dry-run             read-only: render messages to stdout, post nothing,
                        write nothing. Needs no token.
  --interval <seconds>  poll interval for `run` (default: 60, minimum: 5)
  --api <origin>        site API origin (default: https://benjisponge.com)
  --token <token>       bot token; otherwise $DISCORD_BOT_TOKEN, otherwise
                        ~/.config/benjisponge/podrick.token
  -h, --help            this text

ENVIRONMENT
  DISCORD_BOT_TOKEN         bot token from the Discord developer portal
  PODRICK_LIFT_CHANNEL_ID   channel id for lift announcements
  SURREALDB_*               the same five connection variables the site uses

BEHAVIOR
  The first run records a watermark at the newest workout that already exists
  and announces nothing — existing history is never backfilled into a channel.
  Only manually published workouts newer than that watermark are announced, in
  the order they happened.

  Each announcement is claimed create-only by workout id before it is posted,
  so a crash, a redeploy mid-post, or two workers cannot produce a duplicate.
  A claim whose post never confirmed is retried on the next pass.

  Exit codes: 0 success, 1 failure (unreachable database, rejected token,
  missing channel permission), 2 usage error.
";

struct Args {
    command: Command,
    api: String,
    interval: Duration,
    dry_run: bool,
    token: Option<String>,
}

#[derive(PartialEq, Eq)]
enum Command {
    Run,
    Once,
}

fn parse_args() -> Result<Args, String> {
    let mut command = None;
    let mut api = DEFAULT_API.to_string();
    let mut interval = Duration::from_secs(DEFAULT_INTERVAL_SECONDS);
    let mut dry_run = false;
    let mut token = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "run" => command = Some(Command::Run),
            "once" => command = Some(Command::Once),
            "--dry-run" => dry_run = true,
            "--interval" => {
                let value = args.next().ok_or("--interval needs a value")?;
                let seconds: u64 = value.parse().map_err(|_| {
                    format!("--interval must be a whole number of seconds: {value}")
                })?;
                // A tighter loop would only add API and Discord traffic; lifts
                // are published by hand, minutes apart at closest.
                interval = Duration::from_secs(seconds.max(5));
            }
            "--api" => {
                api = args
                    .next()
                    .ok_or("--api needs a value")?
                    .trim_end_matches('/')
                    .to_string();
            }
            "--token" => token = Some(args.next().ok_or("--token needs a value")?),
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other} (see --help)")),
        }
    }
    let command = command.ok_or("expected `run` or `once` (see --help)")?;
    Ok(Args {
        command,
        api,
        interval,
        dry_run,
        token,
    })
}

/// `--token`, then the environment, then the same `~/.config/benjisponge`
/// directory the Spire and fitness sync clients read their tokens from.
fn resolve_token(flag: Option<String>) -> Result<String, String> {
    if let Some(token) = flag {
        return Ok(token);
    }
    if let Ok(token) = required_env(TOKEN_VAR) {
        return Ok(token);
    }
    let path = std::env::var("HOME")
        .map(|home| std::path::PathBuf::from(home).join(".config/benjisponge/podrick.token"))
        .map_err(|_| "HOME is not set".to_string())?;
    let token = std::fs::read_to_string(&path)
        .map_err(|error| {
            format!(
                "{TOKEN_VAR} is not set and {} is unreadable: {error}",
                path.display()
            )
        })?
        .trim()
        .to_string();
    if token.is_empty() {
        return Err(format!("{} is empty", path.display()));
    }
    Ok(token)
}

fn required_env(variable: &str) -> Result<String, String> {
    std::env::var(variable)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{variable} is not set"))
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

fn log(event: &str, fields: serde_json::Value) {
    let mut entry = serde_json::json!({ "bot": "podrick", "event": event });
    if let (Some(entry), Some(fields)) = (entry.as_object_mut(), fields.as_object()) {
        for (key, value) in fields {
            entry.insert(key.clone(), value.clone());
        }
    }
    println!("{entry}");
}

fn log_report(report: &TickReport, dry_run: bool) {
    if let Some(watermark) = &report.seeded_watermark {
        log(
            "watermark-seeded",
            serde_json::json!({
                // Named for its timezone because the watermark lands exactly ON
                // the newest workout, and /lifting shows that workout in
                // Eastern — so a reader comparing the two does not recognize
                // the lift sitting at the boundary as the one being excluded.
                "watermark_utc": watermark,
                "written": !dry_run,
                "note": if dry_run {
                    "dry run: nothing was written, this is the value a real run would seed"
                } else {
                    "the newest workout already present; it and everything older \
                     are history and are never announced"
                },
            }),
        );
    }
    for id in &report.announced {
        log("announced", serde_json::json!({ "workout": id }));
    }
    for id in &report.retried {
        log(
            "announced-after-retry",
            serde_json::json!({ "workout": id }),
        );
    }
    for failure in &report.failed {
        log(
            "announce-failed",
            serde_json::json!({ "detail": failure, "note": "will retry next pass" }),
        );
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("podrick: {error}");
            return ExitCode::from(2);
        }
    };

    // The token is not needed for a dry run, so `--dry-run` works before the
    // Discord application exists — useful while wiring this up.
    let token = match (args.dry_run, resolve_token(args.token.clone())) {
        (_, Ok(token)) => token,
        (true, Err(_)) => String::new(),
        (false, Err(error)) => {
            eprintln!("podrick: {error}");
            return ExitCode::from(2);
        }
    };
    let channel_id = match required_env(LIFT_CHANNEL_VAR) {
        Ok(channel_id) => channel_id,
        Err(error) => {
            eprintln!("podrick: {error}");
            return ExitCode::from(2);
        }
    };

    let announcer = Announcer {
        discord: Discord::new(token),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("podrick (+https://benjisponge.com/podrick)")
            .build()
            .expect("reqwest client"),
        api_origin: args.api.clone(),
        channel_id: channel_id.clone(),
        dry_run: args.dry_run,
    };
    let data = Data::from_env();

    log(
        "starting",
        serde_json::json!({
            "mode": if args.command == Command::Run { "run" } else { "once" },
            "api": args.api,
            "channel": channel_id,
            "dry_run": args.dry_run,
            "interval_seconds": args.interval.as_secs(),
        }),
    );

    if args.command == Command::Once {
        return match run_pass(&data, &announcer).await {
            Ok(report) => {
                log_report(&report, args.dry_run);
                // A single pass is run by a human, so say so explicitly rather
                // than exiting silently: "nothing to announce" and "the bot is
                // broken" should not look the same.
                if report.is_quiet() {
                    log(
                        "idle",
                        serde_json::json!({ "note": "nothing new to announce" }),
                    );
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("podrick: {error}");
                ExitCode::FAILURE
            }
        };
    }

    loop {
        match run_pass(&data, &announcer).await {
            Ok(report) => log_report(&report, args.dry_run),
            Err(error) => {
                // Only unrecoverable conditions reach here: a rejected token,
                // a channel the bot cannot post in, or a database that stayed
                // unreachable. Exiting lets the restart policy and the logs
                // show it instead of a silent loop that never posts.
                eprintln!("podrick: {error}");
                log(
                    "stopping",
                    serde_json::json!({ "error": error.to_string() }),
                );
                return ExitCode::FAILURE;
            }
        }
        tokio::time::sleep(args.interval).await;
    }
}

/// One pass, with database connection errors treated as transient.
///
/// `Data::db()` does not cache a failed initialization, so a database that is
/// merely restarting resolves itself on the next pass rather than killing the
/// worker.
async fn run_pass(
    data: &Data,
    announcer: &Announcer,
) -> Result<TickReport, Box<dyn std::error::Error>> {
    let handle = match data.db().await {
        Ok(handle) => handle,
        Err(error) => {
            log(
                "database-unavailable",
                serde_json::json!({ "error": error.to_string(), "note": "will retry next pass" }),
            );
            return Ok(TickReport::default());
        }
    };
    Ok(announcer.tick(&handle, now_seconds()).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_path_round_trips_through_the_shared_eastern_module() {
        let instant = eastern::EasternInstant {
            local: "2026-07-28 18:30:00".to_string(),
            offset_minutes: -240,
        };
        let path = eastern::public_path(&instant);
        assert_eq!(path, "2026-07-28T18-30-00-04-00");
        assert_eq!(eastern::parse_public_path(&path), Some(instant));
    }

    #[test]
    fn a_seeded_watermark_is_reported_as_activity() {
        let report = TickReport {
            seeded_watermark: Some("2026-07-28 22:30:00".to_string()),
            ..TickReport::default()
        };
        assert!(!report.is_quiet());
    }
}
