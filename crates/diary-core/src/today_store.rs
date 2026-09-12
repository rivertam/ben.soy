//! Live daily autosave. Revision CAS protects concurrent tabs/devices; a
//! retried identical write is acknowledged without spending the budget twice.
use crate::{
    Db,
    today::{self, Snapshot},
};
use serde::{Deserialize, Serialize};

pub const DAY_PROJECTION: &str = "day, body, used_ms, closed, closed_at, updated_at, revision";
pub const API_PATH: &str = "/api/diary/today";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Remember {
        usages: Vec<crate::emoji_usage::Usage>,
    },
    Save {
        day: String,
        body: String,
        used_ms: u32,
        close: bool,
        expected_revision: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub schema_epoch: u16,
    pub action: Action,
}

impl Command {
    pub fn new(action: Action) -> Self {
        Self {
            schema_epoch: crate::contract::CURRENT_SCHEMA_EPOCH,
            action,
        }
    }
}

pub async fn snapshot(db: &Db) -> Result<Snapshot, String> {
    let mut result = db
        .query(format!(
            "SELECT {DAY_PROJECTION} FROM diary_days ORDER BY day ASC"
        ))
        .await
        .map_err(err)?
        .check()
        .map_err(err)?;
    Ok(Snapshot {
        schema_epoch: crate::contract::CURRENT_SCHEMA_EPOCH,
        days: result.take(0).map_err(err)?,
        emoji_usage: crate::emoji_usage::recent(db).await?,
    })
}

/// Live autosaves/heartbeats carry only the selected reflection, so frequent
/// requests never download the entire private writing archive. No day means
/// just the small Now emoji history.
pub async fn live_snapshot(db: &Db, day: Option<&str>) -> Result<Snapshot, String> {
    let days = if let Some(day) = day {
        let mut result = db
            .query(format!(
                "SELECT {DAY_PROJECTION} FROM diary_days WHERE day = $day"
            ))
            .bind(("day", day.to_string()))
            .await
            .map_err(err)?
            .check()
            .map_err(err)?;
        result.take(0).map_err(err)?
    } else {
        Vec::new()
    };
    Ok(Snapshot {
        schema_epoch: crate::contract::CURRENT_SCHEMA_EPOCH,
        days,
        emoji_usage: crate::emoji_usage::recent(db).await?,
    })
}

pub async fn apply(db: &Db, command: &Command, now: i64) -> Result<Snapshot, String> {
    if command.schema_epoch != crate::contract::CURRENT_SCHEMA_EPOCH {
        return Err("schema epoch mismatch".into());
    }
    match &command.action {
        Action::Remember { usages } => {
            if usages.len() > 10
                || usages.iter().any(|usage| {
                    !crate::entry::valid_emoji(&usage.emoji)
                        || usage.used_at_ms < 0
                        || usage.used_at_ms > now.saturating_add(300).saturating_mul(1000)
                })
            {
                return Err("invalid emoji usage".into());
            }
            for usage in usages {
                crate::emoji_usage::remember(db, usage).await?;
            }
        }
        Action::Save {
            day,
            body,
            used_ms,
            close,
            expected_revision,
        } => {
            let current = today::day_at(now).ok_or("invalid server date")?;
            if !today::valid_day(day)
                || day > &current
                || *used_ms > today::BUDGET_MS
                || body.chars().count() > crate::entry::MAX_ENTRY_CHARS
                || *expected_revision >= i64::MAX as u64
            {
                return Err("invalid reflection".into());
            }
            let body = body.replace("\r\n", "\n").replace('\r', "\n");
            let closed = *close || *used_ms == today::BUDGET_MS;
            // A last autosave can arrive after 04:00, without permitting a
            // new entry for an old day or reopening its editor.
            let updated_at = now.min(today::day_end(day).ok_or("invalid date")? - 1);
            for attempt in 0..5 {
                let result = db.query(
                    "BEGIN TRANSACTION;
                     LET $old = (SELECT * FROM ONLY type::record('diary_days', $day));
                     LET $replay = $old != NONE AND $old.revision = $revision + 1
                         AND $old.body = $body AND $old.used_ms = $used_ms AND $old.closed = $closed;
                     IF !$replay {
                         IF $old != NONE AND $old.closed { THROW 'reflection-closed'; };
                         IF ($old = NONE AND ($revision != 0 OR $day != $current))
                             OR ($old != NONE AND ($old.revision != $revision OR $used_ms < $old.used_ms)) {
                             THROW 'stale-reflection';
                         };
                         UPSERT type::record('diary_days', $day) CONTENT {
                             day: $day, body: $body, used_ms: $used_ms, closed: $closed,
                             closed_at: IF $closed { $now } ELSE { NONE }, updated_at: $now,
                             revision: $revision + 1
                         };
                     };
                     COMMIT TRANSACTION;"
                )
                .bind(("day", day.clone())).bind(("current", current.clone()))
                .bind(("body", body.clone())).bind(("used_ms", *used_ms))
                .bind(("closed", closed)).bind(("revision", *expected_revision))
                .bind(("now", updated_at))
                .await.map_err(err).and_then(|response| response.check().map_err(err));
                match result {
                    Err(error) if error.contains("Transaction conflict") && attempt < 4 => continue,
                    result => {
                        result?;
                        break;
                    }
                }
            }
        }
    }
    let day = match &command.action {
        Action::Save { day, .. } => Some(day.as_str()),
        Action::Remember { .. } => None,
    };
    live_snapshot(db, day).await
}

