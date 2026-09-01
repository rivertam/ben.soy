//! OAuth-protected MCP facade for the complete fitness store.
//!
//! The service deliberately exposes a bounded record API rather than raw
//! SurrealQL. Table and field names come from a closed catalog, values are
//! bound parameters, reads are paginated, and writes are small atomic batches.
//! It runs inside the site process and deliberately shares the site's root
//! database connection. The external boundary is therefore OAuth plus this
//! module's closed operation catalog, validation, and bound query values.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, Implementation, MetaObject, ServerCapabilities, ServerInfo,
    },
    object,
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::{
        StreamableHttpServerConfig,
        streamable_http_server::{
            session::local::LocalSessionManager, tower::StreamableHttpService,
        },
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use surrealdb::types::SerdeWrapper;
use tokio::sync::{Mutex, Semaphore};
use topcoat::router::{Method, Methods, RouterBuilder, tower::TowerRoute};
use tracing::{info, warn};
use url::Url;

use crate::data::{Data, Db};

const READ_SCOPE: &str = "fitness:read";
const WRITE_SCOPE: &str = "fitness:write";
const SITE_ORIGIN_VAR: &str = "SITE_ORIGIN";
const OAUTH_ISSUER_VAR: &str = "FITNESS_MCP_OAUTH_ISSUER";
const ALLOWED_SUBJECTS_VAR: &str = "FITNESS_MCP_ALLOWED_SUBJECTS";
const ALLOWED_HOSTS_VAR: &str = "FITNESS_MCP_ALLOWED_HOSTS";
const ALLOWED_ORIGINS_VAR: &str = "FITNESS_MCP_ALLOWED_ORIGINS";
const MCP_PATH: &str = "/mcp";
const METADATA_PATH: &str = "/.well-known/oauth-protected-resource/mcp";
const FALLBACK_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";
const QUERY_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_READ_ROWS: u16 = 500;
const MAX_OUTPUT_BYTES: usize = 1_000_000;
const MAX_CHANGES: usize = 50;
const MAX_CHANGE_BYTES: usize = 256 * 1024;
const JWKS_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FitnessTable {
    Workouts,
    Exercises,
    ExerciseAliases,
    ExerciseTags,
    Sets,
    FitnessMeta,
    RunningActivities,
    Muscles,
    ExerciseMuscles,
    FitnessInterruptions,
}

#[derive(Clone, Copy)]
struct TableSpec {
    table: FitnessTable,
    name: &'static str,
    description: &'static str,
    fields: &'static [(&'static str, &'static str)],
    mutable: bool,
    bumps_version: bool,
}

const WORKOUT_FIELDS: &[(&str, &str)] = &[
    ("id", "string record key"),
    ("title", "string"),
    ("raw_title", "string"),
    ("started_at_utc", "string"),
    ("started_at_local", "string"),
    ("eastern_offset_minutes", "int"),
    ("duration_seconds", "int"),
    ("duration_suspicious", "bool"),
    ("notes", "optional string"),
    ("description", "optional string"),
    ("source", "string"),
    ("imported_at", "int unix seconds"),
];
const EXERCISE_FIELDS: &[(&str, &str)] = &[("id", "string record key"), ("name", "string")];
const EXERCISE_ALIAS_FIELDS: &[(&str, &str)] = &[
    ("id", "string record key"),
    ("alias_name", "alternate imported exercise name"),
    ("canonical_name", "canonical exercise name"),
    ("updated_at", "int unix seconds"),
];
const TAG_FIELDS: &[(&str, &str)] = &[
    ("id", "string record key"),
    ("exercise_name", "string"),
    ("kind", "string"),
    ("value", "string"),
];
const SET_FIELDS: &[(&str, &str)] = &[
    ("id", "string record key"),
    ("workout_id", "string workout key"),
    ("exercise_name", "string"),
    ("raw_exercise_name", "string"),
    ("ordinal", "int"),
    ("exercise_note", "optional string"),
    ("superset_id", "optional int"),
    ("weight_milli", "optional int; 1000 = one weight unit"),
    ("weight_unit", "string"),
    ("reps", "optional int"),
    ("effort_hundredths", "optional int; 100 = one RPE point"),
    ("distance_milli", "optional int; 1000 = one distance unit"),
    ("set_time_seconds", "optional int"),
    ("set_type", "string"),
    ("incomplete", "bool"),
];
const META_FIELDS: &[(&str, &str)] = &[("id", "string record key"), ("k", "string"), ("v", "int")];
const RUN_FIELDS: &[(&str, &str)] = &[
    ("id", "64-hex string record key"),
    ("source", "garmin-connect or manual"),
    ("source_activity_id", "string"),
    ("source_url", "optional canonical Garmin URL"),
    ("title", "string"),
    ("activity_type", "string"),
    ("started_at_utc", "YYYY-MM-DD HH:MM:SS"),
    ("started_at_local", "YYYY-MM-DD HH:MM:SS"),
    ("eastern_offset_minutes", "-300 or -240"),
    ("duration_milliseconds", "int"),
    ("moving_duration_milliseconds", "optional int"),
    ("distance_millimeters", "int"),
    ("ascent_millimeters", "optional int"),
    ("imported_at", "int unix seconds"),
];
const MUSCLE_FIELDS: &[(&str, &str)] = &[
    ("id", "string record key"),
    ("name", "curated muscle id"),
    ("label", "string"),
    ("muscle_group", "curated group id"),
    ("ordinal", "int"),
];
const EXERCISE_MUSCLE_FIELDS: &[(&str, &str)] = &[
    ("id", "64-hex string record key"),
    ("exercise_name", "string"),
    ("muscle", "curated muscle id"),
    ("ratio_hundredths", "int 1..100"),
    ("source", "seed, derived, or admin"),
    ("updated_at", "int unix seconds"),
];
const INTERRUPTION_FIELDS: &[(&str, &str)] = &[
    ("id", "32-hex string record key"),
    ("from_date", "YYYY-MM-DD inclusive"),
    ("to_date", "optional YYYY-MM-DD inclusive"),
    ("note", "string"),
    ("emoji", "string"),
    ("updated_at", "int unix seconds"),
];

const TABLES: &[TableSpec] = &[
    TableSpec {
        table: FitnessTable::Workouts,
        name: "workouts",
        description: "Lifting workout headers; sets join through workout_id.",
        fields: WORKOUT_FIELDS,
        mutable: true,
        bumps_version: true,
    },
    TableSpec {
        table: FitnessTable::Exercises,
        name: "exercises",
        description: "Canonical lifting exercise names.",
        fields: EXERCISE_FIELDS,
        mutable: true,
        bumps_version: true,
    },
    TableSpec {
        table: FitnessTable::ExerciseAliases,
        name: "exercise_aliases",
        description: "Alternate source names resolving to canonical lifting exercises.",
        fields: EXERCISE_ALIAS_FIELDS,
        mutable: true,
        bumps_version: true,
    },
    TableSpec {
        table: FitnessTable::ExerciseTags,
        name: "exercise_tags",
        description: "Many taxonomy facet rows per exercise.",
        fields: TAG_FIELDS,
        mutable: true,
        bumps_version: true,
    },
    TableSpec {
        table: FitnessTable::Sets,
        name: "sets",
        description: "Performed lifting sets; raw integer units are preserved.",
        fields: SET_FIELDS,
        mutable: true,
        bumps_version: true,
    },
    TableSpec {
        table: FitnessTable::FitnessMeta,
        name: "fitness_meta",
        description: "Internal version clock. Read fitness_meta:version before a lifting write.",
        fields: META_FIELDS,
        mutable: false,
        bumps_version: false,
    },
    TableSpec {
        table: FitnessTable::RunningActivities,
        name: "running_activities",
        description: "Route-free running summaries; GPS and device details are never stored.",
        fields: RUN_FIELDS,
        mutable: true,
        bumps_version: false,
    },
    TableSpec {
        table: FitnessTable::Muscles,
        name: "muscles",
        description: "The code-aligned 28-muscle vocabulary.",
        fields: MUSCLE_FIELDS,
        mutable: true,
        bumps_version: true,
    },
    TableSpec {
        table: FitnessTable::ExerciseMuscles,
        name: "exercise_muscles",
        description: "Weighted exercise-to-muscle credit; admin rows block reseeding.",
        fields: EXERCISE_MUSCLE_FIELDS,
        mutable: true,
        bumps_version: true,
    },
    TableSpec {
        table: FitnessTable::FitnessInterruptions,
        name: "fitness_interruptions",
        description: "Annotate-only illness, travel, or other training interruptions.",
        fields: INTERRUPTION_FIELDS,
        mutable: true,
        bumps_version: true,
    },
];

impl FitnessTable {
    fn spec(self) -> &'static TableSpec {
        TABLES
            .iter()
            .find(|spec| spec.table == self)
            .expect("every fitness table enum has a catalog entry")
    }
}

