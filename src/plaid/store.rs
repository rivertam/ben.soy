//! One current plaid, with optimistic saves and a small public-read cache.

use serde::Deserialize;
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

use super::{Pattern, Spec};
use crate::data::{Data, Db};

#[derive(Clone, Debug, Default)]
pub struct Saved {
    pub pattern: Pattern,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Clone)]
pub struct Loaded {
    pub saved: Arc<Saved>,
    pub available: bool,
}

#[derive(Clone)]
pub struct PlaidStore {
    data: Data,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    saved: Arc<Saved>,
    checked: Option<Instant>,
    available: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveError {
    Conflict,
    Unavailable(String),
}

impl PlaidStore {
    pub fn new(data: Data) -> Self {
        Self {
            data,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    pub async fn current(&self, force: bool) -> Loaded {
        let mut state = self.state.lock().await;
        let ttl = Duration::from_secs(if state.available { 2 } else { 5 });
        if force || state.checked.is_none_or(|at| at.elapsed() >= ttl) {
            let result = match self.data.db().await {
                Ok(db) => read_db(&db).await,
                Err(error) => Err(error.to_string()),
            };
            state.available = match result {
                Ok(saved) => {
                    state.saved = Arc::new(saved);
                    true
                }
                Err(error) => {
                    tracing::warn!(%error, "plaid store unavailable; retaining previous cloth");
                    false
                }
            };
            state.checked = Some(Instant::now());
        }
        Loaded {
            saved: state.saved.clone(),
            available: state.available,
        }
    }

    pub async fn save(
        &self,
        pattern: Pattern,
        expected_revision: i64,
    ) -> Result<Arc<Saved>, SaveError> {
        let mut state = self.state.lock().await;
        let db = self
            .data
            .db()
            .await
            .map_err(|e| SaveError::Unavailable(e.to_string()))?;
        let saved = Arc::new(write_db(&db, pattern, expected_revision).await?);
        state.saved = saved.clone();
        state.available = true;
        state.checked = Some(Instant::now());
        Ok(saved)
    }
}

#[derive(Deserialize)]
struct Row {
    spec: Spec,
    revision: i64,
    fingerprint: String,
    updated_at: i64,
}

async fn read_db(db: &Db) -> Result<Saved, String> {
    let mut response = db
        .query("SELECT spec, revision, fingerprint, updated_at FROM plaid_settings:current")
        .await
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| e.to_string())?;
    let rows: Vec<serde_json::Value> = response.take(0).map_err(|e| e.to_string())?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(Saved::default());
    };
    let row: Row = serde_json::from_value(row).map_err(|e| e.to_string())?;
    let pattern = Pattern::new(row.spec)?;
    if pattern.fingerprint() != row.fingerprint || row.revision < 1 {
        return Err("stored plaid fingerprint or revision is invalid".into());
    }
    Ok(Saved {
        pattern,
        revision: row.revision,
        updated_at: row.updated_at,
    })
}

async fn write_db(db: &Db, pattern: Pattern, expected: i64) -> Result<Saved, SaveError> {
    if !(0..i64::MAX).contains(&expected) {
        return Err(SaveError::Conflict);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    for attempt in 0..3 {
        let result = db
            .query(
                "BEGIN TRANSACTION;
             LET $previous = (SELECT VALUE revision FROM plaid_settings:current)[0] ?? 0;
             IF $previous != $expected { THROW 'plaid revision conflict'; };
             UPSERT plaid_settings:current SET spec = $spec, revision = $expected + 1,
                 fingerprint = $fingerprint, updated_at = $now RETURN NONE;
             COMMIT TRANSACTION;",
            )
            .bind((
                "spec",
                serde_json::to_value(pattern.spec()).expect("validated plaid serializes"),
            ))
            .bind(("expected", expected))
            .bind(("fingerprint", pattern.fingerprint()))
            .bind(("now", now))
            .await
            .map_err(|error| error.to_string())
            .and_then(|mut response| {
                // A THROW also marks earlier transaction statements cancelled.
                // Inspect all errors so that cancellation cannot hide the
                // revision conflict (or the retryable storage conflict).
                let errors = response.take_errors();
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors
                        .values()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "))
                }
            });
        match result {
            Ok(_) => {
                return Ok(Saved {
                    pattern,
                    revision: expected + 1,
                    updated_at: now,
                });
            }
            Err(error) => {
                let message = error.to_string();
                if message.contains("plaid revision conflict") {
                    return Err(SaveError::Conflict);
                }
                if attempt < 2 && message.contains("Transaction conflict:") {
                    continue;
                }
                return Err(SaveError::Unavailable(message));
            }
        }
    }
    unreachable!("each final attempt returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn database() -> Db {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("plaid").use_db("test").await.unwrap();
        db.query(include_str!("../schema.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        db
    }

    #[tokio::test]
    async fn saves_round_trip_and_stale_writers_cannot_replace_current() {
        let db = database().await;
        assert_eq!(read_db(&db).await.unwrap().revision, 0);
        let pattern = Pattern::default();
        let saved = write_db(&db, pattern.clone(), 0).await.unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(read_db(&db).await.unwrap().pattern.text(), pattern.text());
        assert!(matches!(
            write_db(&db, pattern.clone(), 0).await,
            Err(SaveError::Conflict)
        ));
        let (a, b) = tokio::join!(write_db(&db, pattern.clone(), 1), write_db(&db, pattern, 1));
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(read_db(&db).await.unwrap().revision, 2);
        // Re-applying additive schema must never replace the saved definition.
        db.query(include_str!("../schema.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(read_db(&db).await.unwrap().revision, 2);
    }

    #[tokio::test]
    async fn unavailable_store_retains_last_good_cloth_and_reports_failure() {
        let store = PlaidStore::new(Data::new(Err("test database is unavailable")));
        let initial = store.current(false).await;
        assert!(!initial.available);
        assert_eq!(initial.saved.revision, 0);
        store.state.lock().await.saved = Arc::new(Saved {
            revision: 7,
            ..Saved::default()
        });
        let stale = store.current(true).await;
        assert!(!stale.available);
        assert_eq!(stale.saved.revision, 7);
        assert!(store.save(Pattern::default(), 7).await.is_err());
        assert_eq!(store.current(false).await.saved.revision, 7);
    }
}
