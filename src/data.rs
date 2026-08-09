//! Lazy, shared SurrealDB access.
//!
//! A missing or unreachable database never prevents the binary from serving
//! pages that do not need stored data. The first data-backed request connects,
//! authenticates, selects the namespace/database, and reconciles the committed
//! schema. A failed initialization is not cached, so later requests can retry.

use std::{future::Future, sync::Arc, time::Duration};

use surrealdb::{
    Surreal,
    engine::any::{self, Any},
    opt::auth::Root,
};
use tokio::sync::OnceCell;

mod diary_migrations;
mod schema_migrations;

#[path = "app/analytics/models.rs"]
pub mod analytics_models;
#[path = "app/interests/lifting/models.rs"]
pub mod fitness_models;
#[path = "app/interests/podrick/models.rs"]
pub mod podrick_models;
#[path = "app/interests/spire/models.rs"]
pub mod spire_models;

pub type Db = Surreal<Any>;

pub const ENDPOINT_VAR: &str = "SURREALDB_ENDPOINT";
pub const NAMESPACE_VAR: &str = "SURREALDB_NAMESPACE";
pub const DATABASE_VAR: &str = "SURREALDB_DATABASE";
pub const USERNAME_VAR: &str = "SURREALDB_USERNAME";
pub const PASSWORD_VAR: &str = "SURREALDB_PASSWORD";

/// The diary direct-sync verification key (PEM, public half). When set, the
/// bootstrap defines the diary's record access method with it; unset, the
/// method is REMOVED and no token can open a session. Its table permission is
/// owned by the current diary migration (docs/diary-sync.md). The key is
/// config rather than committed schema so dev and prod differ and flag-off
/// means surface-off.
pub const DIRECT_SYNC_PUBLIC_KEY_VAR: &str = "DIARY_SYNC_JWT_PUBLIC_KEY";

const SCHEMA: &str = include_str!("schema.surql");
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(8);
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct DataConfig {
    pub endpoint: String,
    pub namespace: String,
    pub database: String,
    pub username: String,
    pub password: String,
}

#[derive(Clone)]
pub struct Data {
    config: Result<Arc<DataConfig>, &'static str>,
    cell: Arc<OnceCell<Db>>,
}

#[derive(Debug)]
pub enum DataError {
    Unconfigured(&'static str),
    Connect(String),
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::Unconfigured(variable) => write!(f, "{variable} is not set"),
            DataError::Connect(error) => write!(f, "database connect failed: {error}"),
        }
    }
}

impl std::error::Error for DataError {}

impl Data {
    pub fn from_env() -> Self {
        Self::new(DataConfig::from_env())
    }

    pub fn new(config: Result<DataConfig, &'static str>) -> Self {
        Data {
            config: config.map(Arc::new),
            cell: Arc::new(OnceCell::new()),
        }
    }

    /// A cheap clone of the shared client, connecting on first use.
    pub async fn db(&self) -> Result<Db, DataError> {
        let config = match &self.config {
            Ok(config) => config,
            Err(variable) => return Err(DataError::Unconfigured(variable)),
        };
        let db = self.cell.get_or_try_init(|| connect(config)).await?;
        Ok(db.clone())
    }

    /// The diary's stricter Interface: migrations run at initialization, then
    /// every use rechecks the shared epoch ledger. A cached predecessor thus
    /// refuses work after activation; deployments drain its in-flight work
    /// before advancing the epoch (docs/diary-sync.md).
    pub async fn diary_db(&self) -> Result<Db, DataError> {
        let db = self.db().await?;
        diary_migrations::require_current(&db)
            .await
            .map_err(|error| DataError::Connect(format!("diary schema check failed: {error}")))?;
        Ok(db)
    }
}