fn catalog() -> Value {
    json!({
        "tables": TABLES.iter().map(|spec| json!({
            "name": spec.name,
            "description": spec.description,
            "mutable": spec.mutable,
            "bumps_lifting_version": spec.bumps_version,
            "fields": spec.fields.iter().map(|(name, kind)| json!({
                "name": name,
                "type": kind,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "rules": [
            "All stored quantities use the raw integer units documented on fields.",
            "Records are derived from set history and are never stored.",
            "Deleting a workout also deletes its sets, but deliberately leaves exercise taxonomy and muscle rows.",
            "Exercise aliases are one-hop: an alias is not a canonical name or another alias; use the exercise page for an atomic canonical rename.",
            "A lifting mutation requires the current fitness_meta:version and bumps it exactly once.",
            "Use paginated reads plus client-side code for complex analysis; raw SurrealQL is not exposed.",
        ],
    })
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    In,
    Contains,
    IsNone,
    IsNotNone,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RecordFilter {
    /// A field from the selected table's catalog.
    pub field: String,
    pub operator: FilterOperator,
    /// Omit for is_none/is_not_none. `in` requires an array.
    #[serde(default)]
    pub value: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RecordSort {
    pub field: String,
    pub direction: SortDirection,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct ReadRecordsRequest {
    pub table: FitnessTable,
    /// Omit for every field. `id` is always returned as the plain record key.
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Flat filters joined with AND by default, or OR when match_any is true.
    #[serde(default)]
    pub filters: Vec<RecordFilter>,
    #[serde(default)]
    pub match_any: bool,
    #[serde(default)]
    pub order_by: Vec<RecordSort>,
    /// Defaults to 100; maximum 500.
    #[serde(default)]
    pub limit: Option<u16>,
    /// Zero-based pagination offset.
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Create,
    Replace,
    Merge,
    Upsert,
    Delete,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RecordChange {
    pub action: ChangeAction,
    pub table: FitnessTable,
    /// Plain record key, without the table prefix.
    pub id: String,
    /// Required for every action except delete. Never include `id` here.
    #[serde(default)]
    pub data: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct ApplyChangesRequest {
    /// Must be true. The calling agent should summarize the exact batch first.
    pub confirmed: bool,
    /// Short human-readable purpose, returned in the receipt but never logged.
    pub reason: String,
    /// Required when any change touches the lifting archive; read fitness_meta:version first.
    #[serde(default)]
    pub expected_version: Option<i64>,
    pub changes: Vec<RecordChange>,
}

#[derive(Clone)]
struct ServiceConfig {
    resource_url: String,
    metadata_url: String,
    oauth_issuer: String,
    oauth_audience: String,
    allowed_subjects: HashSet<String>,
    allowed_hosts: Vec<String>,
    allowed_origins: Vec<String>,
}

impl ServiceConfig {
    fn from_env() -> anyhow::Result<Option<Self>> {
        let oauth_issuer = optional_env(OAUTH_ISSUER_VAR);
        let allowed_subjects = optional_env(ALLOWED_SUBJECTS_VAR);
        let (oauth_issuer, allowed_subjects) = match (oauth_issuer, allowed_subjects) {
            (None, None) => return Ok(None),
            (Some(issuer), Some(subjects)) => (issuer, subjects),
            (None, Some(_)) => anyhow::bail!(
                "{ALLOWED_SUBJECTS_VAR} is set but {OAUTH_ISSUER_VAR} is not; set both or neither"
            ),
            (Some(_), None) => anyhow::bail!(
                "{OAUTH_ISSUER_VAR} is set but {ALLOWED_SUBJECTS_VAR} is not; set both or neither"
            ),
        };

        let site_origin = Url::parse(&required_env(SITE_ORIGIN_VAR)?)?;
        if site_origin.scheme() != "https"
            || site_origin.cannot_be_a_base()
            || site_origin.host_str().is_none()
            || site_origin.path() != "/"
            || site_origin.query().is_some()
            || site_origin.fragment().is_some()
        {
            anyhow::bail!("{SITE_ORIGIN_VAR} must be an HTTPS origin without a path");
        }
        let origin = site_origin.origin().ascii_serialization();
        let resource_url = format!("{origin}{MCP_PATH}");
        let metadata_url = format!("{origin}{METADATA_PATH}");

        let issuer_url = Url::parse(&oauth_issuer)?;
        if issuer_url.scheme() != "https"
            || issuer_url.query().is_some()
            || issuer_url.fragment().is_some()
        {
            anyhow::bail!("{OAUTH_ISSUER_VAR} must be an HTTPS issuer URL");
        }
        let oauth_issuer = issuer_url.to_string();
        let oauth_audience = resource_url.clone();
        let allowed_subjects = split_env_list(&allowed_subjects);
        if allowed_subjects.is_empty() {
            anyhow::bail!("{ALLOWED_SUBJECTS_VAR} must name at least one exact OAuth subject");
        }

        let public_authority = match site_origin.port() {
            Some(port) => format!(
                "{}:{port}",
                site_origin.host_str().expect("HTTPS URL has a host")
            ),
            None => site_origin
                .host_str()
                .expect("HTTPS URL has a host")
                .to_string(),
        };
        let allowed_hosts = optional_env(ALLOWED_HOSTS_VAR)
            .map(|value| split_list(&value).into_iter().collect())
            .unwrap_or_else(|| {
                vec![
                    public_authority,
                    "localhost".to_string(),
                    "127.0.0.1".to_string(),
                    "::1".to_string(),
                ]
            });
        let allowed_origins = optional_env(ALLOWED_ORIGINS_VAR)
            .map(|value| split_list(&value).into_iter().collect())
            .unwrap_or_else(|| {
                vec![
                    origin,
                    "https://chatgpt.com".to_string(),
                    "https://chat.openai.com".to_string(),
                ]
            });

        Ok(Some(Self {
            resource_url,
            metadata_url,
            oauth_issuer,
            oauth_audience,
            allowed_subjects,
            allowed_hosts,
            allowed_origins,
        }))
    }
}

fn required_env(variable: &'static str) -> anyhow::Result<String> {
    optional_env(variable).ok_or_else(|| anyhow::anyhow!("{variable} is not set"))
}

fn optional_env(variable: &str) -> Option<String> {
    std::env::var(variable)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn split_env_list(value: &str) -> HashSet<String> {
    split_list(value).into_iter().collect()
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[derive(Clone)]
struct OAuthVerifier {
    client: reqwest::Client,
    issuer: String,
    audience: String,
    allowed_subjects: HashSet<String>,
    cache: Arc<Mutex<Option<CachedJwks>>>,
}

#[derive(Clone)]
struct CachedJwks {
    loaded_at: Instant,
    keys: JwkSet,
}

#[derive(Debug, Deserialize)]
struct OidcMetadata {
    issuer: String,
    jwks_uri: String,
}

#[derive(Debug, Deserialize)]
struct OAuthClaims {
    sub: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    permissions: Vec<String>,
}

#[derive(Clone, Debug)]
struct VerifiedUser {
    subject_hash: String,
    scopes: HashSet<String>,
}

impl VerifiedUser {
    fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }
}

impl OAuthVerifier {
    fn new(config: &ServiceConfig) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("benjisponge-fitness-mcp/0.1")
            .build()?;
        Ok(Self {
            client,
            issuer: config.oauth_issuer.clone(),
            audience: config.oauth_audience.clone(),
            allowed_subjects: config.allowed_subjects.clone(),
            cache: Arc::new(Mutex::new(None)),
        })
    }

    async fn verify(&self, token: &str) -> Result<VerifiedUser, &'static str> {
        if token.len() > 16 * 1024 {
            return Err("token is too large");
        }
        let header = decode_header(token).map_err(|_| "token header is invalid")?;
        if header.alg != Algorithm::RS256 {
            return Err("token algorithm must be RS256");
        }
        let kid = header.kid.ok_or("token has no key id")?;
        let keys = self.keys().await?;
        let jwk = keys.find(&kid).ok_or("token key is unknown")?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| "token key is invalid")?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[self.audience.as_str()]);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = 30;
        let claims = decode::<OAuthClaims>(token, &key, &validation)
            .map_err(|_| "token validation failed")?
            .claims;
        if !self.allowed_subjects.contains(&claims.sub) {
            return Err("subject is not allowed");
        }
        let mut scopes: HashSet<String> = claims
            .scope
            .split_whitespace()
            .map(str::to_string)
            .collect();
        scopes.extend(claims.permissions);
        let subject_hash = Sha256::digest(claims.sub.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(VerifiedUser {
            subject_hash,
            scopes,
        })
    }

    async fn keys(&self) -> Result<JwkSet, &'static str> {
        // Keep the lock through refresh so a cold start or a burst of forged
        // unknown-kid tokens can cause at most one discovery/JWKS fetch. Key
        // rollover may take up to this short TTL to appear, which Auth0's
        // overlapping signing keys comfortably tolerate.
        let mut cache = self.cache.lock().await;
        if let Some(cache) = cache.as_ref()
            && cache.loaded_at.elapsed() < JWKS_CACHE_TTL
        {
            return Ok(cache.keys.clone());
        }
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.issuer.trim_end_matches('/')
        );
        let metadata: OidcMetadata = self.fetch_json(&discovery_url).await?;
        if metadata.issuer != self.issuer {
            return Err("authorization server issuer mismatch");
        }
        let jwks_url = Url::parse(&metadata.jwks_uri).map_err(|_| "JWKS URL is invalid")?;
        if jwks_url.scheme() != "https" {
            return Err("JWKS URL is not HTTPS");
        }
        let keys: JwkSet = self.fetch_json(jwks_url.as_str()).await?;
        if keys.keys.is_empty() {
            return Err("authorization server returned no keys");
        }
        *cache = Some(CachedJwks {
            loaded_at: Instant::now(),
            keys: keys.clone(),
        });
        Ok(keys)
    }

    async fn fetch_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, &'static str> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| "authorization metadata request failed")?;
        if !response.status().is_success() {
            return Err("authorization metadata request was rejected");
        }
        if response
            .content_length()
            .is_some_and(|length| length > 1_000_000)
        {
            return Err("authorization metadata is too large");
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| "authorization metadata body failed")?;
        if bytes.len() > 1_000_000 {
            return Err("authorization metadata is too large");
        }
        serde_json::from_slice(&bytes).map_err(|_| "authorization metadata is invalid")
    }
}

#[derive(Debug)]
struct BuiltQuery {
    sql: String,
    bindings: BTreeMap<String, Value>,
    limit: u16,
    offset: u32,
}

fn field_expression(spec: &TableSpec, field: &str) -> Result<&'static str, String> {
    let (name, _) = spec
        .fields
        .iter()
        .find(|(candidate, _)| *candidate == field)
        .ok_or_else(|| format!("unknown field `{field}` for `{}`", spec.name))?;
    if *name == "id" {
        Ok("record::id(id)")
    } else {
        Ok(name)
    }
}

fn build_read_query(request: &ReadRecordsRequest) -> Result<BuiltQuery, String> {
    let spec = request.table.spec();
    if request.filters.len() > 20 {
        return Err("at most 20 filters are allowed".to_string());
    }
    if request.order_by.len() > 4 {
        return Err("at most 4 sort fields are allowed".to_string());
    }
    let limit = request.limit.unwrap_or(100);
    if limit == 0 || limit > MAX_READ_ROWS {
        return Err(format!("limit must be between 1 and {MAX_READ_ROWS}"));
    }
    let offset = request.offset.unwrap_or(0);
    if offset > 1_000_000 {
        return Err("offset may not exceed 1,000,000".to_string());
    }

    let projection = match &request.fields {
        None => "*, record::id(id) AS id".to_string(),
        Some(fields) => {
            if fields.is_empty() {
                return Err("fields must be omitted or contain at least one field".to_string());
            }
            if fields.len() > 40 {
                return Err("at most 40 fields are allowed".to_string());
            }
            let mut seen = BTreeSet::new();
            let mut selected = Vec::new();
            for field in fields {
                if !seen.insert(field.clone()) {
                    return Err(format!("field `{field}` is repeated"));
                }
                let expression = field_expression(spec, field)?;
                if field == "id" {
                    selected.push("record::id(id) AS id".to_string());
                } else {
                    selected.push(expression.to_string());
                }
            }
            if !seen.contains("id") {
                selected.insert(0, "record::id(id) AS id".to_string());
            }
            selected.join(", ")
        }
    };

    let mut bindings = BTreeMap::new();
    let mut conditions = Vec::new();
    for (index, filter) in request.filters.iter().enumerate() {
        let field = field_expression(spec, &filter.field)?;
        let condition = match filter.operator {
            FilterOperator::IsNone => {
                if filter.value.is_some() {
                    return Err(format!("filter {index}: is_none does not accept a value"));
                }
                format!("{field} IS NONE")
            }
            FilterOperator::IsNotNone => {
                if filter.value.is_some() {
                    return Err(format!(
                        "filter {index}: is_not_none does not accept a value"
                    ));
                }
                format!("{field} IS NOT NONE")
            }
            operator => {
                let value = filter
                    .value
                    .clone()
                    .ok_or_else(|| format!("filter {index}: value is required"))?;
                if matches!(operator, FilterOperator::In) && !value.is_array() {
                    return Err(format!("filter {index}: in requires an array value"));
                }
                let binding = format!("filter_{index}");
                bindings.insert(binding.clone(), value);
                let operator = match operator {
                    FilterOperator::Eq => "=",
                    FilterOperator::Ne => "!=",
                    FilterOperator::Lt => "<",
                    FilterOperator::Lte => "<=",
                    FilterOperator::Gt => ">",
                    FilterOperator::Gte => ">=",
                    FilterOperator::In => "IN",
                    FilterOperator::Contains => "CONTAINS",
                    FilterOperator::IsNone | FilterOperator::IsNotNone => unreachable!(),
                };
                format!("{field} {operator} ${binding}")
            }
        };
        conditions.push(condition);
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        let join = if request.match_any { " OR " } else { " AND " };
        format!(" WHERE {}", conditions.join(join))
    };
    let order_clause = if request.order_by.is_empty() {
        " ORDER BY id ASC".to_string()
    } else {
        let mut order = Vec::new();
        let mut includes_id = false;
        for sort in &request.order_by {
            let field = field_expression(spec, &sort.field)?;
            includes_id |= sort.field == "id";
            let direction = match sort.direction {
                SortDirection::Asc => "ASC",
                SortDirection::Desc => "DESC",
            };
            order.push(format!("{field} {direction}"));
        }
        if !includes_id {
            order.push("id ASC".to_string());
        }
        format!(" ORDER BY {}", order.join(", "))
    };
    Ok(BuiltQuery {
        sql: format!(
            "SELECT {projection} FROM {}{where_clause}{order_clause} LIMIT {limit} START {offset};",
            spec.name
        ),
        bindings,
        limit,
        offset,
    })
}

async fn read_records(db: &Db, request: &ReadRecordsRequest) -> Result<Value, String> {
    let built = build_read_query(request)?;
    let mut query = db.query(built.sql);
    for (name, value) in built.bindings {
        query = query.bind((name, SerdeWrapper(value)));
    }
    let mut response = tokio::time::timeout(QUERY_TIMEOUT, query)
        .await
        .map_err(|_| "fitness read timed out".to_string())?
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let rows: Vec<SerdeWrapper<Value>> = response.take(0).map_err(|error| error.to_string())?;
    let rows: Vec<Value> = rows.into_iter().map(|row| row.0).collect();
    let returned = rows.len();
    let next_offset = (returned == usize::from(built.limit))
        .then(|| built.offset.saturating_add(u32::from(built.limit)));
    let result = json!({
        "table": request.table.spec().name,
        "offset": built.offset,
        "returned": returned,
        "next_offset": next_offset,
        "records": rows,
    });
    ensure_output_bound(&result)?;
    Ok(result)
}

fn ensure_output_bound(value: &Value) -> Result<(), String> {
    let size = serde_json::to_vec(value)
        .map_err(|error| format!("result serialization failed: {error}"))?
        .len();
    if size > MAX_OUTPUT_BYTES {
        Err(format!(
            "result is {size} bytes; request fewer fields or rows (maximum {MAX_OUTPUT_BYTES})"
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug)]
struct BuiltMutation {
    sql: String,
    bindings: BTreeMap<String, Value>,
    bumps_version: bool,
}

fn validate_change_data(
    index: usize,
    change: &RecordChange,
    spec: &TableSpec,
) -> Result<Option<Value>, String> {
    if change.id.is_empty() || change.id.len() > 256 || change.id.chars().any(char::is_control) {
        return Err(format!(
            "change {index}: id must be 1..256 non-control characters"
        ));
    }
    match change.action {
        ChangeAction::Delete => {
            if change.data.is_some() {
                return Err(format!("change {index}: delete must omit data"));
            }
            Ok(None)
        }
        _ => {
            let data = change
                .data
                .clone()
                .ok_or_else(|| format!("change {index}: data is required"))?;
            if data.is_empty() {
                return Err(format!("change {index}: data may not be empty"));
            }
            if data.contains_key("id") {
                return Err(format!(
                    "change {index}: pass the record key as id, not inside data"
                ));
            }
            for field in data.keys() {
                field_expression(spec, field)
                    .map_err(|_| format!("change {index}: unknown field `{field}`"))?;
            }
            Ok(Some(Value::Object(data)))
        }
    }
}

fn build_mutation(request: &ApplyChangesRequest) -> Result<BuiltMutation, String> {
    if !request.confirmed {
        return Err("confirmed must be true after the user reviews the batch".to_string());
    }
    if request.reason.trim().len() < 3 || request.reason.len() > 500 {
        return Err("reason must be between 3 and 500 bytes".to_string());
    }
    if request.changes.is_empty() || request.changes.len() > MAX_CHANGES {
        return Err(format!(
            "changes must contain between 1 and {MAX_CHANGES} operations"
        ));
    }
    let encoded_size = serde_json::to_vec(&json!({
        "reason": request.reason,
        "expected_version": request.expected_version,
        "changes": request.changes.iter().map(|change| json!({
            "action": change.action,
            "table": change.table.spec().name,
            "id": change.id,
            "data": change.data,
        })).collect::<Vec<_>>(),
    }))
    .map_err(|error| error.to_string())?
    .len();
    if encoded_size > MAX_CHANGE_BYTES {
        return Err(format!(
            "change batch is {encoded_size} bytes; maximum is {MAX_CHANGE_BYTES}"
        ));
    }

    let mut seen = BTreeSet::new();
    let mut bindings = BTreeMap::new();
    let mut statements = String::from("BEGIN TRANSACTION;\n");
    let bumps_version = request
        .changes
        .iter()
        .any(|change| change.table.spec().bumps_version);
    if bumps_version {
        let expected = request.expected_version.ok_or_else(|| {
            "expected_version is required for changes to lifting tables".to_string()
        })?;
        if expected < 0 {
            return Err("expected_version may not be negative".to_string());
        }
        bindings.insert("expected_version".to_string(), json!(expected));
        statements.push_str(
            "IF ((SELECT VALUE v FROM fitness_meta:version)[0] ?? 0) != $expected_version {\n\
                 THROW 'fitness version changed; read fitness_meta:version and review the batch again';\n\
             };\n",
        );
    }

    for (index, change) in request.changes.iter().enumerate() {
        let spec = change.table.spec();
        if !spec.mutable {
            return Err(format!(
                "change {index}: `{}` is service-managed and read-only",
                spec.name
            ));
        }
        if !seen.insert((change.table, change.id.clone())) {
            return Err(format!(
                "change {index}: `{}` / `{}` appears more than once",
                spec.name, change.id
            ));
        }
        let data = validate_change_data(index, change, spec)?;
        let table_binding = format!("table_{index}");
        let id_binding = format!("id_{index}");
        let record_binding = format!("record_{index}");
        let error_binding = format!("error_{index}");
        bindings.insert(table_binding.clone(), json!(spec.name));
        bindings.insert(id_binding.clone(), json!(change.id));
        bindings.insert(
            error_binding.clone(),
            json!(format!(
                "change {index} could not be applied to {}:{}",
                spec.name, change.id
            )),
        );
        statements.push_str(&format!(
            "LET ${record_binding} = type::record(${table_binding}, ${id_binding});\n"
        ));
        match change.action {
            ChangeAction::Create => {
                let data_binding = format!("data_{index}");
                bindings.insert(
                    data_binding.clone(),
                    data.expect("create data was validated"),
                );
                statements.push_str(&format!(
                    "IF record::exists(${record_binding}) {{ THROW ${error_binding}; }};\n\
                     CREATE ONLY ${record_binding} CONTENT ${data_binding} RETURN NONE;\n\
                     IF !record::exists(${record_binding}) {{ THROW ${error_binding}; }};\n"
                ));
            }
            ChangeAction::Replace => {
                let data_binding = format!("data_{index}");
                bindings.insert(
                    data_binding.clone(),
                    data.expect("replace data was validated"),
                );
                statements.push_str(&format!(
                    "IF !record::exists(${record_binding}) {{ THROW ${error_binding}; }};\n\
                     UPDATE ONLY ${record_binding} CONTENT ${data_binding} RETURN NONE;\n\
                     IF !record::exists(${record_binding}) {{ THROW ${error_binding}; }};\n"
                ));
            }
            ChangeAction::Merge => {
                let data_binding = format!("data_{index}");
                bindings.insert(
                    data_binding.clone(),
                    data.expect("merge data was validated"),
                );
                statements.push_str(&format!(
                    "IF !record::exists(${record_binding}) {{ THROW ${error_binding}; }};\n\
                     UPDATE ONLY ${record_binding} MERGE ${data_binding} RETURN NONE;\n\
                     IF !record::exists(${record_binding}) {{ THROW ${error_binding}; }};\n"
                ));
            }
            ChangeAction::Upsert => {
                let data_binding = format!("data_{index}");
                bindings.insert(
                    data_binding.clone(),
                    data.expect("upsert data was validated"),
                );
                statements.push_str(&format!(
                    "UPSERT ONLY ${record_binding} MERGE ${data_binding} RETURN NONE;\n\
                     IF !record::exists(${record_binding}) {{ THROW ${error_binding}; }};\n"
                ));
            }
            ChangeAction::Delete => {
                statements.push_str(&format!(
                    "IF !record::exists(${record_binding}) {{ THROW ${error_binding}; }};\n"
                ));
                if change.table == FitnessTable::Workouts {
                    statements.push_str(&format!(
                        "DELETE sets WHERE workout_id = ${id_binding} RETURN NONE;\n"
                    ));
                }
                statements.push_str(&format!(
                    "DELETE ${record_binding} RETURN NONE;\n\
                     IF record::exists(${record_binding}) {{ THROW ${error_binding}; }};\n"
                ));
                if change.table == FitnessTable::Workouts {
                    statements.push_str(&format!(
                        "IF array::len(SELECT VALUE id FROM sets WHERE workout_id = ${id_binding}) > 0 {{\n\
                             THROW ${error_binding};\n\
                         }};\n"
                    ));
                }
            }
        }
    }
    if bumps_version {
        statements.push_str(
            "UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1 RETURN NONE;\n",
        );
    }
    statements.push_str("COMMIT TRANSACTION;");
    Ok(BuiltMutation {
        sql: statements,
        bindings,
        bumps_version,
    })
}

async fn apply_changes(
    db: &Db,
    request: &ApplyChangesRequest,
    subject_hash: &str,
) -> Result<Value, String> {
    let built = build_mutation(request)?;
    let mut query = db.query(built.sql);
    for (name, value) in built.bindings {
        query = query.bind((name, SerdeWrapper(value)));
    }
    tokio::time::timeout(QUERY_TIMEOUT, query)
        .await
        .map_err(|_| {
            "fitness mutation timed out; check the current version before retrying".to_string()
        })?
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;

    let mut response = db
        .query("SELECT VALUE v FROM fitness_meta:version;")
        .await
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| error.to_string())?;
    let versions: Vec<i64> = response.take(0).map_err(|error| error.to_string())?;
    let version = versions.into_iter().next().unwrap_or(0);
    let mut audit: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for change in &request.changes {
        *audit
            .entry(change.table.spec().name)
            .or_default()
            .entry(match change.action {
                ChangeAction::Create => "create",
                ChangeAction::Replace => "replace",
                ChangeAction::Merge => "merge",
                ChangeAction::Upsert => "upsert",
                ChangeAction::Delete => "delete",
            })
            .or_default() += 1;
    }
    info!(
        subject = %&subject_hash[..16],
        version,
        changes = ?audit,
        "fitness MCP mutation committed"
    );
    Ok(json!({
        "committed": true,
        "reason": request.reason,
        "change_count": request.changes.len(),
        "lifting_version_bumped": built.bumps_version,
        "fitness_version": version,
        "changes": request.changes.iter().map(|change| json!({
            "action": change.action,
            "table": change.table.spec().name,
            "id": change.id,
        })).collect::<Vec<_>>(),
    }))
}

#[derive(Clone)]
struct Runtime {
    config: Arc<ServiceConfig>,
    oauth: OAuthVerifier,
    data: Data,
    permits: Arc<Semaphore>,
}

#[derive(Clone)]
struct FitnessMcp {
    runtime: Arc<Runtime>,
    tool_router: ToolRouter<Self>,
}

impl FitnessMcp {
    fn new(runtime: Arc<Runtime>) -> Self {
        let mut tool_router = Self::tool_router();
        for (name, route) in &mut tool_router.map {
            let scopes = if name.as_ref() == "fitness_apply_changes" {
                vec![READ_SCOPE, WRITE_SCOPE]
            } else {
                vec![READ_SCOPE]
            };
            route.attr.meta = Some(MetaObject(object!({
                "securitySchemes": [{
                    "type": "oauth2",
                    "scopes": scopes,
                }]
            })));
        }
        Self {
            runtime,
            tool_router,
        }
    }

    async fn authorize(
        &self,
        context: &RequestContext<RoleServer>,
        required_scopes: &[&str],
    ) -> Result<VerifiedUser, CallToolResult> {
        let token = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.headers.get(header::AUTHORIZATION))
            .and_then(|value| value.to_str().ok())
            .and_then(bearer_token);
        let Some(token) = token else {
            return Err(self.auth_error(
                "invalid_token",
                "Sign in to access the private fitness log",
                required_scopes,
            ));
        };
        let user = match self.runtime.oauth.verify(token).await {
            Ok(user) => user,
            Err(_) => {
                return Err(self.auth_error(
                    "invalid_token",
                    "The fitness authorization is missing or expired",
                    required_scopes,
                ));
            }
        };
        if let Some(scope) = required_scopes.iter().find(|scope| !user.has_scope(scope)) {
            return Err(self.auth_error(
                "insufficient_scope",
                &format!("Reconnect with the {scope} permission"),
                required_scopes,
            ));
        }
        Ok(user)
    }

    fn auth_error(&self, error: &str, description: &str, scopes: &[&str]) -> CallToolResult {
        let challenge = oauth_challenge(
            &self.runtime.config.metadata_url,
            error,
            description,
            scopes,
        );
        CallToolResult::error(vec![ContentBlock::text(description.to_string())]).with_meta(Some(
            MetaObject(object!({
                "mcp/www_authenticate": [challenge]
            })),
        ))
    }
}

#[tool_router]
impl FitnessMcp {
    /// Describe every accessible table, field, unit, and mutation invariant.
    /// Call this before constructing queries or writes.
    #[tool(
        name = "fitness_catalog",
        annotations(
            title = "Inspect fitness schema",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn fitness_catalog(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let _user = match self.authorize(&context, &[READ_SCOPE]).await {
            Ok(user) => user,
            Err(result) => return Ok(result),
        };
        Ok(CallToolResult::structured(catalog()))
    }

    /// Read one fitness table with bound filters, deterministic ordering, and
    /// offset pagination. Page through next_offset and do complex joins or
    /// calculations in client-side code. The maximum page is 500 records.
    #[tool(
        name = "fitness_read_records",
        annotations(
            title = "Read fitness records",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn fitness_read_records(
        &self,
        Parameters(request): Parameters<ReadRecordsRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let _user = match self.authorize(&context, &[READ_SCOPE]).await {
            Ok(user) => user,
            Err(result) => return Ok(result),
        };
        let _permit = match self.runtime.permits.acquire().await {
            Ok(permit) => permit,
            Err(_) => return Ok(tool_error("fitness service is shutting down")),
        };
        let db = match self.runtime.data.db().await {
            Ok(db) => db,
            Err(error) => return Ok(tool_error(format!("fitness store unavailable: {error}"))),
        };
        match read_records(&db, &request).await {
            Ok(result) => Ok(CallToolResult::structured(result)),
            Err(error) => Ok(tool_error(error)),
        }
    }

    /// Atomically create, replace, merge, upsert, or delete fitness records.
    /// The batch must be user-confirmed. Lifting changes require the version
    /// just read from fitness_meta:version and bump it once. Deleting a workout
    /// cascades only to its sets, preserving taxonomy and muscle corrections.
    #[tool(
        name = "fitness_apply_changes",
        annotations(
            title = "Change fitness records",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn fitness_apply_changes(
        &self,
        Parameters(request): Parameters<ApplyChangesRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let user = match self.authorize(&context, &[READ_SCOPE, WRITE_SCOPE]).await {
            Ok(user) => user,
            Err(result) => return Ok(result),
        };
        if let Err(error) = build_mutation(&request) {
            return Ok(tool_error(error));
        }
        let _permit = match self.runtime.permits.acquire().await {
            Ok(permit) => permit,
            Err(_) => return Ok(tool_error("fitness service is shutting down")),
        };
        let db = match self.runtime.data.db().await {
            Ok(db) => db,
            Err(error) => return Ok(tool_error(format!("fitness store unavailable: {error}"))),
        };
        match apply_changes(&db, &request, &user.subject_hash).await {
            Ok(result) => Ok(CallToolResult::structured(result)),
            Err(error) => Ok(tool_error(format!("fitness change rejected: {error}"))),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FitnessMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "Private fitness log. Call fitness_catalog first. Use fitness_read_records with pagination and perform complex joins/analysis in client-side code. Before any write, read fitness_meta:version, show the user the exact intended changes, and call fitness_apply_changes only after confirmation. Never invent unit conversions or stored records; records are derived from sets. Raw SurrealQL is intentionally unavailable."
                    .to_string(),
            )
    }
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

fn oauth_challenge(metadata_url: &str, error: &str, description: &str, scopes: &[&str]) -> String {
    let clean_description = description.replace(['"', '\r', '\n'], " ");
    format!(
        "Bearer resource_metadata=\"{metadata_url}\", error=\"{error}\", error_description=\"{clean_description}\", scope=\"{}\"",
        scopes.join(" ")
    )
}

#[derive(Clone)]
struct HttpState {
    config: Arc<ServiceConfig>,
    oauth: OAuthVerifier,
}

async fn protected_resource_metadata(State(state): State<Arc<HttpState>>) -> Json<Value> {
    Json(json!({
        "resource": state.config.resource_url,
        "authorization_servers": [state.config.oauth_issuer],
        "scopes_supported": [READ_SCOPE, WRITE_SCOPE],
        "bearer_methods_supported": ["header"],
    }))
}

async fn oauth_guard(
    State(state): State<Arc<HttpState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token);
    if let Some(token) = token {
        match state.oauth.verify(token).await {
            Ok(_) => return next.run(request).await,
            Err(error) => warn!(reason = error, "fitness MCP OAuth token rejected"),
        }
    }
    let challenge = oauth_challenge(
        &state.config.metadata_url,
        "invalid_token",
        "Sign in to access the private fitness log",
        &[READ_SCOPE, WRITE_SCOPE],
    );
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "invalid_token",
            "error_description": "Sign in to access the private fitness log",
        })),
    )
        .into_response();
    if let Ok(value) = header::HeaderValue::from_str(&challenge) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

async fn no_store(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, no-store"),
    );
    response
}

fn http_router(config: ServiceConfig, data: Data) -> anyhow::Result<Router> {
    let config = Arc::new(config);
    let oauth = OAuthVerifier::new(&config)?;
    let runtime = Arc::new(Runtime {
        config: config.clone(),
        oauth: oauth.clone(),
        data,
        permits: Arc::new(Semaphore::new(4)),
    });
    let http_state = Arc::new(HttpState {
        config: config.clone(),
        oauth,
    });
    let factory_runtime = runtime.clone();
    let transport_config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(config.allowed_hosts.clone())
        .with_allowed_origins(config.allowed_origins.clone())
        .with_json_response(true)
        .with_max_request_body_bytes(512 * 1024);
    let mcp_service: StreamableHttpService<FitnessMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(FitnessMcp::new(factory_runtime.clone())),
            LocalSessionManager::default().into(),
            transport_config,
        );
    let protected =
        Router::new()
            .nest_service(MCP_PATH, mcp_service)
            .layer(middleware::from_fn_with_state(
                http_state.clone(),
                oauth_guard,
            ));
    Ok(Router::new()
        .route(METADATA_PATH, get(protected_resource_metadata))
        // Some clients probe the origin-wide form before the path-specific
        // RFC 9728 URL. Both documents describe the same protected resource.
        .route(FALLBACK_METADATA_PATH, get(protected_resource_metadata))
        .merge(protected)
        .with_state(http_state)
        .layer(middleware::from_fn(no_store)))
}

/// Mount the optional OAuth-protected MCP endpoint into the site's Topcoat
/// router. Setting neither OAuth variable leaves the endpoint absent; setting
/// only one is a startup error so a partially protected endpoint cannot ship.
#[must_use]
pub fn mount(builder: RouterBuilder, data: Data) -> RouterBuilder {
    let config = ServiceConfig::from_env()
        .unwrap_or_else(|error| panic!("invalid fitness MCP configuration: {error}"));
    let Some(config) = config else {
        return builder;
    };
    let resource_url = config.resource_url.clone();
    let app = http_router(config, data)
        .unwrap_or_else(|error| panic!("failed to initialize fitness MCP: {error}"));
    info!(resource = %resource_url, "fitness MCP mounted");
    mount_http(builder, app)
}

fn mount_http(builder: RouterBuilder, app: Router) -> RouterBuilder {
    builder
        .route(TowerRoute::new(Methods::Any, MCP_PATH, app.clone()))
        .route(TowerRoute::new(Method::GET, METADATA_PATH, app.clone()))
        .route(TowerRoute::new(Method::GET, FALLBACK_METADATA_PATH, app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::engine::any;
    use topcoat::router::{
        Body as TopcoatBody, OriginPolicy, Router as TopcoatRouter, request::Request, to_bytes,
    };

    fn read_request() -> ReadRecordsRequest {
        ReadRecordsRequest {
            table: FitnessTable::Sets,
            fields: Some(vec!["exercise_name".to_string(), "reps".to_string()]),
            filters: vec![RecordFilter {
                field: "exercise_name".to_string(),
                operator: FilterOperator::Eq,
                value: Some(json!("Bench'; DELETE workouts; --")),
            }],
            match_any: false,
            order_by: vec![RecordSort {
                field: "ordinal".to_string(),
                direction: SortDirection::Asc,
            }],
            limit: Some(25),
            offset: Some(50),
        }
    }

    fn change(
        action: ChangeAction,
        table: FitnessTable,
        id: &str,
        data: Option<Value>,
    ) -> RecordChange {
        RecordChange {
            action,
            table,
            id: id.to_string(),
            data: data.map(|value| value.as_object().unwrap().clone()),
        }
    }

    fn service_config() -> ServiceConfig {
        ServiceConfig {
            resource_url: "https://ben.soy/mcp".to_string(),
            metadata_url: "https://ben.soy/.well-known/oauth-protected-resource/mcp".to_string(),
            oauth_issuer: "https://example.us.auth0.com/".to_string(),
            oauth_audience: "https://ben.soy/mcp".to_string(),
            allowed_subjects: HashSet::from(["google-oauth2|owner".to_string()]),
            allowed_hosts: vec!["ben.soy".to_string()],
            allowed_origins: vec!["https://chatgpt.com".to_string()],
        }
    }

    fn mounted_router() -> TopcoatRouter {
        let app = http_router(service_config(), Data::new(Err("SURREALDB_ENDPOINT"))).unwrap();
        mount_http(
            TopcoatRouter::builder().origin_policy(OriginPolicy::new().exempt_paths([MCP_PATH])),
            app,
        )
        .build()
    }

    #[tokio::test]
    async fn mounted_endpoint_challenges_and_publishes_resource_metadata() {
        let router = mounted_router();
        let request = Request::builder()
            .method(Method::POST)
            .uri(MCP_PATH)
            .header(header::HOST, "ben.soy")
            .header(header::ORIGIN, "https://chatgpt.com")
            .body(TopcoatBody::from("{}"))
            .unwrap();
        let response = router.handle(request).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        assert!(
            response.headers()[header::WWW_AUTHENTICATE]
                .to_str()
                .unwrap()
                .contains(METADATA_PATH)
        );

        let request = Request::builder()
            .uri(METADATA_PATH)
            .header(header::HOST, "ben.soy")
            .body(TopcoatBody::empty())
            .unwrap();
        let response = router.handle(request).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(body["resource"], "https://ben.soy/mcp");
        assert_eq!(
            body["authorization_servers"],
            json!(["https://example.us.auth0.com/"])
        );
        assert_eq!(body["scopes_supported"], json!([READ_SCOPE, WRITE_SCOPE]));
    }

    #[test]
    fn read_builder_binds_values_and_whitelists_identifiers() {
        let built = build_read_query(&read_request()).unwrap();
        assert!(built.sql.contains("FROM sets"));
        assert!(built.sql.contains("exercise_name = $filter_0"));
        assert!(!built.sql.contains("DELETE workouts"));
        assert_eq!(
            built.bindings["filter_0"],
            json!("Bench'; DELETE workouts; --")
        );
        assert_eq!(built.limit, 25);
        assert_eq!(built.offset, 50);

        let mut request = read_request();
        request.filters[0].field = "exercise_name; DELETE workouts".to_string();
        assert!(
            build_read_query(&request)
                .unwrap_err()
                .contains("unknown field")
        );
    }

    #[test]
    fn mutation_builder_requires_version_and_never_interpolates_data() {
        let mut request = ApplyChangesRequest {
            confirmed: true,
            reason: "correct an exercise name".to_string(),
            expected_version: None,
            changes: vec![change(
                ChangeAction::Create,
                FitnessTable::Exercises,
                "probe",
                Some(json!({"name": "Curl'; DELETE sets; --"})),
            )],
        };
        assert!(
            build_mutation(&request)
                .unwrap_err()
                .contains("expected_version")
        );
        request.expected_version = Some(7);
        let built = build_mutation(&request).unwrap();
        assert!(built.bumps_version);
        assert!(!built.sql.contains("DELETE sets; --"));
        assert_eq!(
            built.bindings["data_0"],
            json!({"name": "Curl'; DELETE sets; --"})
        );
    }

    async fn mutation_db() -> Db {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns("fitness_mcp_test")
            .use_db("fitness_mcp_test")
            .await
            .unwrap();
        db.query(
            "DEFINE TABLE exercises SCHEMALESS PERMISSIONS FULL;
             DEFINE TABLE workouts SCHEMALESS PERMISSIONS FULL;
             DEFINE TABLE sets SCHEMALESS PERMISSIONS FULL;
             DEFINE TABLE fitness_meta SCHEMALESS PERMISSIONS FULL;",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
        db
    }

    #[tokio::test]
    async fn atomic_changes_bump_once_and_reads_round_trip_json() {
        let db = mutation_db().await;
        let request = ApplyChangesRequest {
            confirmed: true,
            reason: "add test exercise".to_string(),
            expected_version: Some(0),
            changes: vec![change(
                ChangeAction::Create,
                FitnessTable::Exercises,
                "test-exercise",
                Some(json!({"name": "Test Exercise"})),
            )],
        };
        let receipt = apply_changes(&db, &request, &"a".repeat(64)).await.unwrap();
        assert_eq!(receipt["fitness_version"], 1);
        let page = read_records(
            &db,
            &ReadRecordsRequest {
                table: FitnessTable::Exercises,
                fields: None,
                filters: Vec::new(),
                match_any: false,
                order_by: Vec::new(),
                limit: Some(10),
                offset: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(page["records"][0]["id"], "test-exercise");
        assert_eq!(page["records"][0]["name"], "Test Exercise");
    }

    #[tokio::test]
    async fn deleting_a_workout_cascades_sets_only() {
        let db = mutation_db().await;
        let create = ApplyChangesRequest {
            confirmed: true,
            reason: "seed cascade test".to_string(),
            expected_version: Some(0),
            changes: vec![
                change(
                    ChangeAction::Create,
                    FitnessTable::Workouts,
                    "workout-1",
                    Some(json!({"title": "Test"})),
                ),
                change(
                    ChangeAction::Create,
                    FitnessTable::Sets,
                    "set-1",
                    Some(json!({"workout_id": "workout-1"})),
                ),
                change(
                    ChangeAction::Create,
                    FitnessTable::Exercises,
                    "survivor",
                    Some(json!({"name": "Survivor"})),
                ),
            ],
        };
        apply_changes(&db, &create, &"b".repeat(64)).await.unwrap();
        let delete = ApplyChangesRequest {
            confirmed: true,
            reason: "remove cascade test".to_string(),
            expected_version: Some(1),
            changes: vec![change(
                ChangeAction::Delete,
                FitnessTable::Workouts,
                "workout-1",
                None,
            )],
        };
        let receipt = apply_changes(&db, &delete, &"b".repeat(64)).await.unwrap();
        assert_eq!(receipt["fitness_version"], 2);

        let mut response = db
            .query(
                "RETURN {
                    workouts: array::len(SELECT * FROM workouts),
                    sets: array::len(SELECT * FROM sets),
                    exercises: array::len(SELECT * FROM exercises)
                };",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let counts: Option<SerdeWrapper<Value>> = response.take(0).unwrap();
        assert_eq!(
            counts.unwrap().0,
            json!({"workouts": 0, "sets": 0, "exercises": 1})
        );
    }
}
