//! Lazy, shared SurrealDB access.
//!
//! A missing or unreachable database never prevents the binary from serving
//! pages that do not need stored data. The first data-backed request connects,
//! authenticates, selects the namespace/database, and reconciles the committed
//! schema. A failed initialization is not cached, so later requests can retry.

use std::{sync::Arc, time::Duration};

use surrealdb::{
    Surreal,
    engine::any::{self, Any},
    opt::auth::Root,
};
use tokio::sync::OnceCell;

#[path = "app/analytics/models.rs"]
pub mod analytics_models;
#[path = "app/interests/lifting/models.rs"]
pub mod fitness_models;
#[path = "app/interests/spire/models.rs"]
pub mod spire_models;

pub type Db = Surreal<Any>;

pub const ENDPOINT_VAR: &str = "SURREALDB_ENDPOINT";
pub const NAMESPACE_VAR: &str = "SURREALDB_NAMESPACE";
pub const DATABASE_VAR: &str = "SURREALDB_DATABASE";
pub const USERNAME_VAR: &str = "SURREALDB_USERNAME";
pub const PASSWORD_VAR: &str = "SURREALDB_PASSWORD";

const SCHEMA: &str = include_str!("schema.surql");
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

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
        let db = self
            .cell
            .get_or_try_init(|| async {
                tokio::time::timeout(CONNECT_TIMEOUT, connect(config))
                    .await
                    .map_err(|_| {
                        DataError::Connect(format!(
                            "initialization exceeded {} seconds",
                            CONNECT_TIMEOUT.as_secs()
                        ))
                    })?
            })
            .await?;
        Ok(db.clone())
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
    let db = any::connect(config.endpoint.as_str())
        .await
        .map_err(connect_error)?;
    db.signin(Root {
        username: config.username.clone(),
        password: config.password.clone(),
    })
    .await
    .map_err(connect_error)?;
    db.use_ns(config.namespace.clone())
        .use_db(config.database.clone())
        .await
        .map_err(connect_error)?;
    db.query(SCHEMA)
        .await
        .map_err(connect_error)?
        .check()
        .map_err(connect_error)?;
    db.health().await.map_err(connect_error)?;
    Ok(db)
}

fn connect_error(error: surrealdb::Error) -> DataError {
    DataError::Connect(error.to_string())
}
