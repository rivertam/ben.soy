//! Spire run queries and the import write path.
//!
//! Port of the old Worker's `spire.ts` (since deleted). Validation messages
//! are contract — the
//! golden fixtures in `tests/fixtures/api` capture them verbatim (note the
//! en dash in the runs-array message). Unknown keys on a run object are
//! ignored, exactly like the TS destructuring did.

use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;
use surrealdb::types::SurrealValue;

use benjisponge::data::{
    Db,
    spire_models::{SpireRun, SpireRunRaw},
};

pub const MAX_RUNS_PER_CHUNK: usize = 50;

/// All runs, newest first — mirrors `ORDER BY start_time DESC`.
pub async fn list_runs(db: &Db) -> surrealdb::Result<Vec<SpireRun>> {
    let mut response = db
        .query(
            "SELECT *, game ?? 'sts2' AS game,
                       run_id ?? record::id(id) AS id
             FROM spire_runs
             ORDER BY start_time DESC",
        )
        .await?
        .check()?;
    response.take(0)
}

/// Stored run ids. Deliberately unordered, like `SELECT id FROM spire_runs`;
/// the sync CLI treats the result as a set.
pub async fn list_ids(db: &Db) -> surrealdb::Result<Vec<String>> {
    #[derive(Deserialize, SurrealValue)]
    struct StoredIdentity {
        storage_id: String,
        game: Option<String>,
        run_id: Option<String>,
    }

    let mut response = db
        .query(
            "SELECT record::id(id) AS storage_id, game, run_id
             FROM spire_runs",
        )
        .await?
        .check()?;
    let identities: Vec<StoredIdentity> = response.take(0)?;
    Ok(identities
        .into_iter()
        .map(|row| {
            let game = row.game.unwrap_or_else(|| "sts2".to_string());
            let run_id = row.run_id.unwrap_or(row.storage_id);
            format!("{game}:{run_id}")
        })
        .collect())
}

/// The data version; 0 when the row does not exist yet.
pub async fn current_version(db: &Db) -> surrealdb::Result<i64> {
    let mut response = db
        .query("SELECT VALUE v FROM spire_meta:version")
        .await?
        .check()?;
    let versions: Vec<i64> = response.take(0)?;
    Ok(versions.into_iter().next().unwrap_or(0))
}

/// A validated incoming run: a stored row plus the original `.run` payload.
#[derive(Clone, Debug, PartialEq)]
pub struct IncomingRun {
    pub id: String,
    pub game: String,
    pub date: String,
    pub start_time: i64,
    pub character: String,
    pub win: bool,
    pub abandoned: bool,
    pub ascension: i64,
    pub acts: i64,
    pub floors: i64,
    pub killed_by: Option<String>,
    pub kill_kind: Option<String>,
    pub run_time: i64,
    pub seed: String,
    pub game_mode: String,
    pub build_id: String,
    pub raw: String,
}

/// Parse and validate a POST body. `Err` carries the exact 400 message.
pub fn parse_payload(body: &[u8]) -> Result<Vec<IncomingRun>, String> {
    let value: Value = serde_json::from_slice(body).map_err(|_| "body must be JSON".to_string())?;
    let Some(object) = value.as_object() else {
        return Err("body must be an object".to_string());
    };
    let runs = object.get("runs").and_then(Value::as_array);
    let runs = match runs {
        Some(runs) if !runs.is_empty() && runs.len() <= MAX_RUNS_PER_CHUNK => runs,
        _ => return Err("runs must be an array of 1\u{2013}50 entries".to_string()),
    };
    runs.iter().map(parse_run).collect()
}

