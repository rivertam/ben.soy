//! podrick — the Discord bot for a server I'm in.
//!
//! Job 1 announces a lift when one is published on ben.soy. Job 2
//! syncs and responds to Pants Off messages, seeded silently from the source
//! channel's complete history.
//!
//! Runs as its own Railway service from the same image as the site
//! (`docs/podrick.md`). It reads the site's public API for message content and
//! owns the `podrick_*` tables directly; it never writes a fitness table.
//!
//! Discord is deliberately REST-only — no gateway, no serenity. Fitness
//! announcements are woken by a SurrealDB live query and reconciled from
//! durable database state. See `discord.rs` and `docs/podrick.md`.

use futures_util::StreamExt;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use benjisponge::data::{Data, ENDPOINT_VAR};
use surrealdb::types::Action;

mod announce;
mod db;
mod discord;
mod pants;
mod seed_install;

// The permanent-path format is a public URL contract shared with /fitness and
// the diary. Podrick reuses the implementation rather than restating it; it
// needs only `public_path` and `utc_timestamp`. The module lives in
// diary-core now (the diary's wasm worker compiles it too), which also ends
// this binary's old `#[path]` re-mount.
use diary_core::eastern;

use announce::{Announcer, TickReport};
use discord::Discord;
use pants::{PantsTickReport, PantsWorker};
use seed_install::SeedReport;

const DEFAULT_API: &str = "https://ben.soy";
const DEFAULT_INTERVAL_SECONDS: u64 = 60;
/// Give the web process time to rebuild its post-import fitness snapshot before
/// the announcer asks its workout-detail route for the newly committed lift.
const FITNESS_LIVE_SETTLE_SECONDS: u64 = 2;
const FITNESS_LIVE_RECONNECT_SECONDS: u64 = 1;
const TOKEN_VAR: &str = "DISCORD_BOT_TOKEN";
const LIFT_CHANNEL_VAR: &str = "PODRICK_LIFT_CHANNEL_ID";
const PANTS_CHANNEL_VAR: &str = "PODRICK_PANTS_CHANNEL_ID";
const INFARCTIONS_CHANNEL_VAR: &str = "PODRICK_INFARCTIONS_CHANNEL_ID";

const USAGE: &str = "\
podrick — Discord bot for ben.soy

USAGE
  podrick <COMMAND> [FLAGS]        (or: cargo run --bin podrick -- <COMMAND>)

COMMANDS
  run                   watch fitness and poll Pants forever (deployed mode)
  once                  run a single pass and exit

FLAGS
  --dry-run             read-only: preview work, post/react/write nothing.
                        Pants history reads still need a token.
  --interval <seconds>  Pants poll and safety-reconciliation interval
                        (default: 60, minimum: 5)
  --api <origin>        site API origin (default: https://ben.soy)
  --token <token>       bot token; otherwise $DISCORD_BOT_TOKEN, otherwise
                        ~/.config/benjisponge/podrick.token
  -h, --help            this text

ENVIRONMENT
  DISCORD_BOT_TOKEN         bot token from the Discord developer portal
  PODRICK_LIFT_CHANNEL_ID   optional lift-announcement channel
  PODRICK_PANTS_CHANNEL_ID  optional Pants Off source channel
  PODRICK_INFARCTIONS_CHANNEL_ID
                            infarction output; required with Pants source
  PODRICK_SEED_URL          optional; when set with PODRICK_SYNC_TOKEN and
                            local podrick_* tables are empty, install that
                            full production snapshot before normal work
  PODRICK_SYNC_TOKEN        Bearer for PODRICK_SEED_URL
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
  confirmed. When PODRICK_SEED_URL is set, an empty local Podrick database
  installs production's full podrick_* snapshot first (skipping Discord history
  when pants_cursor is included).

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

fn supports_fitness_live(endpoint: Option<&str>) -> bool {
    endpoint
        .and_then(|endpoint| url::Url::parse(endpoint).ok())
        .is_some_and(|endpoint| matches!(endpoint.scheme(), "ws" | "wss"))
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
    seed: SeedReport,
    announcements: TickReport,
    pants: PantsTickReport,
}

