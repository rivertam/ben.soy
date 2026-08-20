//! spire_sync — upload Slay the Spire 1 and 2 run history to ben.soy.
//!
//! Reads every `<epoch>.run` file either game (or Steam Cloud) has on this
//! machine, asks the site which run ids it already has, and POSTs only the
//! missing ones. Idempotent by construction: the id is the run file's stem,
//! the server skips stored game/id pairs, and re-running is always safe. Designed to
//! be driven by a human, a cron job, or an LLM agent — see `--help`.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Deserialize;
use serde_json::{Value, json};

const DEFAULT_API: &str = "https://ben.soy";
const STS2_STEAM_APP_ID: &str = "2868840";
const CHUNK: usize = 20;

const USAGE: &str = "\
spire_sync — sync Slay the Spire 1 and 2 runs to ben.soy

USAGE
  just sync-spire [FLAGS]            (or: cargo run --bin spire_sync -- [FLAGS])

FLAGS
  --dry-run             scan and diff, but upload nothing
  --json                machine-readable summary on stdout (for agents)
  --api <origin>        API origin (default: https://ben.soy)
  --history-dir <path>  extra run-history root; repeatable. Its files are
                        identified as StS 1 or 2 from their JSON shape.
                        Defaults cover both games' standard Linux paths.
  --token <token>       write token; otherwise $SPIRE_SYNC_TOKEN, otherwise
                        ~/.config/benjisponge/spire.token
  -h, --help            this text

BEHAVIOR
  1. Recursively collect *.run files (JSON) from every history root; the file
     stem is the run id. Identity is the game plus that id.
  2. GET  <api>/api/spire/ids to learn which ids the database already has.
  3. POST <api>/api/spire/runs (Bearer token) with the missing runs, oldest
     first, in chunks. The server ignores duplicates and reports added counts.

  Deploy the matching server/schema before using this client; legacy APIs are
  rejected so game identities cannot be silently lost.

  A read-only diff (--dry-run) needs no token. Exit codes: 0 success (even
  when there was nothing to upload), 1 failure (unreachable API, bad token,
  rejected upload), 2 usage error.
";

struct Args {
    api: String,
    dry_run: bool,
    json: bool,
    dirs: Vec<PathBuf>,
    token: Option<String>,
}

struct HistoryDiscovery {
    dirs: Vec<PathBuf>,
    missing_sts1_history: Vec<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut parsed = Args {
        api: DEFAULT_API.to_string(),
        dry_run: false,
        json: false,
        dirs: Vec::new(),
        token: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => parsed.dry_run = true,
            "--json" => parsed.json = true,
            "--api" => {
                parsed.api = args
                    .next()
                    .ok_or("--api needs a value")?
                    .trim_end_matches('/')
                    .to_string();
            }
            "--history-dir" => {
                parsed.dirs.push(PathBuf::from(
                    args.next().ok_or("--history-dir needs a value")?,
                ));
            }
            "--token" => parsed.token = Some(args.next().ok_or("--token needs a value")?),
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown flag: {other} (see --help)")),
        }
    }
    Ok(parsed)
}

// ---------------------------------------------------------------------------
// Discovery: where run files live on this machine.

fn subdirs(path: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn profile_history_dirs(parent: &Path, found: &mut Vec<PathBuf>) {
    for profile in subdirs(parent) {
        let is_profile = profile
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("profile"));
        if !is_profile {
            continue;
        }
        let history = profile.join("saves/history");
        if history.is_dir() {
            found.push(history);
        }
    }
}

fn steam_library_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![home.join(".local/share/Steam"), home.join(".steam/steam")];
    for steam_root in roots.clone() {
        let manifest = steam_root.join("steamapps/libraryfolders.vdf");
        let Ok(vdf) = fs::read_to_string(manifest) else {
            continue;
        };
        roots.extend(vdf.lines().filter_map(|line| {
            let mut quoted = line.split('"');
            (quoted.nth(1) == Some("path"))
                .then(|| quoted.nth(1))
                .flatten()
                .map(PathBuf::from)
        }));
    }
    let mut seen = HashSet::new();
    roots.retain(|path| {
        let identity = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        seen.insert(identity)
    });
    roots
}