fn parse_run(value: &Value) -> Result<IncomingRun, String> {
    let Some(run) = value.as_object() else {
        return Err("run must be an object".to_string());
    };

    let id = match run.get("id").and_then(Value::as_str) {
        Some(id) if (1..=12).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()) => id,
        _ => return Err("bad id".to_string()),
    };
    let game = match run.get("game") {
        None => "sts2",
        Some(Value::String(game)) if matches!(game.as_str(), "sts1" | "sts2") => game,
        _ => return Err(format!("bad game on run {id}")),
    };

    let date = run.get("date").and_then(Value::as_str);
    let date = match date {
        Some(date) if is_iso_date_shape(date) => date,
        _ => return Err(format!("bad date on run {id}")),
    };

    let start_time = valid_integer(run.get("start_time"))
        .ok_or_else(|| format!("bad start_time on run {id}"))?;
    let ascension =
        valid_integer(run.get("ascension")).ok_or_else(|| format!("bad ascension on run {id}"))?;
    let acts = valid_integer(run.get("acts")).ok_or_else(|| format!("bad acts on run {id}"))?;
    let floors =
        valid_integer(run.get("floors")).ok_or_else(|| format!("bad floors on run {id}"))?;
    let run_time =
        valid_integer(run.get("run_time")).ok_or_else(|| format!("bad run_time on run {id}"))?;

    let (win, abandoned) = match (
        run.get("win").and_then(Value::as_bool),
        run.get("abandoned").and_then(Value::as_bool),
    ) {
        (Some(win), Some(abandoned)) => (win, abandoned),
        _ => return Err(format!("bad result flags on run {id}")),
    };

    let character = required_text(run.get("character"), 1, 120)
        .ok_or_else(|| format!("bad character on run {id}"))?;
    let seed =
        required_text(run.get("seed"), 0, 120).ok_or_else(|| format!("bad seed on run {id}"))?;
    let game_mode = required_text(run.get("game_mode"), 0, 120)
        .ok_or_else(|| format!("bad game_mode on run {id}"))?;
    let build_id = required_text(run.get("build_id"), 0, 120)
        .ok_or_else(|| format!("bad build_id on run {id}"))?;

    let killed_by = nullable_text(run.get("killed_by"), 120)
        .ok_or_else(|| format!("bad killed_by on run {id}"))?;
    let kill_kind = nullable_text(run.get("kill_kind"), 120)
        .ok_or_else(|| format!("bad kill_kind on run {id}"))?;

    // Largest observed .run file is ~97 KB; 500 KB leaves generous headroom.
    let raw =
        required_text(run.get("raw"), 1, 500_000).ok_or_else(|| format!("bad raw on run {id}"))?;

    Ok(IncomingRun {
        id: id.to_string(),
        game: game.to_string(),
        date: date.to_string(),
        start_time,
        character,
        win,
        abandoned,
        ascension,
        acts,
        floors,
        killed_by,
        kill_kind,
        run_time,
        seed,
        game_mode,
        build_id,
        raw,
    })
}

/// `typeof v === "number" && Number.isInteger(v) && 0 <= v <= 1e12`,
/// including JSON floats with integral values (`5.0` passes in JS).
fn valid_integer(value: Option<&Value>) -> Option<i64> {
    const MAX: i64 = 1_000_000_000_000;
    let number = value?.as_number()?;
    if let Some(integer) = number.as_i64() {
        return (0..=MAX).contains(&integer).then_some(integer);
    }
    let float = number.as_f64()?;
    if float.fract() == 0.0 && (0.0..=MAX as f64).contains(&float) {
        return Some(float as i64);
    }
    None
}

/// String with a JS-style length bound (UTF-16 code units).
fn required_text(value: Option<&Value>, min: usize, max: usize) -> Option<String> {
    let text = value?.as_str()?;
    let units = text.encode_utf16().count();
    (units >= min && units <= max).then(|| text.to_string())
}

/// Explicit `null` or a bounded string; an absent key is invalid (it was
/// `undefined` in the Worker, which failed the type check).
#[allow(clippy::option_option)]
fn nullable_text(value: Option<&Value>, max: usize) -> Option<Option<String>> {
    match value {
        Some(Value::Null) => Some(None),
        Some(Value::String(text)) if text.encode_utf16().count() <= max => Some(Some(text.clone())),
        _ => None,
    }
}