fn err(error: surrealdb::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn store() -> Db {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("diary").use_db("diary").await.unwrap();
        db.query(include_str!(
            "../../../src/data/diary_migrations/0004_today_live.surql"
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
        db
    }
    fn now() -> i64 {
        "2026-09-10T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_second()
    }
    fn save(body: &str, used_ms: u32, revision: u64, close: bool) -> Command {
        Command::new(Action::Save {
            day: "2026-09-10".into(),
            body: body.into(),
            used_ms,
            expected_revision: revision,
            close,
        })
    }
    #[test]
    fn http_commands_round_trip_and_reject_unknown_fields() {
        let original = save("draft", 0, 0, false);
        let parsed: Command =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(matches!(parsed.action, Action::Save { .. }));
        let mut unknown = serde_json::to_value(&original).unwrap();
        unknown["device"] = serde_json::json!("obsolete-grant");
        assert!(serde_json::from_value::<Command>(unknown).is_err());
    }
    #[tokio::test]
    async fn autosave_replays_share_budget_and_finishing_is_permanent() {
        let db = store().await;
        let first = save("  private reflection\n", 100_000, 0, false);
        let saved = apply(&db, &first, now()).await.unwrap();
        let replay = apply(&db, &first, now() + 1).await.unwrap();
        assert_eq!(saved.days, replay.days);
        assert!(
            apply(&db, &save("decrease", 99_000, 1, false), now())
                .await
                .is_err()
        );
        let next = apply(&db, &save("second device", 125_000, 1, false), now())
            .await
            .unwrap();
        assert_eq!(next.days[0].used_ms, 125_000);
        assert!(apply(&db, &first, now()).await.is_err());
        apply(&db, &save("finished", 125_000, 2, true), now())
            .await
            .unwrap();
        assert!(
            apply(&db, &save("reopen", 125_000, 3, false), now())
                .await
                .is_err()
        );
        assert_eq!(snapshot(&db).await.unwrap().days[0].body, "finished");
    }
    #[tokio::test]
    async fn concurrent_devices_cannot_overwrite_each_other() {
        let db = store().await;
        apply(&db, &save("original", 1000, 0, false), now())
            .await
            .unwrap();
        let a = save("device A", 2000, 1, false);
        let b = save("device B", 3000, 1, false);
        let (a, b) = tokio::join!(apply(&db, &a, now()), apply(&db, &b, now()));
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(snapshot(&db).await.unwrap().days[0].revision, 2);
    }
    #[tokio::test]
    async fn live_responses_include_only_the_requested_day() {
        let db = store().await;
        apply(&db, &save("yesterday", 1000, 0, true), now())
            .await
            .unwrap();
        let mut next = save("today", 2000, 0, false);
        if let Action::Save { day, .. } = &mut next.action {
            *day = "2026-09-11".into();
        }
        let saved = apply(&db, &next, now() + 86400).await.unwrap();
        assert_eq!(saved.days.len(), 1);
        assert_eq!(saved.days[0].body, "today");
        let yesterday = live_snapshot(&db, Some("2026-09-10")).await.unwrap();
        assert_eq!(yesterday.days.len(), 1);
        assert_eq!(yesterday.days[0].body, "yesterday");
        assert!(live_snapshot(&db, None).await.unwrap().days.is_empty());
        assert_eq!(snapshot(&db).await.unwrap().days.len(), 2);
    }

    #[tokio::test]
    async fn budget_exhaustion_closes_and_old_days_cannot_be_created() {
        let db = store().await;
        let saved = apply(&db, &save("last words", today::BUDGET_MS, 0, false), now())
            .await
            .unwrap();
        assert!(saved.days[0].closed);
        assert!(
            apply(&db, &save("too much", today::BUDGET_MS + 1, 1, true), now())
                .await
                .is_err()
        );
        let tomorrow = now() + 86400;
        let other = store().await;
        assert!(
            apply(&other, &save("new old day", 0, 0, false), tomorrow)
                .await
                .is_err()
        );
    }
}