fn sts1_history_dirs(install: &Path) -> Vec<PathBuf> {
    subdirs(install)
        .into_iter()
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy().to_ascii_lowercase();
                name.starts_with("runs") || name.starts_with("betaruns")
            })
        })
        .collect()
}

fn default_history_dirs() -> HistoryDiscovery {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return HistoryDiscovery {
            dirs: Vec::new(),
            missing_sts1_history: Vec::new(),
        };
    };
    let mut dirs = Vec::new();
    let mut missing_sts1_history = Vec::new();
    // StS 1 stores history below the game install rather than Steam userdata.
    for library in steam_library_roots(&home) {
        let install = library.join("steamapps/common/SlayTheSpire");
        if install.is_dir() {
            let histories = sts1_history_dirs(&install);
            if histories.is_empty() {
                missing_sts1_history.push(install);
            } else {
                dirs.extend(histories);
            }
        }
    }
    // The game's own data dir wins ties with the cloud mirror (listed first).
    for account in subdirs(&home.join(".local/share/SlayTheSpire2/steam")) {
        profile_history_dirs(&account, &mut dirs);
    }
    for account in subdirs(&home.join(".local/share/Steam/userdata")) {
        profile_history_dirs(&account.join(STS2_STEAM_APP_ID).join("remote"), &mut dirs);
    }
    let mut seen = HashSet::new();
    dirs.retain(|path| {
        let identity = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        seen.insert(identity)
    });
    HistoryDiscovery {
        dirs,
        missing_sts1_history,
    }
}

// ---------------------------------------------------------------------------
// Extraction: one .run file (SerializableRun JSON) → the API's run shape.

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Game {
    Sts1,
    Sts2,
}

impl Game {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sts1 => "sts1",
            Self::Sts2 => "sts2",
        }
    }
}

struct SyncRun {
    id: String,
    game: Game,
    date: String,
    start_time: i64,
    character: String,
    win: bool,
    abandoned: bool,
    ascension: u64,
    acts: u64,
    floors: u64,
    killed_by: Option<String>,
    kill_kind: Option<String>,
    run_time: u64,
    seed: String,
    game_mode: String,
    build_id: String,
    raw: String,
}

fn extract_run(id: &str, raw: &str) -> Result<SyncRun, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("bad json: {e}"))?;
    if v.get("character_chosen").is_some() {
        extract_sts1_run(id, raw, &v)
    } else if v.get("players").is_some() {
        extract_sts2_run(id, raw, &v)
    } else {
        Err("unrecognized Slay the Spire run format".to_string())
    }
}

