//! podrick — the Discord bot for a server I'm in.
//!
//! Job 1 announces a lift when one is published on benjisponge.com. Job 2
//! syncs and responds to Pants Off messages, seeded silently from the source
//! channel's complete history.
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
mod pants;

// The permanent-path format is a public URL contract shared with /lifting and
// the diary. Podrick reuses the implementation rather than restating it; it
// needs only `public_path` and `utc_timestamp` from the module.
#[allow(dead_code)]
#[path = "../lifting/archive/eastern.rs"]
mod eastern;

use announce::{Announcer, TickReport};
use discord::Discord;
use pants::{PantsTickReport, PantsWorker};

const DEFAULT_API: &str = "https://benjisponge.com";
const DEFAULT_INTERVAL_SECONDS: u64 = 60;
const TOKEN_VAR: &str = "DISCORD_BOT_TOKEN";
const LIFT_CHANNEL_VAR: &str = "PODRICK_LIFT_CHANNEL_ID";
const PANTS_CHANNEL_VAR: &str = "PODRICK_PANTS_CHANNEL_ID";
const INFARCTIONS_CHANNEL_VAR: &str = "PODRICK_INFARCTIONS_CHANNEL_ID";

const USAGE: &str = "\
podrick — Discord bot for benjisponge.com

USAGE
  podrick <COMMAND> [FLAGS]        (or: cargo run --bin podrick -- <COMMAND>)

COMMANDS
  run                   poll forever (the deployed mode)
  once                  run a single pass and exit

FLAGS
  --dry-run             read-only: preview work, post/react/write nothing.
                        Pants history reads still need a token.
  --interval <seconds>  poll interval for `run` (default: 60, minimum: 5)
  --api <origin>        site API origin (default: https://benjisponge.com)
  --token <token>       bot token; otherwise $DISCORD_BOT_TOKEN, otherwise
                        ~/.config/benjisponge/podrick.token
  -h, --help            this text

ENVIRONMENT
  DISCORD_BOT_TOKEN         bot token from the Discord developer portal
  PODRICK_LIFT_CHANNEL_ID   optional lift-announcement channel
  PODRICK_PANTS_CHANNEL_ID  optional Pants Off source channel
  PODRICK_INFARCTIONS_CHANNEL_ID
                            infarction output; required with Pants source
  SURREALDB_*               the same five connection variables the site uses

BEHAVIOR
  The first run records a watermark at the newest workout that already exists
  and announces nothing — existing history is never backfilled into a channel.
  Only manually published workouts newer than that watermark are announced, in
  the order they happened.

  Each announcement is claimed create-only by workout id before it is posted,
  so competing first claims converge. A claim whose post never confirmed is
  retried on the next pass; keep the deployed worker at one replica because
  retries are not leased between workers.

  Pants Off's first run walks the source channel's complete history into the
  database without reacting or reporting historical infarctions. Live messages
  are classified in America/New_York: 6:07 AM/PM claims a slot; another
  HH:07 is out of town; any other minute is an infarction. Worm reactions and
  infarction posts are claimed before Discord is called and retried until
  confirmed.

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
    optional_env(variable).ok_or_else(|| format!("{variable} is not set"))
}

fn optional_env(variable: &str) -> Option<String> {
    std::env::var(variable)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

#[derive(Default)]
struct PassReport {
    announcements: TickReport,
    pants: PantsTickReport,
}

impl PassReport {
    fn is_quiet(&self) -> bool {
        self.announcements.is_quiet() && self.pants.is_quiet()
    }

    fn retry_after(&self) -> Option<Duration> {
        match (self.announcements.retry_after, self.pants.retry_after) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (Some(after), None) | (None, Some(after)) => Some(after),
            (None, None) => None,
        }
    }
}