impl DataConfig {
    fn from_env() -> Result<Self, &'static str> {
        Ok(Self {
            endpoint: required_env(ENDPOINT_VAR)?,
            namespace: required_env(NAMESPACE_VAR)?,
            database: required_env(DATABASE_VAR)?,
            username: required_env(USERNAME_VAR)?,
            password: required_env(PASSWORD_VAR)?,
        })
    }
}

fn required_env(variable: &'static str) -> Result<String, &'static str> {
    std::env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(variable)
}

pub async fn connect(config: &DataConfig) -> Result<Db, DataError> {
    let db = timed(CONNECTION_TIMEOUT, "connection", async {
        any::connect(config.endpoint.as_str())
            .await
            .map_err(connect_error)
    })
    .await?;
    timed(CONNECTION_TIMEOUT, "authentication", async {
        db.signin(Root {
            username: config.username.clone(),
            password: config.password.clone(),
        })
        .await
        .map_err(connect_error)
    })
    .await?;
    timed(CONNECTION_TIMEOUT, "namespace selection", async {
        db.use_ns(config.namespace.clone())
            .use_db(config.database.clone())
            .await
            .map_err(connect_error)
    })
    .await?;
    timed(BOOTSTRAP_TIMEOUT, "schema reconciliation", async {
        db.query(SCHEMA)
            .await
            .map_err(connect_error)?
            .check()
            .map_err(connect_error)
            .map(|_| ())
    })
    .await?;
    timed(BOOTSTRAP_TIMEOUT, "schema migrations", async {
        schema_migrations::apply(&db)
            .await
            .map_err(|error| DataError::Connect(format!("schema migration failed: {error}")))
    })
    .await?;
    timed(BOOTSTRAP_TIMEOUT, "diary migrations", async {
        diary_migrations::apply(&db)
            .await
            .map_err(|error| DataError::Connect(format!("diary migration failed: {error}")))
    })
    .await?;
    timed(
        CONNECTION_TIMEOUT,
        "diary direct-sync configuration",
        async { define_direct_sync_access(&db).await },
    )
    .await?;
    timed(CONNECTION_TIMEOUT, "health verification", async {
        db.health().await.map_err(connect_error)
    })
    .await?;
    Ok(db)
}

async fn timed<T>(
    timeout: Duration,
    stage: &'static str,
    future: impl Future<Output = Result<T, DataError>>,
) -> Result<T, DataError> {
    tokio::time::timeout(timeout, future).await.map_err(|_| {
        DataError::Connect(format!("{stage} exceeded {} seconds", timeout.as_secs()))
    })?
}

/// Reconcile the diary direct-sync access method with the environment, the
/// same way the committed schema reconciles on every connect. MUST stay
/// `TYPE RECORD WITH JWT`: a plain `TYPE JWT` session carries a
/// database-level Viewer role floor that reads EVERY table regardless of
/// PERMISSIONS (probed on 3.2.3); record sessions are fully
/// permission-bound, which is the entire point. The PEM rides a bound
/// parameter — never string-built into the statement.
async fn define_direct_sync_access(db: &Db) -> Result<(), DataError> {
    match std::env::var(DIRECT_SYNC_PUBLIC_KEY_VAR)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        Some(public_key) => db
            .query(format!(
                "DEFINE ACCESS OVERWRITE {} ON DATABASE TYPE RECORD \
                 WITH JWT ALGORITHM ES256 KEY $public_key \
                 DURATION FOR SESSION 15m",
                diary_core::contract::DIRECT_ACCESS
            ))
            .bind(("public_key", public_key))
            .await
            .map_err(connect_error)?
            .check()
            .map_err(connect_error)
            .map(|_| ()),
        None => db
            .query(format!(
                "REMOVE ACCESS IF EXISTS {} ON DATABASE",
                diary_core::contract::DIRECT_ACCESS
            ))
            .await
            .map_err(connect_error)?
            .check()
            .map_err(connect_error)
            .map(|_| ()),
    }
}

fn connect_error(error: surrealdb::Error) -> DataError {
    DataError::Connect(error.to_string())
}