impl PassReport {
    fn is_quiet(&self) -> bool {
        self.seed.is_quiet() && self.announcements.is_quiet() && self.pants.is_quiet()
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
    if !report.seed.is_quiet() {
        log(
            "podrick-seeded",
            serde_json::json!({
                "announcements": report.seed.announcements,
                "pants_messages": report.seed.pants_messages,
                "pants_actions": report.seed.pants_actions,
                "meta": report.seed.meta,
                "written": !dry_run,
            }),
        );
    }
    if let Some(watermark) = &report.announcements.seeded_watermark {
        log(
            "watermark-seeded",
            serde_json::json!({
                // Named for its timezone because the watermark lands exactly ON
                // the newest workout, and /fitness shows that workout in
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
            .user_agent("podrick (+https://ben.soy/podrick)")
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
    let fitness_live = supports_fitness_live(optional_env(ENDPOINT_VAR).as_deref());

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
            "fitness_live": announcer.is_some() && fitness_live,
        }),
    );

    if args.command == Command::Once {
        return match run_pass(
            &data,
            announcer.as_ref(),
            pants_worker.as_ref(),
            args.dry_run,
        )
        .await
        {
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

    match run_forever(
        &data,
        announcer.as_ref(),
        pants_worker.as_ref(),
        args.dry_run,
        args.interval,
        fitness_live,
    )
    .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Only unrecoverable conditions reach here: a rejected token, a
            // channel the bot cannot post in, or a database query that failed
            // after connection. Exiting lets the restart policy and the logs
            // show it instead of a silent loop that never posts.
            eprintln!("podrick: {error}");
            log(
                "stopping",
                serde_json::json!({ "error": error.to_string() }),
            );
            ExitCode::FAILURE
        }
    }
}

/// Run scheduled reconciliation, with manual workout creates able to wake the
/// lift announcer between intervals.
///
/// A live notification is deliberately not work itself. The stream is opened
/// before each catch-up pass, and `Announcer::tick` still reads the durable
/// watermark, unconfirmed claims, and unclaimed workouts. If the pinned SDK
/// closes the stream on a WebSocket reset, dropping it and returning to the
/// top of this loop resubscribes before doing another catch-up pass.
async fn run_forever(
    data: &Data,
    announcer: Option<&Announcer>,
    pants_worker: Option<&PantsWorker>,
    dry_run: bool,
    interval: Duration,
    fitness_live: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut workout_changes = None;

    if announcer.is_some() && !fitness_live {
        log(
            "fitness-live-unavailable",
            serde_json::json!({
                "error": format!("{ENDPOINT_VAR} must use ws:// or wss://"),
                "note": "using interval reconciliation"
            }),
        );
    }

    loop {
        if fitness_live && workout_changes.is_none() && announcer.is_some() {
            workout_changes = match data.db().await {
                Ok(handle) => match db::watch_manual_workouts(&handle).await {
                    Ok(changes) => {
                        log(
                            "fitness-live-subscribed",
                            serde_json::json!({ "source": db::ANNOUNCED_SOURCE }),
                        );
                        Some(changes)
                    }
                    Err(error) => {
                        log(
                            "fitness-live-unavailable",
                            serde_json::json!({
                                "error": error.to_string(),
                                "note": "using interval reconciliation; will retry"
                            }),
                        );
                        None
                    }
                },
                Err(error) => {
                    log(
                        "fitness-live-unavailable",
                        serde_json::json!({
                            "error": error.to_string(),
                            "note": "using interval reconciliation; will retry"
                        }),
                    );
                    None
                }
            };
        }

        let delay = match run_pass(data, announcer, pants_worker, dry_run).await {
            Ok(report) => {
                let delay = report
                    .retry_after()
                    .map_or(interval, |after| interval.max(after));
                let rate_limited = report.retry_after().is_some();
                log_report(&report, dry_run);
                if rate_limited {
                    tokio::time::sleep(delay).await;
                    continue;
                }
                delay
            }
            Err(error) => return Err(error),
        };

        let Some(changes) = workout_changes.as_mut() else {
            tokio::time::sleep(delay).await;
            continue;
        };
        let deadline = tokio::time::Instant::now() + delay;
        let mut reconnect = false;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                change = changes.next() => match change {
                    Some(Ok(change)) if change.action == Action::Create => {
                        // The workout and its sets committed atomically, but the
                        // web process rebuilds its read snapshot just after that
                        // commit. A short settle avoids racing the detail route.
                        tokio::time::sleep(Duration::from_secs(
                            FITNESS_LIVE_SETTLE_SECONDS,
                        ))
                        .await;
                        let report = run_announcement_pass(
                            data,
                            announcer.expect("live stream requires announcer"),
                        )
                        .await?;
                        let retry_after = report.retry_after;
                        log_report(
                            &PassReport {
                                announcements: report,
                                ..PassReport::default()
                            },
                            dry_run,
                        );
                        if let Some(after) = retry_after {
                            // A Discord 429 may be global. Pause every Discord
                            // path, including Pants, for the full advertised
                            // delay just as the interval pass does.
                            tokio::time::sleep(interval.max(after)).await;
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        log(
                            "fitness-live-disconnected",
                            serde_json::json!({
                                "error": error.to_string(),
                                "note": "will resubscribe and reconcile"
                            }),
                        );
                        reconnect = true;
                        break;
                    }
                    None => {
                        log(
                            "fitness-live-disconnected",
                            serde_json::json!({
                                "error": "stream ended",
                                "note": "will resubscribe and reconcile"
                            }),
                        );
                        reconnect = true;
                        break;
                    }
                }
            }
        }

        if reconnect {
            workout_changes = None;
            tokio::time::sleep(Duration::from_secs(FITNESS_LIVE_RECONNECT_SECONDS)).await;
        }
    }
}

async fn run_announcement_pass(
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

/// One pass, with database connection errors treated as transient.
///
/// `Data::db()` does not cache a failed initialization, so a database that is
/// merely restarting resolves itself on the next pass rather than killing the
/// worker.
async fn run_pass(
    data: &Data,
    announcer: Option<&Announcer>,
    pants_worker: Option<&PantsWorker>,
    dry_run: bool,
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
    let pants_channel = pants_worker.map(|worker| worker.channel_id.as_str());
    let seed = seed_install::maybe_install_from_api(&handle, dry_run, pants_channel)
        .await?
        .unwrap_or_default();
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
        seed,
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
            ..PassReport::default()
        };
        assert_eq!(report.retry_after(), Some(Duration::from_secs(1_337)));
    }

    #[test]
    fn fitness_live_requires_a_websocket_endpoint() {
        assert!(supports_fitness_live(Some("ws://database.internal:8000")));
        assert!(supports_fitness_live(Some("wss://database.example")));
        assert!(!supports_fitness_live(Some(
            "http://database.internal:8000"
        )));
        assert!(!supports_fitness_live(None));
    }
}