fn log_report(report: &PassReport, dry_run: bool) {
    if let Some(watermark) = &report.announcements.seeded_watermark {
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
    for id in &report.announcements.announced {
        log("announced", serde_json::json!({ "workout": id }));
    }
    for id in &report.announcements.retried {
        log(
            "announced-after-retry",
            serde_json::json!({ "workout": id }),
        );
    }
    for failure in &report.announcements.failed {
        log(
            "announce-failed",
            serde_json::json!({ "detail": failure, "note": "will retry next pass" }),
        );
    }
    if report.pants.history_scanned > 0 {
        log(
            "pants-history",
            serde_json::json!({
                "messages_scanned": report.pants.history_scanned,
                "participant_messages": report.pants.history_stored,
                "complete": report.pants.history_complete,
                "written": !dry_run,
            }),
        );
    } else if report.pants.history_complete {
        log(
            "pants-history-complete",
            serde_json::json!({ "written": !dry_run }),
        );
    }
    if report.pants.live_stored > 0 {
        log(
            "pants-synced",
            serde_json::json!({
                "participant_messages": report.pants.live_stored,
                "written": !dry_run,
            }),
        );
    }
    for detail in &report.pants.infarctions {
        log(
            if dry_run {
                "pants-infarction-preview"
            } else {
                "pants-infarction-posted"
            },
            serde_json::json!({ "detail": detail }),
        );
    }
    for detail in &report.pants.worms {
        log(
            if dry_run {
                "pants-worm-preview"
            } else {
                "pants-wormed"
            },
            serde_json::json!({ "detail": detail }),
        );
    }
    for detail in &report.pants.skipped {
        log(
            "pants-action-skipped",
            serde_json::json!({ "detail": detail }),
        );
    }
    for failure in &report.pants.failed {
        log(
            "pants-failed",
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

    let lift_channel = optional_env(LIFT_CHANNEL_VAR);
    let pants_channels = match (
        optional_env(PANTS_CHANNEL_VAR),
        optional_env(INFARCTIONS_CHANNEL_VAR),
    ) {
        (None, None) => None,
        (Some(source), Some(infarctions)) => Some((source, infarctions)),
        (Some(_), None) => {
            eprintln!(
                "podrick: {INFARCTIONS_CHANNEL_VAR} is required when {PANTS_CHANNEL_VAR} is set"
            );
            return ExitCode::from(2);
        }
        (None, Some(_)) => {
            eprintln!(
                "podrick: {PANTS_CHANNEL_VAR} is required when {INFARCTIONS_CHANNEL_VAR} is set"
            );
            return ExitCode::from(2);
        }
    };
    if lift_channel.is_none() && pants_channels.is_none() {
        eprintln!("podrick: configure {LIFT_CHANNEL_VAR}, or both Pants Off channel variables");
        return ExitCode::from(2);
    }

    // A lift-only dry run never calls Discord and can be used before the
    // application exists. Pants Off must authenticate even to read history.
    let token_required = !args.dry_run || pants_channels.is_some();
    let token = match (token_required, resolve_token(args.token)) {
        (_, Ok(token)) => token,
        (false, Err(_)) => String::new(),
        (true, Err(error)) => {
            eprintln!("podrick: {error}");
            return ExitCode::from(2);
        }
    };
    let discord = Discord::new(token);
    let announcer = lift_channel.as_ref().map(|channel_id| Announcer {
        discord: discord.clone(),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("podrick (+https://benjisponge.com/podrick)")
            .build()
            .expect("reqwest client"),
        api_origin: args.api.clone(),
        channel_id: channel_id.clone(),
        dry_run: args.dry_run,
    });
    let pants_worker = pants_channels
        .as_ref()
        .map(|(channel_id, infarctions_channel_id)| PantsWorker {
            discord,
            channel_id: channel_id.clone(),
            infarctions_channel_id: infarctions_channel_id.clone(),
            dry_run: args.dry_run,
        });
    let data = Data::from_env();

    log(
        "starting",
        serde_json::json!({
            "mode": if args.command == Command::Run { "run" } else { "once" },
            "api": args.api,
            "lift_channel": lift_channel,
            "pants_channel": pants_channels.as_ref().map(|channels| &channels.0),
            "infarctions_channel": pants_channels.as_ref().map(|channels| &channels.1),
            "dry_run": args.dry_run,
            "interval_seconds": args.interval.as_secs(),
        }),
    );

    if args.command == Command::Once {
        return match run_pass(&data, announcer.as_ref(), pants_worker.as_ref()).await {
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
        let delay = match run_pass(&data, announcer.as_ref(), pants_worker.as_ref()).await {
            Ok(report) => {
                let delay = report
                    .retry_after()
                    .map_or(args.interval, |after| args.interval.max(after));
                log_report(&report, args.dry_run);
                delay
            }
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
        };
        tokio::time::sleep(delay).await;
    }
}

/// One pass, with database connection errors treated as transient.
///
/// `Data::db()` does not cache a failed initialization, so a database that is
/// merely restarting resolves itself on the next pass rather than killing the
/// worker.
async fn run_pass(
    data: &Data,
    announcer: Option<&Announcer>,
    pants_worker: Option<&PantsWorker>,
) -> Result<PassReport, Box<dyn std::error::Error>> {
    let handle = match data.db().await {
        Ok(handle) => handle,
        Err(error) => {
            log(
                "database-unavailable",
                serde_json::json!({ "error": error.to_string(), "note": "will retry next pass" }),
            );
            return Ok(PassReport::default());
        }
    };
    let now = now_seconds();
    let announcements = match announcer {
        Some(announcer) => announcer.tick(&handle, now).await?,
        None => TickReport::default(),
    };
    // A 429 can be route-specific or global. Without guessing which bucket
    // Discord applied, make no more Discord calls in this pass and honor the
    // full delay before resuming either job.
    let pants = match (announcements.retry_after, pants_worker) {
        (None, Some(worker)) => worker.tick(&handle, now).await?,
        _ => PantsTickReport::default(),
    };
    Ok(PassReport {
        announcements,
        pants,
    })
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
        let report = PassReport {
            announcements: TickReport {
                seeded_watermark: Some("2026-07-28 22:30:00".to_string()),
                ..TickReport::default()
            },
            ..PassReport::default()
        };
        assert!(!report.is_quiet());
    }

    #[test]
    fn the_longest_discord_retry_hint_controls_the_next_pass() {
        let report = PassReport {
            announcements: TickReport {
                retry_after: Some(Duration::from_secs(30)),
                ..TickReport::default()
            },
            pants: PantsTickReport {
                retry_after: Some(Duration::from_secs(1_337)),
                ..PantsTickReport::default()
            },
        };
        assert_eq!(report.retry_after(), Some(Duration::from_secs(1_337)));
    }
}