fn is_iso_date_shape(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportOutcome {
    pub received: usize,
    pub added: usize,
    pub skipped: usize,
    pub version: i64,
}

/// Idempotent insert: already-stored ids are skipped; the version bumps
/// once when anything landed. Runs and their raw payloads commit together.
///
/// Concurrent imports of the same new id abort one transaction on the
/// primary-key conflict instead of silently ignoring it (D1 used INSERT OR
/// IGNORE); the sync CLI is the only writer and simply reruns.
pub async fn insert_runs(
    db: &Db,
    incoming: &[IncomingRun],
    added_at_epoch: i64,
) -> surrealdb::Result<ImportOutcome> {
    let existing: HashSet<String> = list_ids(db).await?.into_iter().collect();

    let candidates: Vec<&IncomingRun> = incoming
        .iter()
        .filter(|run| !existing.contains(&format!("{}:{}", run.game, run.id)))
        .collect();
    let added = candidates.len();

    if added > 0 {
        let runs: Vec<SpireRun> = candidates
            .iter()
            .map(|run| SpireRun {
                id: run.id.clone(),
                game: run.game.clone(),
                date: run.date.clone(),
                start_time: run.start_time,
                character: run.character.clone(),
                win: run.win,
                abandoned: run.abandoned,
                ascension: run.ascension,
                acts: run.acts,
                floors: run.floors,
                killed_by: run.killed_by.clone(),
                kill_kind: run.kill_kind.clone(),
                run_time: run.run_time,
                seed: run.seed.clone(),
                game_mode: run.game_mode.clone(),
                build_id: run.build_id.clone(),
                added_at: added_at_epoch,
            })
            .collect();
        let raws: Vec<SpireRunRaw> = candidates
            .iter()
            .map(|run| SpireRunRaw {
                id: run.id.clone(),
                game: run.game.clone(),
                raw: run.raw.clone(),
            })
            .collect();

        db.query(
            "BEGIN TRANSACTION;
             FOR $run IN $runs {
                 CREATE ONLY type::record('spire_runs', $run.game + ':' + $run.id)
                     SET game = $run.game,
                         run_id = $run.id,
                         date = $run.date,
                         start_time = $run.start_time,
                         character = $run.character,
                         win = $run.win,
                         abandoned = $run.abandoned,
                         ascension = $run.ascension,
                         acts = $run.acts,
                         floors = $run.floors,
                         killed_by = $run.killed_by,
                         kill_kind = $run.kill_kind,
                         run_time = $run.run_time,
                         seed = $run.seed,
                         game_mode = $run.game_mode,
                         build_id = $run.build_id,
                         added_at = $run.added_at;
             };
             FOR $raw IN $raws {
                 CREATE ONLY type::record('spire_run_raws', $raw.game + ':' + $raw.id)
                     SET game = $raw.game,
                         run_id = $raw.id,
                         raw = $raw.raw;
             };
             UPSERT spire_meta:version
                 SET k = 'version',
                     v = (v ?? 0) + 1;
             COMMIT TRANSACTION;",
        )
        .bind(("runs", runs))
        .bind(("raws", raws))
        .await?
        .check()?;
    }

    let version = current_version(db).await?;
    Ok(ImportOutcome {
        received: incoming.len(),
        added,
        skipped: incoming.len() - added,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_run() -> Value {
        serde_json::json!({
            "id": "1753210000",
            "date": "2026-07-22",
            "start_time": 1753210000,
            "character": "IRONCLAD",
            "win": true,
            "abandoned": false,
            "ascension": 5,
            "acts": 4,
            "floors": 57,
            "killed_by": null,
            "kill_kind": null,
            "run_time": 2400,
            "seed": "ABC123",
            "game_mode": "STANDARD",
            "build_id": "2026-07-01",
            "raw": "{}"
        })
    }

    fn payload(runs: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "runs": runs })).unwrap()
    }

    #[test]
    fn accepts_a_valid_run() {
        let parsed = parse_payload(&payload(vec![valid_run()])).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "1753210000");
        assert_eq!(parsed[0].game, "sts2");
        assert_eq!(parsed[0].killed_by, None);
    }

    #[test]
    fn accepts_both_games_and_rejects_unknown_discriminators() {
        let mut sts1 = valid_run();
        sts1["game"] = Value::from("sts1");
        assert_eq!(parse_payload(&payload(vec![sts1])).unwrap()[0].game, "sts1");

        let mut unknown = valid_run();
        unknown["game"] = Value::from("sts3");
        assert_eq!(
            parse_payload(&payload(vec![unknown])).unwrap_err(),
            "bad game on run 1753210000"
        );
    }

    #[test]
    fn error_messages_match_the_worker_verbatim() {
        assert_eq!(parse_payload(b"nope").unwrap_err(), "body must be JSON");
        assert_eq!(parse_payload(b"[]").unwrap_err(), "body must be an object");
        assert_eq!(
            parse_payload(b"{}").unwrap_err(),
            "runs must be an array of 1\u{2013}50 entries",
        );
        assert_eq!(
            parse_payload(&payload(vec![Value::from(3)])).unwrap_err(),
            "run must be an object",
        );

        let mut bad_date = valid_run();
        bad_date["date"] = Value::from("07/22/2026");
        assert_eq!(
            parse_payload(&payload(vec![bad_date])).unwrap_err(),
            "bad date on run 1753210000",
        );

        let mut missing_kill = valid_run();
        missing_kill.as_object_mut().unwrap().remove("killed_by");
        assert_eq!(
            parse_payload(&payload(vec![missing_kill])).unwrap_err(),
            "bad killed_by on run 1753210000",
        );
    }

    #[test]
    fn fifty_one_runs_is_too_many() {
        let runs = std::iter::repeat_with(valid_run).take(51).collect();
        assert_eq!(
            parse_payload(&payload(runs)).unwrap_err(),
            "runs must be an array of 1\u{2013}50 entries",
        );
    }

    #[test]
    fn id_must_be_one_to_twelve_digits() {
        for (id, ok) in [
            ("1", true),
            ("123456789012", true),
            ("", false),
            ("1234567890123", false),
            ("12a", false),
            ("-1", false),
        ] {
            let mut run = valid_run();
            run["id"] = Value::from(id);
            assert_eq!(parse_payload(&payload(vec![run])).is_ok(), ok, "id {id:?}");
        }
    }

    #[test]
    fn integral_floats_pass_like_javascript() {
        let mut run = valid_run();
        run["floors"] = serde_json::json!(57.0);
        assert!(parse_payload(&payload(vec![run])).is_ok());

        let mut fractional = valid_run();
        fractional["floors"] = serde_json::json!(57.5);
        assert_eq!(
            parse_payload(&payload(vec![fractional])).unwrap_err(),
            "bad floors on run 1753210000",
        );
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let mut run = valid_run();
        run["shiny_new_field"] = Value::from("ignored");
        assert!(parse_payload(&payload(vec![run])).is_ok());
    }

    #[test]
    fn raw_is_required_and_bounded() {
        let mut empty = valid_run();
        empty["raw"] = Value::from("");
        assert_eq!(
            parse_payload(&payload(vec![empty])).unwrap_err(),
            "bad raw on run 1753210000",
        );
    }
}