fn extract_sts2_run(id: &str, raw: &str, v: &Value) -> Result<SyncRun, String> {
    let start_time = v
        .get("start_time")
        .and_then(Value::as_i64)
        .ok_or("missing start_time")?;
    let character = v
        .get("players")
        .and_then(Value::as_array)
        .map(|players| {
            players
                .iter()
                .filter_map(|p| p.get("character").and_then(Value::as_str))
                .map(pretty_character)
                .collect::<Vec<_>>()
                .join(" + ")
        })
        .unwrap_or_default();
    if character.is_empty() {
        return Err("no player characters".to_string());
    }
    let (killed_by, kill_kind) = killer(
        v.get("killed_by_encounter")
            .and_then(Value::as_str)
            .unwrap_or(""),
        v.get("killed_by_event")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    let floors = v
        .get("map_point_history")
        .and_then(Value::as_array)
        .map(|acts| {
            acts.iter()
                .filter_map(Value::as_array)
                .map(Vec::len)
                .sum::<usize>() as u64
        })
        .unwrap_or(0);
    Ok(SyncRun {
        id: id.to_string(),
        game: Game::Sts2,
        date: eastern_date(start_time),
        start_time,
        character,
        win: v.get("win").and_then(Value::as_bool).unwrap_or(false),
        abandoned: v
            .get("was_abandoned")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ascension: v.get("ascension").and_then(Value::as_u64).unwrap_or(0),
        acts: v
            .get("acts")
            .and_then(Value::as_array)
            .map(|a| a.len() as u64)
            .unwrap_or(0),
        floors,
        killed_by,
        kill_kind,
        run_time: v.get("run_time").and_then(Value::as_u64).unwrap_or(0),
        seed: str_field(v, "seed"),
        game_mode: str_field(v, "game_mode"),
        build_id: str_field(v, "build_id"),
        raw: raw.to_string(),
    })
}

fn extract_sts1_run(id: &str, raw: &str, v: &Value) -> Result<SyncRun, String> {
    let start_time = v
        .get("timestamp")
        .and_then(Value::as_i64)
        .ok_or("missing timestamp")?;
    let character_id = v
        .get("character_chosen")
        .and_then(Value::as_str)
        .ok_or("missing character_chosen")?;
    let character = pretty_character(character_id.strip_prefix("THE_").unwrap_or(character_id));
    let win = v.get("victory").and_then(Value::as_bool).unwrap_or(false);
    let killed_by = v
        .get("killed_by")
        .and_then(Value::as_str)
        .filter(|killer| !killer.is_empty())
        .map(str::to_string);
    let abandoned = !win && killed_by.is_none();
    let floors = v.get("floor_reached").and_then(Value::as_u64).unwrap_or(0);
    let acts = if floors == 0 {
        0
    } else {
        ((floors - 1) / 17 + 1).min(4)
    };
    let kill_kind = killed_by.as_ref().map(|_| {
        match v
            .get("path_taken")
            .and_then(Value::as_array)
            .and_then(|path| path.iter().rev().find_map(Value::as_str))
        {
            Some("BOSS") => "boss",
            Some("E") => "elite",
            _ => "monster",
        }
        .to_string()
    });
    let game_mode = if bool_field(v, "is_daily") {
        "daily"
    } else if bool_field(v, "is_trial") {
        "trial"
    } else if bool_field(v, "is_endless") {
        "endless"
    } else {
        "standard"
    };

    Ok(SyncRun {
        id: id.to_string(),
        game: Game::Sts1,
        date: eastern_date(start_time),
        start_time,
        character,
        win,
        abandoned,
        ascension: v
            .get("ascension_level")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        acts,
        floors,
        killed_by,
        kill_kind,
        run_time: v.get("playtime").and_then(Value::as_u64).unwrap_or(0),
        seed: scalar_text(v.get("seed_played")),
        game_mode: game_mode.to_string(),
        build_id: str_field(v, "build_version"),
        raw: raw.to_string(),
    })
}

fn bool_field(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn scalar_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// "CHARACTER.NECROBINDER" → "Necrobinder"; "BYGONE_EFFIGY" → "Bygone Effigy".
fn pretty_character(id: &str) -> String {
    title_words(id.rsplit('.').next().unwrap_or(id))
}

fn title_words(shouty: &str) -> String {
    shouty
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The game reports "NONE.NONE" for both killer fields on a win or abandon.
/// Encounter ids end in the encounter's class: `..._BOSS`, `..._ELITE`, or
/// nothing for a regular fight; the one observed event death is EVENT.NEOW.
fn killer(encounter: &str, event: &str) -> (Option<String>, Option<String>) {
    const NONE: &str = "NONE.NONE";
    if !encounter.is_empty() && encounter != NONE {
        let name = encounter.rsplit('.').next().unwrap_or(encounter);
        for (suffix, kind) in [("_BOSS", "boss"), ("_ELITE", "elite")] {
            if let Some(base) = name.strip_suffix(suffix) {
                return (Some(title_words(base)), Some(kind.to_string()));
            }
        }
        return (Some(title_words(name)), Some("monster".to_string()));
    }
    if !event.is_empty() && event != NONE {
        let name = event.rsplit('.').next().unwrap_or(event);
        return (Some(title_words(name)), Some("event".to_string()));
    }
    (None, None)
}

// ---------------------------------------------------------------------------
// Dates: epoch seconds → YYYY-MM-DD in US Eastern time. Hand-rolled like the
// site's RFC 2822 code — no date crate in the tree. Post-2007 US DST rules
// (in force for every run this game can produce): second Sunday of March
// 02:00 EST through first Sunday of November 02:00 EDT.

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Civil date for days since 1970-01-01 (the inverse of [`days_from_civil`]).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 {
        yoe + era * 400 + 1
    } else {
        yoe + era * 400
    };
    (year, month, day)
}

/// Epoch day of the nth Sunday of a month (1970-01-01 was a Thursday).
fn nth_sunday(year: i64, month: u32, nth: u32) -> i64 {
    let first = days_from_civil(year, month, 1);
    let weekday = (first + 4).rem_euclid(7); // 0 = Sunday
    first + (7 - weekday) % 7 + 7 * (nth as i64 - 1)
}

fn in_us_eastern_dst(epoch: i64) -> bool {
    let (year, _, _) = civil_from_days(epoch.div_euclid(86400));
    let start = nth_sunday(year, 3, 2) * 86400 + 7 * 3600; // 02:00 EST = 07:00 UTC
    let end = nth_sunday(year, 11, 1) * 86400 + 6 * 3600; // 02:00 EDT = 06:00 UTC
    epoch >= start && epoch < end
}

fn eastern_date(epoch: i64) -> String {
    let offset = if in_us_eastern_dst(epoch) { -4 } else { -5 } * 3600;
    let (year, month, day) = civil_from_days((epoch + offset).div_euclid(86400));
    format!("{year:04}-{month:02}-{day:02}")
}

// ---------------------------------------------------------------------------
// Sync.

fn resolve_token(cli: Option<String>) -> Option<String> {
    if let Some(token) = cli {
        return Some(token);
    }
    if let Ok(token) = std::env::var("SPIRE_SYNC_TOKEN")
        && !token.trim().is_empty()
    {
        return Some(token.trim().to_string());
    }
    let path = std::env::var_os("HOME")
        .map(PathBuf::from)?
        .join(".config/benjisponge/spire.token");
    fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn to_payload(run: &SyncRun) -> Value {
    json!({
        "id": run.id,
        "game": run.game.as_str(),
        "date": run.date,
        "start_time": run.start_time,
        "character": run.character,
        "win": run.win,
        "abandoned": run.abandoned,
        "ascension": run.ascension,
        "acts": run.acts,
        "floors": run.floors,
        "killed_by": run.killed_by,
        "kill_kind": run.kill_kind,
        "run_time": run.run_time,
        "seed": run.seed,
        "game_mode": run.game_mode,
        "build_id": run.build_id,
        "raw": run.raw,
    })
}

fn collect_run_files(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_run_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "run")
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.chars().all(|c| c.is_ascii_digit()))
        {
            files.push(path);
        }
    }
}

