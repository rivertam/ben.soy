//! The snapshot's lifecycle: when to trust it, when to rebuild it.
//!
//! One store lives in the app context. Readers call [`FitnessStore::snapshot`];
//! the import handler calls [`FitnessStore::rebuild`] after its commit so
//! the very next render sees the new data. The version re-check is a
//! debounced backstop (the container is a singleton, so in-process rebuild
//! covers the normal path); the mutex intentionally coalesces concurrent
//! checks — a page's parallel facets+sets loads share one.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use benjisponge::data::Data;

use super::db;
use super::snapshot::{self, Snapshot};

/// How long a version check stays fresh.
const CHECK_DEBOUNCE: Duration = Duration::from_secs(2);
/// How long to serve stale (or fail fast) after a failed load.
const FAILURE_COOLDOWN: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct FitnessStore {
    data: Data,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    snapshot: Option<Arc<Snapshot>>,
    checked_at: Option<Instant>,
    failed_at: Option<Instant>,
}

#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fitness snapshot unavailable: {}", self.0)
    }
}

impl std::error::Error for StoreError {}

impl FitnessStore {
    pub fn new(data: Data) -> Self {
        FitnessStore {
            data,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    pub async fn snapshot(&self) -> Result<Arc<Snapshot>, StoreError> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let cooling = state
            .failed_at
            .is_some_and(|at| now.duration_since(at) < FAILURE_COOLDOWN);

        let Some(current) = state.snapshot.clone() else {
            if cooling {
                return Err(StoreError("cooling down after a failed load".to_string()));
            }
            return match self.load().await {
                Ok(fresh) => {
                    state.snapshot = Some(fresh.clone());
                    state.checked_at = Some(now);
                    state.failed_at = None;
                    Ok(fresh)
                }
                Err(error) => {
                    state.failed_at = Some(now);
                    Err(error)
                }
            };
        };

        let debounced = state
            .checked_at
            .is_some_and(|at| now.duration_since(at) < CHECK_DEBOUNCE);
        if debounced || cooling {
            return Ok(current);
        }

        let version = match self.version().await {
            Ok(version) => version,
            Err(error) => {
                // Stale-on-error: the archive changes a few times a week,
                // a hiccup should not blank the pages.
                log_stale(&error);
                state.failed_at = Some(now);
                return Ok(current);
            }
        };
        if version == current.version {
            state.checked_at = Some(now);
            return Ok(current);
        }
        match self.load().await {
            Ok(fresh) => {
                state.snapshot = Some(fresh.clone());
                state.checked_at = Some(now);
                state.failed_at = None;
                Ok(fresh)
            }
            Err(error) => {
                log_stale(&error);
                state.failed_at = Some(now);
                Ok(current)
            }
        }
    }

    /// Unconditional reload — the import handler's post-commit call.
    pub async fn rebuild(&self) -> Result<(), StoreError> {
        let mut state = self.state.lock().await;
        match self.load().await {
            Ok(fresh) => {
                state.snapshot = Some(fresh);
                state.checked_at = Some(Instant::now());
                state.failed_at = None;
                Ok(())
            }
            Err(error) => {
                // Leave checked_at stale so the next read retries promptly.
                state.checked_at = None;
                state.failed_at = None;
                Err(error)
            }
        }
    }

    async fn version(&self) -> Result<i64, StoreError> {
        let handle = self
            .data
            .db()
            .await
            .map_err(|error| StoreError(error.to_string()))?;
        db::current_version(&handle)
            .await
            .map_err(|error| StoreError(error.to_string()))
    }

    async fn load(&self) -> Result<Arc<Snapshot>, StoreError> {
        let handle = self
            .data
            .db()
            .await
            .map_err(|error| StoreError(error.to_string()))?;
        // Seed default weights for any exercise that has none before reading;
        // insert-only, so in steady state this writes nothing (archive/db.rs).
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        db::reconcile_muscle_weights(&handle, now)
            .await
            .map_err(|error| StoreError(error.to_string()))?;
        let (version, workouts, sets, tags, weights) = db::load_archive(&handle)
            .await
            .map_err(|error| StoreError(error.to_string()))?;
        snapshot::build(version, workouts, sets, tags, weights)
            .map(Arc::new)
            .map_err(|error| StoreError(error.to_string()))
    }
}

fn log_stale(error: &StoreError) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "fitness snapshot refresh failed; serving stale",
            "error": error.to_string(),
        })
    );
}
