//! The Slay the Spire 2 run log, queried in-process from SurrealDB.
//! The sync CLI (`src/bin/spire_sync.rs`) is the write path via
//! `POST /api/spire/runs` and does all the prettifying, so a [`Run`] arrives
//! display-ready. Results are cached in-process for a minute; a failed query
//! serves the last good copy, or an empty log flagged `live: false` — run
//! data must never 500 a page. The import endpoint calls [`invalidate`]
//! after a commit, so a sync is visible on the very next render.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use benjisponge::data::{Data, spire_models::SpireRun};

/// One synced run — the fields the site renders. The database row carries
/// more (seed, acts, build id); the conversion drops what pages don't use.
#[derive(Clone, Debug)]
pub struct Run {
    /// The run file's stem — also its `start_time`, as a string.
    pub id: String,
    /// `YYYY-MM-DD`, US Eastern, stamped by the sync CLI.
    pub date: String,
    /// Epoch seconds; the log's sort key.
    pub start_time: i64,
    /// Prettified, e.g. "Necrobinder"; co-op runs join with " + ".
    pub character: String,
    pub win: bool,
    pub abandoned: bool,
    pub ascension: u32,
    pub floors: u32,
    /// Prettified killer, e.g. "Knowledge Demon"; `None` on wins/abandons.
    pub killed_by: Option<String>,
    /// "boss" | "elite" | "monster" | "event", when `killed_by` is set.
    pub kill_kind: Option<String>,
    /// Seconds of play.
    pub run_time: u64,
    /// "standard" for almost everything; "daily" runs get a marker.
    pub game_mode: String,
}

impl Run {
    /// The run-log table's result cell: "won", "died to X", "abandoned".
    pub fn result_label(&self) -> String {
        if self.win {
            "won".to_string()
        } else if self.abandoned {
            "abandoned".to_string()
        } else {
            match &self.killed_by {
                Some(killer) => format!("died to {killer}"),
                None => "died".to_string(),
            }
        }
    }
}

/// What a page gets: the runs (newest first) and whether they are real.
pub struct RunLog {
    pub runs: Arc<Vec<Run>>,
    /// False only when the database was unreachable AND no cached copy
    /// existed — the one case pages should admit the log is missing rather
    /// than empty.
    pub live: bool,
}

/// "7534 s" → "2h 05m"; sub-hour runs render as "48m".
pub fn fmt_duration(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

static CACHE: Mutex<Option<(Instant, Arc<Vec<Run>>)>> = Mutex::new(None);
const TTL: Duration = Duration::from_secs(60);

/// Load the run log through the one-minute cache.
pub async fn load(data: &Data) -> RunLog {
    if let Some((at, runs)) = CACHE.lock().unwrap().clone()
        && at.elapsed() < TTL
    {
        return RunLog { runs, live: true };
    }
    match query(data).await {
        Ok(runs) => {
            let runs = Arc::new(runs);
            *CACHE.lock().unwrap() = Some((Instant::now(), Arc::clone(&runs)));
            RunLog { runs, live: true }
        }
        Err(err) => {
            eprintln!("spire run query failed: {err}");
            // A stale copy beats an empty page; the TTL stays expired, so the
            // next request retries the query.
            match CACHE.lock().unwrap().clone() {
                Some((_, runs)) => RunLog { runs, live: true },
                None => RunLog {
                    runs: Arc::new(Vec::new()),
                    live: false,
                },
            }
        }
    }
}

/// Drop the cached copy — the import endpoint calls this after a commit so
/// the next render reflects the new runs immediately.
pub fn invalidate() {
    *CACHE.lock().unwrap() = None;
}

async fn query(data: &Data) -> Result<Vec<Run>, Box<dyn std::error::Error + Send + Sync>> {
    let db = data.db().await?;
    let rows = super::db::list_runs(&db).await?;
    Ok(from_rows(rows))
}

/// Convert stored rows: newest first, and any run whose date isn't
/// `YYYY-MM-DD` is dropped (the feed's RFC 2822 conversion indexes into that
/// shape, and one bad row must not take out `/feed.xml`).
fn from_rows(rows: Vec<SpireRun>) -> Vec<Run> {
    let mut runs: Vec<Run> = rows
        .into_iter()
        .filter(|row| {
            let ok = iso_date(&row.date);
            if !ok {
                eprintln!(
                    "spire run {} has malformed date {:?}; dropped",
                    row.id, row.date
                );
            }
            ok
        })
        .map(|row| Run {
            id: row.id,
            date: row.date,
            start_time: row.start_time,
            character: row.character,
            win: row.win,
            abandoned: row.abandoned,
            ascension: row.ascension.try_into().unwrap_or(u32::MAX),
            floors: row.floors.try_into().unwrap_or(u32::MAX),
            killed_by: row.killed_by,
            kill_kind: row.kill_kind,
            run_time: row.run_time.try_into().unwrap_or(0),
            game_mode: row.game_mode,
        })
        .collect();
    runs.sort_unstable_by_key(|r| std::cmp::Reverse(r.start_time));
    runs
}

fn iso_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    date.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, date: &str, start_time: i64, win: bool) -> SpireRun {
        SpireRun {
            id: id.to_string(),
            date: date.to_string(),
            start_time,
            character: if win { "Necrobinder" } else { "Silent" }.to_string(),
            win,
            abandoned: false,
            ascension: 3,
            acts: 3,
            floors: 34,
            killed_by: (!win).then(|| "Knowledge Demon".to_string()),
            kill_kind: (!win).then(|| "boss".to_string()),
            run_time: 7534,
            seed: "6CCBAWXEKT".to_string(),
            game_mode: "standard".to_string(),
            build_id: "v0.107.1".to_string(),
            added_at: 0,
        }
    }

    #[test]
    fn converts_and_sorts_newest_first() {
        let runs = from_rows(vec![
            row("1772889032", "2026-03-07", 1772889032, false),
            row("1784587453", "2026-07-20", 1784587453, true),
        ]);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, "1784587453");
        assert!(runs[0].win);
        assert_eq!(runs[1].killed_by.as_deref(), Some("Knowledge Demon"));
    }

    #[test]
    fn malformed_dates_are_dropped_not_fatal() {
        let runs = from_rows(vec![
            row("1772889032", "March 7th", 1772889032, false),
            row("1784587453", "2026-07-20", 1784587453, true),
        ]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, "1784587453");
    }

    #[test]
    fn result_labels() {
        let runs = from_rows(vec![
            row("1772889032", "2026-03-07", 1772889032, false),
            row("1784587453", "2026-07-20", 1784587453, true),
        ]);
        assert_eq!(runs[0].result_label(), "won");
        assert_eq!(runs[1].result_label(), "died to Knowledge Demon");
        let mut abandoned = runs[1].clone();
        abandoned.abandoned = true;
        abandoned.killed_by = None;
        assert_eq!(abandoned.result_label(), "abandoned");
    }

    #[test]
    fn durations_read_like_a_clock() {
        assert_eq!(fmt_duration(7534), "2h 05m");
        assert_eq!(fmt_duration(2891), "48m");
        assert_eq!(fmt_duration(59), "0m");
        assert_eq!(fmt_duration(3600), "1h 00m");
    }
}