async fn fetch_existing_ids(
    client: &reqwest::Client,
    api: &str,
) -> Result<HashSet<String>, String> {
    #[derive(Deserialize)]
    struct Ids {
        ids: Vec<String>,
    }
    let url = format!("{api}/api/spire/ids");
    let ids: Ids = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GET {url}: {e}"))?
        .json()
        .await
        .map_err(|e| format!("GET {url}: bad response: {e}"))?;
    existing_id_set(ids.ids)
}

fn existing_id_set(ids: Vec<String>) -> Result<HashSet<String>, String> {
    if let Some(legacy) = ids
        .iter()
        .find(|id| !id.starts_with("sts1:") && !id.starts_with("sts2:"))
    {
        return Err(format!(
            "API returned legacy unscoped run id {legacy:?} — deploy the game-aware server \
             before running this sync"
        ));
    }
    Ok(ids.into_iter().collect())
}

async fn upload_chunk(
    client: &reqwest::Client,
    api: &str,
    token: &str,
    chunk: &[&SyncRun],
) -> Result<(u64, u64), String> {
    #[derive(Deserialize)]
    struct Receipt {
        added: u64,
        skipped: u64,
    }
    let url = format!("{api}/api/spire/runs");
    let body = json!({ "runs": chunk.iter().map(|r| to_payload(r)).collect::<Vec<_>>() });
    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("unauthorized — token rejected (see --help for token sources)".to_string());
    }
    let receipt: Receipt = response
        .error_for_status()
        .map_err(|e| format!("POST {url}: {e}"))?
        .json()
        .await
        .map_err(|e| format!("POST {url}: bad response: {e}"))?;
    Ok((receipt.added, receipt.skipped))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("spire_sync: {err}");
            return ExitCode::from(2);
        }
    };

    let discovery = if args.dirs.is_empty() {
        default_history_dirs()
    } else {
        HistoryDiscovery {
            dirs: args.dirs.clone(),
            missing_sts1_history: Vec::new(),
        }
    };
    let dirs = discovery.dirs;
    for install in &discovery.missing_sts1_history {
        eprintln!(
            "spire_sync: Slay the Spire 1 is installed at {}, but it has no runs history \
             directory; aggregate preference stats cannot recreate individual runs",
            install.display()
        );
    }
    if dirs.is_empty() {
        eprintln!(
            "spire_sync: no run history directories found — is Slay the Spire installed? \
             (override with --history-dir)"
        );
        return ExitCode::FAILURE;
    }

    let mut paths = Vec::new();
    for dir in &dirs {
        collect_run_files(dir, &mut paths);
    }

    // game:id → run; the first configured root wins duplicate mirrors.
    let mut runs: BTreeMap<String, SyncRun> = BTreeMap::new();
    let mut parse_failures: Vec<(PathBuf, String)> = Vec::new();
    for path in &paths {
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        match fs::read_to_string(path) {
            Ok(raw) => match extract_run(id, &raw) {
                Ok(run) => {
                    let key = format!("{}:{}", run.game.as_str(), run.id);
                    runs.entry(key).or_insert(run);
                }
                Err(err) => parse_failures.push((path.clone(), err)),
            },
            Err(err) => parse_failures.push((path.clone(), err.to_string())),
        }
    }
    for (path, err) in &parse_failures {
        eprintln!("spire_sync: skipping {}: {err}", path.display());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("spire-sync (+https://ben.soy/spire)")
        .build()
        .expect("reqwest client");

    let existing = match fetch_existing_ids(&client, &args.api).await {
        Ok(ids) => ids,
        Err(err) => {
            eprintln!("spire_sync: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut new_runs: Vec<&SyncRun> = runs
        .iter()
        .filter_map(|(key, run)| (!existing.contains(key)).then_some(run))
        .collect();
    new_runs.sort_by_key(|r| r.start_time); // oldest first

    let mut uploaded = 0u64;
    let mut skipped = 0u64;
    if !args.dry_run && !new_runs.is_empty() {
        let Some(token) = resolve_token(args.token.clone()) else {
            eprintln!(
                "spire_sync: {} new runs but no write token — set SPIRE_SYNC_TOKEN, pass \
                 --token, or create ~/.config/benjisponge/spire.token",
                new_runs.len()
            );
            return ExitCode::FAILURE;
        };
        let chunks: Vec<&[&SyncRun]> = new_runs.chunks(CHUNK).collect();
        for (index, chunk) in chunks.iter().enumerate() {
            match upload_chunk(&client, &args.api, &token, chunk).await {
                Ok((added, ignored)) => {
                    uploaded += added;
                    skipped += ignored;
                    if !args.json {
                        println!("  chunk {}/{}: {} added", index + 1, chunks.len(), added);
                    }
                }
                Err(err) => {
                    eprintln!("spire_sync: {err}");
                    eprintln!(
                        "spire_sync: aborted after {uploaded} uploads — rerun to resume \
                         (duplicates are ignored server-side)"
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    if args.json {
        println!(
            "{}",
            json!({
                "api": args.api,
                "dirs": dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>(),
                "missing_sts1_history": discovery.missing_sts1_history.iter()
                    .map(|path| path.display().to_string()).collect::<Vec<_>>(),
                "scanned": runs.len(),
                "scanned_sts1": runs.values().filter(|run| run.game == Game::Sts1).count(),
                "scanned_sts2": runs.values().filter(|run| run.game == Game::Sts2).count(),
                "parse_failures": parse_failures.len(),
                "already_synced": existing.len(),
                "new": new_runs.len(),
                "uploaded": uploaded,
                "skipped_as_duplicates": skipped,
                "dry_run": args.dry_run,
            })
        );
    } else {
        println!(
            "scanned {} runs ({} StS 1, {} StS 2) in {} dir(s); {} already in the database",
            runs.len(),
            runs.values().filter(|run| run.game == Game::Sts1).count(),
            runs.values().filter(|run| run.game == Game::Sts2).count(),
            dirs.len(),
            existing.len()
        );
        if args.dry_run {
            println!("dry run: {} run(s) would upload", new_runs.len());
            for run in new_runs.iter().take(10) {
                println!(
                    "  {}:{} {} {} (asc {})",
                    run.game.as_str(),
                    run.id,
                    run.date,
                    run.character,
                    run.ascension
                );
            }
            if new_runs.len() > 10 {
                println!("  … and {} more", new_runs.len() - 10);
            }
        } else if new_runs.is_empty() {
            println!("nothing to upload — in sync");
        } else {
            println!("uploaded {uploaded} run(s) ({skipped} duplicate(s) ignored)");
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEATH: &str = r#"{
        "start_time": 1772889032, "win": false, "was_abandoned": false,
        "ascension": 2, "seed": "ABCD", "game_mode": "standard",
        "build_id": "v0.98.2", "run_time": 3600,
        "killed_by_encounter": "ENCOUNTER.KNOWLEDGE_DEMON_BOSS",
        "killed_by_event": "NONE.NONE",
        "acts": ["ACT.UNDERDOCKS", "ACT.HIVE"],
        "map_point_history": [[1, 2, 3], [4, 5]],
        "players": [{"character": "CHARACTER.NECROBINDER"}]
    }"#;

    #[test]
    fn extracts_a_death() {
        let run = extract_run("1772889032", DEATH).unwrap();
        assert_eq!(run.id, "1772889032");
        assert_eq!(run.game, Game::Sts2);
        assert_eq!(run.character, "Necrobinder");
        assert!(!run.win);
        assert_eq!(run.ascension, 2);
        assert_eq!(run.acts, 2);
        assert_eq!(run.floors, 5);
        assert_eq!(run.killed_by.as_deref(), Some("Knowledge Demon"));
        assert_eq!(run.kill_kind.as_deref(), Some("boss"));
        assert_eq!(run.raw, DEATH);
    }

    #[test]
    fn extracts_an_sts1_run() {
        let raw = r#"{
            "timestamp": 1534999722,
            "local_time": "20180823004842",
            "character_chosen": "THE_SILENT",
            "victory": false,
            "ascension_level": 17,
            "floor_reached": 28,
            "killed_by": "Book of Stabbing",
            "path_taken": ["M", "BOSS", "M", "E"],
            "playtime": 1710,
            "seed_played": "-693890031029414182",
            "is_daily": false,
            "is_trial": false,
            "is_endless": false,
            "build_version": "2018-08-16"
        }"#;
        let run = extract_run("1534999722", raw).unwrap();
        assert_eq!(run.game, Game::Sts1);
        assert_eq!(run.character, "Silent");
        assert_eq!(run.date, "2018-08-23");
        assert_eq!(run.acts, 2);
        assert_eq!(run.floors, 28);
        assert_eq!(run.killed_by.as_deref(), Some("Book of Stabbing"));
        assert_eq!(run.kill_kind.as_deref(), Some("elite"));
        assert_eq!(run.seed, "-693890031029414182");
        assert!(!run.abandoned);
    }

    #[test]
    fn treats_an_sts1_loss_without_a_killer_as_abandoned() {
        let raw = r#"{
            "timestamp": 1707684719,
            "character_chosen": "IRONCLAD",
            "victory": false,
            "floor_reached": 7
        }"#;
        let run = extract_run("1707684719", raw).unwrap();
        assert!(run.abandoned);
        assert_eq!(run.killed_by, None);
    }

    #[test]
    fn rejects_legacy_unscoped_server_ids() {
        let error = existing_id_set(vec!["1772889032".to_string()]).unwrap_err();
        assert!(error.contains("deploy the game-aware server"));
        assert_eq!(
            existing_id_set(vec![
                "sts1:1534999722".to_string(),
                "sts2:1772889032".to_string(),
            ])
            .unwrap()
            .len(),
            2
        );
    }

    #[test]
    fn extracts_a_win_with_no_killer() {
        let win = DEATH
            .replace("\"win\": false", "\"win\": true")
            .replace("ENCOUNTER.KNOWLEDGE_DEMON_BOSS", "NONE.NONE");
        let run = extract_run("1772889032", &win).unwrap();
        assert!(run.win);
        assert_eq!(run.killed_by, None);
        assert_eq!(run.kill_kind, None);
    }

    #[test]
    fn killer_classification() {
        assert_eq!(
            killer("ENCOUNTER.BYGONE_EFFIGY_ELITE", "NONE.NONE"),
            (Some("Bygone Effigy".into()), Some("elite".into()))
        );
        assert_eq!(
            killer("ENCOUNTER.SPIRE_CRAWLER", "NONE.NONE"),
            (Some("Spire Crawler".into()), Some("monster".into()))
        );
        assert_eq!(
            killer("NONE.NONE", "EVENT.NEOW"),
            (Some("Neow".into()), Some("event".into()))
        );
        assert_eq!(killer("NONE.NONE", "NONE.NONE"), (None, None));
    }

    #[test]
    fn characters_prettify_and_coop_joins() {
        assert_eq!(pretty_character("CHARACTER.IRONCLAD"), "Ironclad");
        assert_eq!(
            pretty_character("CHARACTER.RANDOM_CHARACTER"),
            "Random Character"
        );
        let coop = DEATH.replace(
            r#"[{"character": "CHARACTER.NECROBINDER"}]"#,
            r#"[{"character": "CHARACTER.NECROBINDER"}, {"character": "CHARACTER.SILENT"}]"#,
        );
        let run = extract_run("1", &coop).unwrap();
        assert_eq!(run.character, "Necrobinder + Silent");
    }

    #[test]
    fn civil_date_round_trips() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        for days in [-719468, -1, 0, 1, 19000, 20635, 40000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "round trip for {days}");
        }
    }

    #[test]
    fn eastern_dates_match_known_runs() {
        // 2026-03-07T13:10:32Z, before US DST starts (EST, UTC-5) → same day.
        assert_eq!(eastern_date(1772889032), "2026-03-07");
        // 2026-07-20T22:44:13Z, during DST (EDT, UTC-4) → still the 20th.
        assert_eq!(eastern_date(1784587453), "2026-07-20");
        // 2026-01-01T03:00:00Z is still New Year's Eve in New York.
        assert_eq!(eastern_date(1767236400), "2025-12-31");
    }

    #[test]
    fn dst_boundaries_2026() {
        // DST starts Sunday 2026-03-08 at 07:00 UTC and ends 2026-11-01 06:00 UTC.
        assert_eq!(nth_sunday(2026, 3, 2), days_from_civil(2026, 3, 8));
        assert_eq!(nth_sunday(2026, 11, 1), days_from_civil(2026, 11, 1));
        let spring = days_from_civil(2026, 3, 8) * 86400 + 7 * 3600;
        assert!(!in_us_eastern_dst(spring - 1));
        assert!(in_us_eastern_dst(spring));
        let fall = days_from_civil(2026, 11, 1) * 86400 + 6 * 3600;
        assert!(in_us_eastern_dst(fall - 1));
        assert!(!in_us_eastern_dst(fall));
    }
}
