//! Read-only checks and recovery of the web process's shared database session.
//!
//! Never bootstrap from here: deploy-time health requests must not apply diary
//! migrations while the preceding deployment still serves traffic.

use std::{sync::Arc, time::Duration};

use super::{Data, DataError, Db, connect_error, connect_session, timed};

const CHECK_INTERVAL: Duration = Duration::from_secs(30);
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const OUTAGE_BACKOFF: Duration = Duration::from_secs(60);
const FAILURE_LIMIT: u8 = 3;
const RECOVERY_LIMIT: u8 = 3;
const STABLE_CHECKS: u8 = 3;

// The baseline ledger row exists after bootstrap, is tiny, and has PERMISSIONS
// NONE. Requiring its value also catches sessions whose permissions silently
// filter all rows. A ping or RETURN true would not test auth/namespace/database.
const CHECK_QUERY: &str = "SELECT VALUE epoch FROM site_schema_migrations WHERE epoch = 1 LIMIT 1";

impl Data {
    async fn initialized_db(&self) -> Option<Arc<Db>> {
        Some(Arc::clone(&*self.cell.get()?.read().await))
    }

    /// Check the actual request session, without initializing or repairing it.
    /// The HTTP adapter deliberately does not expose internal error details.
    pub async fn readiness(&self) -> Result<(), DataError> {
        let db = self
            .initialized_db()
            .await
            .ok_or(DataError::NotInitialized)?;
        probe(&db).await
    }

    /// Run one serial recovery loop. Returns only to request a process restart.
    /// Keep this future owned by the server's shutdown future, not detached.
    /// Unconfigured and caller-supplied clients are never repaired or restarted.
    pub async fn maintain_connection(&self) {
        let Ok(_) = &self.config else {
            return std::future::pending().await;
        };
        let mut policy = RecoveryPolicy::default();
        loop {
            tokio::time::sleep(CHECK_INTERVAL).await;
            let Some(current) = self.initialized_db().await else {
                continue;
            };
            match probe(&current).await {
                Ok(()) => {
                    if policy.healthy() {
                        log("database connection stable", None);
                    }
                    continue;
                }
                Err(error) => {
                    log("database connection check failed", Some(&error));
                    if !policy.failed() {
                        continue;
                    }
                }
            }

            // A fresh authenticated query must work before replacing anything
            // or counting toward a restart. A database/network outage therefore
            // leaves the process serving independent pages and retries later.
            match self.recover_session(&current).await {
                Ok(replaced) => {
                    if replaced {
                        log("database connection replaced", None);
                        if policy.reconnected() {
                            log("database recovery exhausted; requesting restart", None);
                            return;
                        }
                    }
                }
                Err(error) => {
                    log("database unavailable; delaying recovery", Some(&error));
                    tokio::time::sleep(OUTAGE_BACKOFF).await;
                }
            }
        }
    }

    async fn recover_session(&self, current: &Arc<Db>) -> Result<bool, DataError> {
        let config = self
            .config
            .as_ref()
            .map_err(|variable| DataError::Unconfigured(variable))?;
        let candidate = timed(RECONNECT_TIMEOUT, "session recovery", async {
            let db = connect_session(config).await?;
            probe(&db).await?;
            Ok(Arc::new(db))
        })
        .await?;
        Ok(self.replace_session(current, candidate).await)
    }

    // Readers holding the old Arc may finish; new requests use the replacement.
    // No network operation holds the write lock. Compare before publishing so a
    // stale repair can never replace a newer session.
    async fn replace_session(&self, old: &Arc<Db>, replacement: Arc<Db>) -> bool {
        let Some(cell) = self.cell.get() else {
            return false;
        };
        let mut current = cell.write().await;
        if !Arc::ptr_eq(&current, old) {
            return false;
        }
        *current = replacement;
        true
    }
}

async fn probe(db: &Db) -> Result<(), DataError> {
    timed(CHECK_TIMEOUT, "database readiness query", async {
        let mut response = db
            .query(CHECK_QUERY)
            .await
            .map_err(connect_error)?
            .check()
            .map_err(connect_error)?;
        let epochs: Vec<u16> = response.take(0).map_err(connect_error)?;
        if epochs == [1] {
            Ok(())
        } else {
            Err(DataError::Connect(
                "database readiness row is unavailable".into(),
            ))
        }
    })
    .await
}

#[derive(Default)]
struct RecoveryPolicy {
    failures: u8,
    recoveries: u8,
    successes: u8,
}

impl RecoveryPolicy {
    fn healthy(&mut self) -> bool {
        self.failures = 0;
        self.successes = self.successes.saturating_add(1);
        if self.successes >= STABLE_CHECKS {
            let recovered = self.recoveries != 0;
            self.recoveries = 0;
            recovered
        } else {
            false
        }
    }

    fn failed(&mut self) -> bool {
        self.successes = 0;
        self.failures += 1;
        if self.failures < FAILURE_LIMIT {
            false
        } else {
            self.failures = 0;
            true
        }
    }

    fn reconnected(&mut self) -> bool {
        self.successes = 0;
        self.failures = 0;
        self.recoveries += 1;
        self.recoveries >= RECOVERY_LIMIT
    }
}

fn log(message: &str, error: Option<&DataError>) {
    eprintln!(
        "{}",
        serde_json::json!({ "message": message, "error": error.map(ToString::to_string) })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::engine::any;

    #[test]
    fn isolated_failures_never_reconnect() {
        let mut policy = RecoveryPolicy::default();
        for _ in 0..100 {
            assert!(!policy.failed());
            assert!(!policy.failed());
            assert!(!policy.healthy());
        }
    }

    #[test]
    fn an_outage_never_exhausts_recovery() {
        let mut policy = RecoveryPolicy::default();
        for _ in 0..100 {
            assert!(!policy.failed());
            assert!(!policy.failed());
            assert!(policy.failed());
            // A fresh connection also fails: no successful repair is counted.
        }
        assert!(!policy.reconnected());
    }

    #[test]
    fn repeated_session_loss_requests_restart_but_stability_resets_the_budget() {
        let mut policy = RecoveryPolicy::default();
        assert!(!policy.reconnected());
        assert!(!policy.healthy());
        assert!(!policy.reconnected());
        assert!(!policy.healthy());
        assert!(!policy.healthy());
        assert!(policy.healthy());
        assert!(!policy.reconnected());
        assert!(!policy.reconnected());
        assert!(policy.reconnected());
    }

    #[tokio::test]
    async fn readiness_never_initializes_the_database() {
        let data = Data::new(Err("test"));
        assert!(matches!(
            data.readiness().await,
            Err(DataError::NotInitialized)
        ));
        assert!(data.cell.get().is_none());
    }

    #[tokio::test]
    async fn readiness_requires_the_bootstrapped_ledger_and_selected_database() {
        let db = any::connect("mem://").await.unwrap();
        assert!(probe(&db).await.is_err());
        db.use_ns("recovery_test")
            .use_db("recovery_test")
            .await
            .unwrap();
        assert!(probe(&db).await.is_err());
        db.query("CREATE site_schema_migrations:baseline SET epoch = 1")
            .await
            .unwrap()
            .check()
            .unwrap();
        probe(&db).await.unwrap();
        db.query("DELETE site_schema_migrations:baseline")
            .await
            .unwrap()
            .check()
            .unwrap();
        assert!(probe(&db).await.is_err());
    }

    #[tokio::test]
    async fn replacement_reaches_every_data_clone_without_revoking_in_flight_handles() {
        let data = Data::from_initialized_db(Db::init());
        let other = data.clone();
        let old = data.db().await.unwrap();
        let new = Arc::new(Db::init());
        assert!(data.replace_session(&old, Arc::clone(&new)).await);
        assert!(Arc::ptr_eq(&other.db().await.unwrap(), &new));
        assert!(!Arc::ptr_eq(&old, &new));
        assert!(!data.replace_session(&old, Arc::new(Db::init())).await);
        assert!(Arc::ptr_eq(&data.db().await.unwrap(), &new));
    }
    #[tokio::test]
    async fn unconfigured_recovery_never_requests_restart() {
        let data = Data::new(Err("unused"));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), data.maintain_connection())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_stalled_operation_has_a_finite_budget() {
        let result = timed::<()>(
            Duration::from_millis(10),
            "test operation",
            std::future::pending(),
        )
        .await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("test operation exceeded")
        );
    }

    #[tokio::test]
    #[ignore = "requires the local SurrealDB 3.2.3 container on port 5800 (root/dev)"]
    async fn websocket_session_recovery_preserves_data_without_bootstrap() {
        let namespace = format!("recovery_{}", uuid::Uuid::new_v4().simple());
        let config = super::super::DataConfig {
            endpoint: "ws://127.0.0.1:5800".into(),
            namespace: namespace.clone(),
            database: "test".into(),
            username: "root".into(),
            password: "dev".into(),
        };
        let admin = connect_session(&config).await.unwrap();
        admin.query("DEFINE TABLE site_schema_migrations SCHEMALESS PERMISSIONS NONE; CREATE site_schema_migrations:baseline SET epoch = 1")
            .await.unwrap().check().unwrap();
        let data = Data::new(Ok(config.clone()));
        data.cell
            .set(tokio::sync::RwLock::new(Arc::new(
                connect_session(&config).await.unwrap(),
            )))
            .unwrap();
        let reader = data.clone();
        let old = data.db().await.unwrap();
        data.readiness().await.unwrap();
        old.invalidate().await.unwrap();
        assert!(data.readiness().await.is_err());
        assert!(data.recover_session(&old).await.unwrap());
        reader.readiness().await.unwrap();
        assert!(!Arc::ptr_eq(&old, &reader.db().await.unwrap()));
        assert!(probe(&old).await.is_err());
        let mut response = admin
            .query("SELECT VALUE epoch FROM site_schema_migrations ORDER BY epoch")
            .await
            .unwrap()
            .check()
            .unwrap();
        let epochs: Vec<u16> = response.take(0).unwrap();
        assert_eq!(epochs, [1], "recovery must not apply migrations");

        // Unreachable fresh connection: keep the current handle, do not publish
        // an unauthenticated/unverified replacement.
        let mut bad_config = config;
        bad_config.password = "deliberately-wrong".into();
        let broken = Data::new(Ok(bad_config));
        let current = reader.db().await.unwrap();
        broken
            .cell
            .set(tokio::sync::RwLock::new(Arc::clone(&current)))
            .unwrap();
        assert!(broken.recover_session(&current).await.is_err());
        assert!(Arc::ptr_eq(&current, &broken.db().await.unwrap()));
        admin
            .query(format!("REMOVE NAMESPACE {namespace}"))
            .await
            .unwrap()
            .check()
            .unwrap();
    }
}
