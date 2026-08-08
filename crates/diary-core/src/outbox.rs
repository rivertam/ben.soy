//! The device-local diary store: mirror and outbox in ONE table.
//!
//! On the phone this runs against `indxdb://` (SurrealDB's IndexedDB engine)
//! inside the service worker and the page; under `cargo test` it runs
//! against `mem://`. Nothing here knows which: every function takes the same
//! `Surreal<Any>` handle the site's server code uses, which is the point —
//! the logic is written once and exercised natively before it ships to wasm.
//!
//! The local `diary_entries` table holds the canonical Diary Entry fields
//! plus local-only sync state: a queued entry is just a row with
//! `state = 'pending'`, a delivered one flips to `'synced'` in place, and a
//! permanently rejected one to `'failed'`. Ids are predicted locally with
//! the SAME probe-and-dedupe loop the server runs ([`crate::store`]), so an
//! entry gets a predicted Entry Key from the moment it is enqueued; a Saved
//! Reference reconciles the rare server-side collision bump. Rows are never
//! deleted on delivery — that delete-before-report gap is what forced the
//! old page to keep a provisional-bubble machine.
//!
//! Deliberately no `Send` bounds on [`flush`]'s transport: on wasm the
//! injected future wraps a browser `fetch` and is `!Send`; natively the test
//! doubles are ordinary futures. Single-threaded wasm never needs `Send`,
//! and adding it would make the shared signature uncompilable there.
//!
//! Pull reconciliation lives here too: [`reconcile`] makes the local mirror
//! agree with one remote snapshot without ever overwriting pending or failed
//! text. The sync module only sequences this store with its remote.
//!
//! The query shapes follow docs/surrealdb-notes.md: explicit projections
//! (because `SELECT *` omits `option` fields holding `NONE`), string keys
//! returned via `record::id(id)`, and one `=` per delete.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use surrealdb::{
    engine::any,
    types::{SurrealValue, Value},
};

use crate::contract::{CURRENT_WIRE_VERSION, SendOutcome};
use crate::entry::{ComposedEntry, DiaryEntry, PROJECTION, SavedRef, prepare_for_queue};
use crate::placement::{self, Occupant, Placement as EntryPlacement};
use crate::store::entry_key;

pub use crate::Db;

/// Local namespace/database names. Nothing else ever lives in this store.
const NAMESPACE: &str = "diary";
const DATABASE: &str = "diary";
/// Rows created before entry generations were persisted are generation 1.
/// This must stay frozen when newer business fields bump the current wire.
const LEGACY_ENTRY_VERSION: i64 = 1;

pub const STATE_SYNCED: &str = "synced";
pub const STATE_PENDING: &str = "pending";
pub const STATE_FAILED: &str = "failed";

/// The local schema, reconciled by [`open`] the way `src/data.rs` reconciles
/// the committed server schema. Two deliberate absences: no length ASSERT on
/// `body` (the queue must hold whatever text is already on the device — the
/// server is the judge, and its rejection marks the entry failed instead of
/// stranding it), and no id-shape ASSERT (a row whose `written_at` cannot
/// project to a real key is still never dropped — it ports under a synthetic
/// key as failed).
const SCHEMA: &str = "\
    DEFINE TABLE OVERWRITE diary_entries SCHEMAFULL PERMISSIONS NONE;\n\
    DEFINE FIELD OVERWRITE written_at ON diary_entries TYPE int;\n\
    DEFINE FIELD OVERWRITE body ON diary_entries TYPE string;\n\
    DEFINE FIELD OVERWRITE entry_version ON diary_entries TYPE option<int> \
        ASSERT $value = NONE OR $value >= 1;\n\
    DEFINE FIELD OVERWRITE state ON diary_entries TYPE string \
        ASSERT $value IN ['synced', 'pending', 'failed'];\n\
    DEFINE FIELD OVERWRITE reason ON diary_entries TYPE option<string>;\n\
    DEFINE FIELD OVERWRITE enqueued_at ON diary_entries TYPE int;\n\
    DEFINE FIELD OVERWRITE write_fingerprint ON diary_entries TYPE option<string> \
        ASSERT $value = NONE OR string::matches($value, '^[0-9a-f]{64}$');\n";

/// The pre-single-store queue table (v1). Still DEFINEd on every open so the
/// standing drain below can always SELECT it: during a deploy's version skew
/// an old page or worker happily re-creates and writes `diary_outbox` after
/// the new code emptied it, so the drain is a permanent cheap step (exactly
/// like the pre-wasm IndexedDB migration before it), never a one-shot.
const V1_SCHEMA: &str = "\
    DEFINE TABLE OVERWRITE diary_outbox SCHEMAFULL PERMISSIONS NONE;\n\
    DEFINE FIELD OVERWRITE written_at ON diary_outbox TYPE int;\n\
    DEFINE FIELD OVERWRITE body ON diary_outbox TYPE string;\n\
    DEFINE FIELD OVERWRITE state ON diary_outbox TYPE string \
        ASSERT $value IN ['pending', 'failed'];\n\
    DEFINE FIELD OVERWRITE reason ON diary_outbox TYPE option<string>;\n\
    DEFINE FIELD OVERWRITE enqueued_at ON diary_outbox TYPE int;\n";

#[derive(Debug)]
pub enum OutboxError {
    /// Empty after normalization — nothing to queue. This is the ONLY
    /// validation applied to new text; anything non-empty is queued and the
    /// server judges it on replay, so a rejection marks the entry failed
    /// with its text preserved instead of bouncing it into the lossy
    /// form-POST fallback.
    InvalidBody,
    Db(String),
}

impl std::fmt::Display for OutboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutboxError::InvalidBody => write!(f, "not an entry we would store"),
            OutboxError::Db(error) => write!(f, "local store failed: {error}"),
        }
    }
}

impl std::error::Error for OutboxError {}

/// One local row. `id` is the entry's predicted Entry Key (or a
/// synthetic key for unprojectable failed rows); `enqueued_at`
/// (caller-supplied milliseconds) orders the flush.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
pub struct LocalEntry {
    #[serde(flatten)]
    #[surreal(flatten)]
    pub entry: DiaryEntry,
    pub state: String,
    pub reason: Option<String>,
    pub enqueued_at: i64,
}

/// The store-only shape used by a flush. `LocalEntry` deliberately does not
/// expose this implementation token in its Rust or JSON interface, while the
/// flusher must carry the exact fingerprint written by the worker generation
/// that enqueued the row. Recomputing it after a canonical field is added
/// would strand older pending writes whose serialized shape lacked that field.
#[derive(Clone, Debug, Deserialize, SurrealValue)]
struct QueuedWrite {
    #[serde(flatten)]
    #[surreal(flatten)]
    entry: LocalEntry,
    write_fingerprint: String,
}

impl Deref for LocalEntry {
    type Target = DiaryEntry;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

impl DerefMut for LocalEntry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entry
    }
}

/// The record value stored under a SurrealDB record id. Keeping the id out
/// of the payload lets `CONTENT $row` replace every mutable field without
/// trying to replace SurrealDB's read-only `id` field. In particular,
/// `Option::None` becomes SurrealQL `NONE` through the pinned SDK's
/// `SurrealValue` conversion, so optional fields need no query-shape branch.
#[derive(Clone, Debug, SurrealValue)]
struct LocalRow {
    #[surreal(flatten)]
    composed: ComposedEntry,
    entry_version: i64,
    state: String,
    reason: Option<String>,
    enqueued_at: i64,
    write_fingerprint: String,
}

impl LocalRow {
    fn new(
        composed: ComposedEntry,
        state: impl Into<String>,
        reason: Option<String>,
        enqueued_at: i64,
    ) -> Self {
        let entry_version = i64::from(CURRENT_WIRE_VERSION);
        let write_fingerprint = write_fingerprint(&composed, enqueued_at, entry_version);
        Self {
            composed,
            entry_version,
            state: state.into(),
            reason,
            enqueued_at,
            write_fingerprint,
        }
    }
}

impl LocalEntry {
    fn row_with(&self, state: &str, reason: Option<String>) -> LocalRow {
        self.row_with_composed(self.composed.clone(), state, reason)
    }

    fn row_with_composed(
        &self,
        composed: ComposedEntry,
        state: &str,
        reason: Option<String>,
    ) -> LocalRow {
        LocalRow::new(composed, state, reason, self.enqueued_at)
    }
}

/// An immutable token for one queued write. Its semantic generation and
/// canonical entry are serialized as a whole, so future business fields
/// automatically become part of the compare-and-swap identity without
/// changing any SurrealQL predicate. Enqueue order distinguishes a discarded
/// write from a later, otherwise identical replacement.
fn write_fingerprint(composed: &ComposedEntry, enqueued_at: i64, entry_version: i64) -> String {
    let bytes = serde_json::to_vec(&(entry_version, composed, enqueued_at))
        .expect("canonical diary entries always serialize");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut fingerprint = String::with_capacity(digest.len() * 2);
    for byte in digest {
        fingerprint.push(char::from(HEX[usize::from(byte >> 4)]));
        fingerprint.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    fingerprint
}

/// A queue entry arriving from the pre-wasm IndexedDB store (the worker
/// reads them out once and passes them to [`import`]). Unknown fields — the
/// old store's `qid` — are ignored, and bodies are preserved byte-for-byte:
/// the server is the only judge of old text.
#[derive(Debug, Deserialize)]
pub struct LegacyEntry {
    pub written_at: i64,
    pub body: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub enqueued_at: Option<i64>,
}

/// What one flush did, in the shape the page's BroadcastChannel message has
/// always carried (`blocked` serializes to `null` / `"auth"` / `"net"`).
/// `saved_refs` maps each delivered entry's local id (`qid`, its
/// predicted key) to the identity the server actually assigned — identical
/// in the common case, different only when the server's cross-device
/// collision probe bumped the second.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FlushReport {
    pub saved: u32,
    pub pending: u32,
    pub failed: u32,
    pub blocked: Option<Blocked>,
    pub saved_refs: Vec<SavedRefMapping>,
    /// How many mirror rows the pull that follows a flush changed — `None`
    /// when the pull was skipped or classified Auth/Retry (a no-op). Filled
    /// by `sync::run`, not by [`flush`] itself; stale pages ignore it.
    pub pulled: Option<u32>,
}

/// One entry a flush landed: the local id it was queued under and the
/// permanent identity the server holds it under now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SavedRefMapping {
    pub qid: String,
    #[serde(flatten)]
    pub saved: SavedRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Blocked {
    Auth,
    Net,
}

/// Connect to an endpoint (`indxdb://diary` on the device, `mem://` in
/// tests), select the fixed namespace, reconcile both schemas, and drain any
/// v1 queue rows into the single store. Local engines need no signin.
/// Concurrent opens (the page and the worker both call this) are safe: the
/// drain's placement loop is check-CREATE-recheck, so a lost race reads as
/// the twin it is.
pub async fn open(endpoint: &str) -> Result<Db, OutboxError> {
    let db = any::connect(endpoint).await.map_err(db_error)?;
    db.use_ns(NAMESPACE)
        .use_db(DATABASE)
        .await
        .map_err(db_error)?;
    initialize(&db).await?;
    Ok(db)
}

/// Reconcile one already-selected local database. Kept separate from
/// [`open`] so the upgrade path can be exercised against a store populated
/// by the preceding single-table worker generation.
async fn initialize(db: &Db) -> Result<(), OutboxError> {
    let mut last_error = None;
    for _ in 0..3 {
        match initialize_once(db).await {
            Err(OutboxError::Db(message)) if message.contains("Resource busy") => {
                last_error = Some(OutboxError::Db(message));
            }
            other => return other,
        }
    }
    Err(last_error.expect("loop only exits early or stores an error"))
}

/// Every initialization step is idempotent. Retrying the whole sequence is
/// therefore the safest response when the page and service worker race on a
/// schema or fingerprint write: the winner's standing migration is observed
/// on the next SELECT, and no special partially-initialized state exists.
async fn initialize_once(db: &Db) -> Result<(), OutboxError> {
    db.query(SCHEMA)
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    backfill_row_metadata_once(db).await?;
    db.query(V1_SCHEMA)
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    drain_v1(db).await?;
    Ok(())
}

/// A cached current DB handle can outlive an older page context that writes
/// the preceding row shape. Re-run the cheap guarded migrations immediately
/// before every flush selection, with the same bounded conflict handling as
/// initialization, so such a row never waits for a worker restart.
async fn backfill_row_metadata(db: &Db) -> Result<(), OutboxError> {
    let mut last_error = None;
    for _ in 0..3 {
        match backfill_row_metadata_once(db).await {
            Err(OutboxError::Db(message)) if message.contains("Resource busy") => {
                last_error = Some(OutboxError::Db(message));
            }
            other => return other,
        }
    }
    Err(last_error.expect("loop only exits early or stores an error"))
}

async fn backfill_row_metadata_once(db: &Db) -> Result<(), OutboxError> {
    backfill_entry_versions(db).await?;
    backfill_write_fingerprints(db).await
}

/// The single-table generation immediately before this infrastructure had
/// no semantic-generation column. Its body-only values are generation 1.
/// Keep this standing and conditional so an older active page cannot make a
/// permanently invisible row during rollout.
async fn backfill_entry_versions(db: &Db) -> Result<(), OutboxError> {
    db.query(
        "UPDATE diary_entries SET entry_version = $version \
         WHERE entry_version = NONE",
    )
    .bind(("version", LEGACY_ENTRY_VERSION))
    .await
    .map_err(db_error)?
    .check()
    .map_err(db_error)?;
    Ok(())
}

/// The immediately preceding single-store generation has every canonical
/// entry field and enqueue order, but no CAS token. Hash those whole values
/// before flushing. The guarded update is a standing migration: concurrent
/// current workers calculate the same token, and one can never overwrite a
/// fingerprint already supplied by another current writer.
async fn backfill_write_fingerprints(db: &Db) -> Result<(), OutboxError> {
    #[derive(Deserialize, SurrealValue)]
    struct UnfingerprintedRow {
        #[serde(flatten)]
        #[surreal(flatten)]
        entry: DiaryEntry,
        enqueued_at: i64,
        entry_version: i64,
    }

    let mut response = db
        .query(format!(
            "SELECT {PROJECTION}, enqueued_at, entry_version FROM diary_entries \
             WHERE write_fingerprint = NONE \
                 AND entry_version = {LEGACY_ENTRY_VERSION} \
             ORDER BY written_at ASC, id ASC"
        ))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let rows: Vec<UnfingerprintedRow> = response.take(0).map_err(db_error)?;
    for row in rows {
        let fingerprint =
            write_fingerprint(&row.entry.composed, row.enqueued_at, row.entry_version);
        install_legacy_fingerprint(
            db,
            &row.entry,
            row.enqueued_at,
            row.entry_version,
            fingerprint,
        )
        .await?;
    }
    Ok(())
}

/// Install the token only if the frozen generation-1 value selected for the
/// hash still occupies this id. A discard/recreate between SELECT and UPDATE
/// stays tokenless and is picked up from fresh content on the next pass.
async fn install_legacy_fingerprint(
    db: &Db,
    entry: &DiaryEntry,
    enqueued_at: i64,
    entry_version: i64,
    fingerprint: String,
) -> Result<(), OutboxError> {
    db.query(
        "UPDATE type::record('diary_entries', $id) \
         SET write_fingerprint = $fingerprint \
         WHERE write_fingerprint = NONE \
             AND entry_version = $entry_version \
             AND written_at = $written_at \
             AND body = $body \
             AND enqueued_at = $enqueued_at",
    )
    .bind(("id", entry.id.clone()))
    .bind(("fingerprint", fingerprint))
    .bind(("entry_version", entry_version))
    .bind(("written_at", entry.written_at))
    .bind(("body", entry.body.clone()))
    .bind(("enqueued_at", enqueued_at))
    .await
    .map_err(db_error)?
    .check()
    .map_err(db_error)?;
    Ok(())
}

/// Queue one entry composed at `written_at` (epoch seconds). Line endings
/// are normalized here so what sits locally is byte-identical to what the
/// server would store — but the length bound deliberately is NOT checked:
/// over-length text queues fine, replays, gets the server's 422, and stays
/// on the page as a failed entry for manual copy. `enqueued_at_ms` comes
/// from the caller because the caller owns the clock (the page passes
/// `Date.now()`).
///
/// The id is resolved HERE, with the same probe-and-dedupe rule the server
/// applies: same second + same Entry Content at any probed key is the same
/// entry (returned as-is — a double-tap converges instead of minting a
/// twin); only different content probes forward. The resulting key is the
/// device prediction, and the server's own probe is the cross-device
/// backstop that may re-anchor it.
pub async fn enqueue(
    db: &Db,
    entry: ComposedEntry,
    enqueued_at_ms: i64,
) -> Result<LocalEntry, OutboxError> {
    let entry = prepare_for_queue(entry).map_err(|_| OutboxError::InvalidBody)?;
    place_local(db, &entry, STATE_PENDING, None, enqueued_at_ms).await
}

/// Every understood not-yet-synced row, oldest enqueue first — flush order
/// and the page's bubble order. Newer generations remain opaque; synced rows
/// are absent because the server (or offline render) already shows them.
pub async fn queued(db: &Db) -> Result<Vec<LocalEntry>, OutboxError> {
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION}, state, reason, enqueued_at \
             FROM diary_entries \
             WHERE state != 'synced' \
                 AND (entry_version = NONE \
                     OR entry_version <= {CURRENT_WIRE_VERSION}) \
             ORDER BY enqueued_at ASC, id ASC"
        ))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    response.take(0).map_err(db_error)
}

async fn queued_writes(db: &Db) -> Result<Vec<QueuedWrite>, OutboxError> {
    backfill_row_metadata(db).await?;
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION}, state, reason, enqueued_at, write_fingerprint \
             FROM diary_entries \
             WHERE state != 'synced' AND entry_version <= {CURRENT_WIRE_VERSION} \
             ORDER BY enqueued_at ASC, id ASC"
        ))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    response.take(0).map_err(db_error)
}

/// Every understood local row in every state, oldest write first — the
/// worker's offline SSR read and the content-bearing side of [`reconcile`].
pub async fn all_local(db: &Db) -> Result<Vec<LocalEntry>, OutboxError> {
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION}, state, reason, enqueued_at \
             FROM diary_entries \
             WHERE entry_version = NONE \
                 OR entry_version <= {CURRENT_WIRE_VERSION} \
             ORDER BY written_at ASC, id ASC"
        ))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    response.take(0).map_err(db_error)
}

/// One understood local row by id, any state — the worker's offline
/// permalink read. A newer generation is intentionally indistinguishable
/// from an absent page value here, while mutation paths track it as opaque.
pub async fn entry(db: &Db, id: &str) -> Result<Option<LocalEntry>, OutboxError> {
    local_by_id(db, id).await
}

/// Make the local mirror agree with a remote snapshot: create missing rows
/// as synced, update synced rows the server changed, and delete synced rows
/// the server no longer has. Pending and failed rows always win at a
/// matching id; their own flush is the only operation allowed to change
/// them. Same-record write conflicts are retried against a fresh local read.
///
/// An empty snapshot never wipes an already-populated mirror. SurrealDB can
/// answer a permission-denied read as an empty result, so treating that as
/// an authoritative empty diary would destroy the offline copy.
///
/// Returns how many rows changed.
pub async fn reconcile(db: &Db, incoming: &[DiaryEntry]) -> Result<u32, OutboxError> {
    let mut last_error = None;
    for _ in 0..3 {
        match reconcile_once(db, incoming).await {
            Err(OutboxError::Db(message)) if message.contains("Resource busy") => {
                last_error = Some(OutboxError::Db(message));
            }
            other => return other,
        }
    }
    Err(last_error.expect("loop only exits early or stores an error"))
}

async fn reconcile_once(db: &Db, incoming: &[DiaryEntry]) -> Result<u32, OutboxError> {
    let local = all_local(db).await?;
    let opaque_ids = opaque_local_ids(db).await?;
    if incoming.is_empty() && local.iter().any(|row| row.state == STATE_SYNCED) {
        return Ok(0);
    }
    let plan = pull_plan(&local, &opaque_ids, incoming);
    let changes = (plan.creates.len() + plan.updates.len() + plan.deletes.len()) as u32;
    if changes == 0 {
        return Ok(0);
    }

    // One transaction keeps the mirror at one coherent snapshot. Each row
    // payload is a typed object, so adding a persisted entry field changes
    // LocalRow once instead of adding another bind for every statement.
    let mut statements = String::from("BEGIN TRANSACTION;\n");
    let mut id_binds: Vec<(String, String)> = Vec::new();
    let mut row_binds: Vec<(String, LocalRow)> = Vec::new();
    for (index, entry) in plan.creates.iter().enumerate() {
        statements.push_str(&format!(
            "CREATE ONLY type::record('diary_entries', $c{index}_id) \
             CONTENT $c{index}_row;\n"
        ));
        id_binds.push((format!("c{index}_id"), entry.id.clone()));
        row_binds.push((
            format!("c{index}_row"),
            LocalRow::new(entry.composed.clone(), STATE_SYNCED, None, 0),
        ));
    }
    for (index, update) in plan.updates.iter().enumerate() {
        statements.push_str(&format!(
            "UPDATE type::record('diary_entries', $u{index}_id) \
             CONTENT $u{index}_row \
             WHERE state = 'synced' \
                 AND (entry_version = NONE \
                     OR entry_version <= {CURRENT_WIRE_VERSION});\n"
        ));
        id_binds.push((format!("u{index}_id"), update.incoming.id.clone()));
        row_binds.push((
            format!("u{index}_row"),
            LocalRow::new(
                update.incoming.composed.clone(),
                STATE_SYNCED,
                None,
                update.enqueued_at,
            ),
        ));
    }
    for (index, id) in plan.deletes.iter().enumerate() {
        statements.push_str(&format!(
            "DELETE type::record('diary_entries', $d{index}_id) \
             WHERE state = 'synced' \
                 AND (entry_version = NONE \
                     OR entry_version <= {CURRENT_WIRE_VERSION});\n"
        ));
        id_binds.push((format!("d{index}_id"), id.clone()));
    }
    statements.push_str("COMMIT TRANSACTION;");

    let mut query = db.query(statements);
    for (name, value) in id_binds {
        query = query.bind((name, value));
    }
    for (name, value) in row_binds {
        query = query.bind((name, value));
    }
    query.await.map_err(db_error)?.check().map_err(db_error)?;
    Ok(changes)
}

/// Record identities owned by a newer semantic generation. Older code must
/// not deserialize their business fields, but reconciliation still needs to
/// know the keys are occupied so it cannot create over them.
async fn opaque_local_ids(db: &Db) -> Result<HashSet<String>, OutboxError> {
    #[derive(Deserialize, SurrealValue)]
    struct IdRow {
        id: String,
    }

    let mut response = db
        .query(format!(
            "SELECT record::id(id) AS id FROM diary_entries \
             WHERE entry_version > {CURRENT_WIRE_VERSION}"
        ))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let rows: Vec<IdRow> = response.take(0).map_err(db_error)?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

struct PullUpdate<'a> {
    incoming: &'a DiaryEntry,
    enqueued_at: i64,
}

struct PullPlan<'a> {
    creates: Vec<&'a DiaryEntry>,
    updates: Vec<PullUpdate<'a>>,
    deletes: Vec<String>,
}

fn pull_plan<'a>(
    local: &[LocalEntry],
    opaque_ids: &HashSet<String>,
    incoming: &'a [DiaryEntry],
) -> PullPlan<'a> {
    let by_id: HashMap<&str, &LocalEntry> =
        local.iter().map(|row| (row.id.as_str(), row)).collect();
    let mut incoming_ids = HashSet::new();
    let mut creates = Vec::new();
    let mut updates = Vec::new();
    for entry in incoming {
        incoming_ids.insert(entry.id.as_str());
        if opaque_ids.contains(&entry.id) {
            continue;
        }
        match by_id.get(entry.id.as_str()) {
            None => creates.push(entry),
            Some(row) if row.state == STATE_SYNCED => {
                if row.entry != *entry {
                    updates.push(PullUpdate {
                        incoming: entry,
                        enqueued_at: row.enqueued_at,
                    });
                }
            }
            Some(_) => {}
        }
    }
    let deletes = local
        .iter()
        .filter(|row| {
            row.state == STATE_SYNCED
                && !opaque_ids.contains(&row.id)
                && !incoming_ids.contains(row.id.as_str())
        })
        .map(|row| row.id.clone())
        .collect();
    PullPlan {
        creates,
        updates,
        deletes,
    }
}

/// Drop an understood queued or failed row — the page's discard button.
/// Synced and newer-generation rows are out of reach on purpose: this worker
/// cannot safely mutate either. Idempotent.
pub async fn discard(db: &Db, id: &str) -> Result<(), OutboxError> {
    db.query(
        "DELETE type::record('diary_entries', $id) \
         WHERE state != 'synced' \
             AND (entry_version = NONE \
                 OR entry_version <= $current_version)",
    )
    .bind(("id", id.to_string()))
    .bind(("current_version", i64::from(CURRENT_WIRE_VERSION)))
    .await
    .map_err(db_error)?
    .check()
    .map_err(db_error)?;
    Ok(())
}

/// Import the pre-wasm IndexedDB queue. Idempotent by composition second plus
/// Entry Content—the placement loop's dedupe—so the caller deleting old records only
/// after this returns makes a crash between the two re-run safely.
/// Returns how many entries were newly written.
pub async fn import(db: &Db, legacy: &[LegacyEntry]) -> Result<u32, OutboxError> {
    let mut imported = 0;
    for entry in legacy {
        if entry.body.is_empty() {
            continue; // the old page never queued empty text; nothing to keep
        }
        let state = match entry.state.as_deref() {
            Some(STATE_FAILED) => STATE_FAILED,
            _ => STATE_PENDING,
        };
        let placed = place_preserving(
            db,
            &ComposedEntry::new(entry.written_at, entry.body.clone()),
            state,
            entry.reason.as_deref(),
            entry.enqueued_at.unwrap_or(0),
        )
        .await?;
        if placed {
            imported += 1;
        }
    }
    Ok(imported)
}

/// Replay every pending entry, oldest first, through `send`. Stop on the
/// first Auth/Retry outcome so composition order survives; permanent
/// rejections mark the entry failed and move on. A delivered entry flips to
/// `synced` IN PLACE — never deleted — so no snapshot taken mid-flush can
/// watch an entry blink out of existence, and the offline render always has
/// the full transcript. The server's same-second + same-content dedupe is the
/// real idempotency guarantee: a retried send whose response was lost
/// counts as saved there.
pub async fn flush<F, Fut>(db: &Db, mut send: F) -> Result<FlushReport, OutboxError>
where
    F: FnMut(ComposedEntry) -> Fut,
    Fut: Future<Output = SendOutcome>,
{
    let rows = queued_writes(db).await?;
    let mut saved = 0;
    let mut saved_refs = Vec::new();
    let mut blocked = None;
    for queued in rows
        .iter()
        .filter(|queued| queued.entry.state == STATE_PENDING)
    {
        let entry = &queued.entry;
        match send(entry.composed.clone()).await {
            SendOutcome::Saved(server) => {
                saved += 1;
                if mark_synced(
                    db,
                    entry,
                    &queued.write_fingerprint,
                    &server.id,
                    server.written_at,
                )
                .await?
                {
                    saved_refs.push(SavedRefMapping {
                        qid: entry.id.clone(),
                        saved: server,
                    });
                }
            }
            SendOutcome::Auth => {
                blocked = Some(Blocked::Auth);
                break;
            }
            SendOutcome::Retry => {
                blocked = Some(Blocked::Net);
                break;
            }
            SendOutcome::Rejected(status) => {
                mark_failed(
                    db,
                    entry,
                    &queued.write_fingerprint,
                    crate::contract::rejection_reason(status),
                )
                .await?;
            }
        }
    }
    let after = queued(db).await?;
    Ok(FlushReport {
        saved,
        pending: count_state(&after, STATE_PENDING),
        failed: count_state(&after, STATE_FAILED),
        blocked,
        saved_refs,
        pulled: None,
    })
}

fn count_state(entries: &[LocalEntry], state: &str) -> u32 {
    entries.iter().filter(|entry| entry.state == state).count() as u32
}

/// Flip the exact delivered write to synced. The fingerprint is a CAS token:
/// a row discarded and replaced while `send` awaited can never be mistaken
/// for the stale write just accepted by the server. Returns whether the
/// original row transitioned, which is also whether its qid acknowledgement
/// is safe to broadcast.
///
/// A bump creates the server row first, then conditionally deletes the old
/// write in one transaction. Thus a replacement at the old key survives while
/// the delivered server value still materializes at its real key. If the real
/// key is occupied, only a verified destination collision licenses releasing
/// the old write, and that release is fingerprint-guarded too.
async fn mark_synced(
    db: &Db,
    entry: &LocalEntry,
    fingerprint: &str,
    server_id: &str,
    server_written_at: i64,
) -> Result<bool, OutboxError> {
    if server_id == entry.id {
        let mut response = db
            .query(
                "UPDATE type::record('diary_entries', $id) \
                 CONTENT $row \
                 WHERE state = 'pending' AND write_fingerprint = $fingerprint \
                 RETURN VALUE true",
            )
            .bind(("id", entry.id.clone()))
            .bind(("fingerprint", fingerprint.to_string()))
            .bind((
                "row",
                entry.row_with_composed(
                    entry.composed.placed_at(server_written_at),
                    STATE_SYNCED,
                    None,
                ),
            ))
            .await
            .map_err(db_error)?
            .check()
            .map_err(db_error)?;
        let applied: Vec<bool> = response.take(0).map_err(db_error)?;
        return Ok(!applied.is_empty());
    }
    let moved = db
        .query(
            "BEGIN TRANSACTION;
             CREATE ONLY type::record('diary_entries', $new)
                 CONTENT $row RETURN NONE;
             DELETE type::record('diary_entries', $old)
                 WHERE state = 'pending' AND write_fingerprint = $fingerprint
                 RETURN VALUE true;
             COMMIT TRANSACTION;",
        )
        .bind(("old", entry.id.clone()))
        .bind(("new", server_id.to_string()))
        .bind(("fingerprint", fingerprint.to_string()))
        .bind((
            "row",
            LocalRow::new(
                entry.composed.placed_at(server_written_at),
                STATE_SYNCED,
                None,
                entry.enqueued_at,
            ),
        ))
        .await
        .map_err(db_error)?
        .check();
    match moved {
        Ok(mut response) => {
            // BEGIN, CREATE, DELETE, COMMIT are four response slots; the
            // DELETE's boolean marker says whether the original row moved.
            let applied: Vec<bool> = response.take(2).map_err(db_error)?;
            Ok(!applied.is_empty())
        }
        Err(move_error) => {
            // Statement errors are not all collisions. Only an actual row at
            // the destination licenses the intentional release path; every
            // other failure leaves the source untouched for replay.
            if !local_record_exists(db, server_id).await? {
                return Err(db_error(move_error));
            }
            cas_delete_pending(db, &entry.id, fingerprint).await?;
            Ok(false)
        }
    }
}

/// Only pending entries fail; an entry discarded mid-flush stays discarded.
async fn mark_failed(
    db: &Db,
    entry: &LocalEntry,
    fingerprint: &str,
    reason: String,
) -> Result<bool, OutboxError> {
    let mut response = db
        .query(
            "UPDATE type::record('diary_entries', $id) \
             CONTENT $row \
             WHERE state = 'pending' AND write_fingerprint = $fingerprint \
             RETURN VALUE true",
        )
        .bind(("id", entry.id.clone()))
        .bind(("fingerprint", fingerprint.to_string()))
        .bind(("row", entry.row_with(STATE_FAILED, Some(reason))))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let applied: Vec<bool> = response.take(0).map_err(db_error)?;
    Ok(!applied.is_empty())
}

async fn cas_delete_pending(db: &Db, id: &str, fingerprint: &str) -> Result<bool, OutboxError> {
    let mut response = db
        .query(
            "DELETE type::record('diary_entries', $id) \
             WHERE state = 'pending' AND write_fingerprint = $fingerprint \
             RETURN VALUE true",
        )
        .bind(("id", id.to_string()))
        .bind(("fingerprint", fingerprint.to_string()))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let applied: Vec<bool> = response.take(0).map_err(db_error)?;
    Ok(!applied.is_empty())
}

/// Place a composed value through the shared domain algorithm and attach
/// device-only state to a fresh row. A dedupe returns the existing local row
/// unchanged, including its original enqueue order and sync state.
async fn place_local(
    db: &Db,
    requested: &ComposedEntry,
    state: &str,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<LocalEntry, OutboxError> {
    let state = state.to_string();
    let reason = reason.map(str::to_string);
    match place_entry(db, requested, &state, reason.as_deref(), enqueued_at).await? {
        EntryPlacement::Placed(entry) => Ok(LocalEntry {
            entry,
            state,
            reason,
            enqueued_at,
        }),
        EntryPlacement::Deduped(entry) => local_by_id(db, &entry.id)
            .await?
            .ok_or_else(|| OutboxError::Db("deduped local row disappeared".to_string())),
        EntryPlacement::Exhausted => Err(OutboxError::Db(format!(
            "no free second near {}",
            requested.written_at
        ))),
    }
}

/// [`place_local`], but exhaustion and unprojectable epochs fall back to a
/// synthetic failed row instead of erroring — the migration paths use this
/// because migrated text must never be dropped or error out of the drain.
/// Returns whether a row was newly written (a dedupe hit is not).
async fn place_preserving(
    db: &Db,
    requested: &ComposedEntry,
    state: &str,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<bool, OutboxError> {
    if entry_key(requested.written_at).is_some() {
        match place_entry(db, requested, state, reason, enqueued_at).await? {
            EntryPlacement::Placed(_) => return Ok(true),
            EntryPlacement::Deduped(_) => return Ok(false),
            EntryPlacement::Exhausted => {}
        }
    }
    synthetic_failed(db, requested, reason, enqueued_at).await
}

async fn place_entry(
    db: &Db,
    requested: &ComposedEntry,
    state: &str,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<EntryPlacement, OutboxError> {
    let state = state.to_string();
    let reason = reason.map(str::to_string);
    placement::place(
        requested,
        |written_at| {
            entry_key(written_at)
                .ok_or_else(|| OutboxError::Db(format!("unprojectable epoch {written_at}")))
        },
        |id| async move { local_occupant(db, &id).await },
        |entry| {
            let id = entry.id.clone();
            let row = LocalRow::new(entry.composed, state.clone(), reason.clone(), enqueued_at);
            async move { create_row(db, &id, row).await }
        },
    )
    .await
}

/// A deterministic non-projected key for text whose timestamp cannot become
/// a real permalink. Deterministic so re-running a crashed migration
/// converges on the same row.
async fn synthetic_failed(
    db: &Db,
    requested: &ComposedEntry,
    reason: Option<&str>,
    enqueued_at: i64,
) -> Result<bool, OutboxError> {
    let id = format!("failed-{}-{enqueued_at}", requested.written_at);
    if local_record_exists(db, &id).await? {
        Ok(false)
    } else {
        let reason = reason.unwrap_or("unprojectable timestamp");
        create_row(
            db,
            &id,
            LocalRow::new(
                requested.clone(),
                STATE_FAILED,
                Some(reason.to_string()),
                enqueued_at,
            ),
        )
        .await?;
        Ok(true)
    }
}

async fn local_by_id(db: &Db, id: &str) -> Result<Option<LocalEntry>, OutboxError> {
    let mut response = db
        .query(format!(
            "SELECT {PROJECTION}, state, reason, enqueued_at \
             FROM type::record('diary_entries', $id) \
             WHERE entry_version = NONE \
                 OR entry_version <= {CURRENT_WIRE_VERSION}"
        ))
        .bind(("id", id.to_string()))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let rows: Vec<LocalEntry> = response.take(0).map_err(db_error)?;
    Ok(rows.into_iter().next())
}

/// Read only the local metadata needed to decide whether a key is absent,
/// understood, or owned by a newer generation. The nested option preserves
/// the distinction between no row and a legacy row whose version is `NONE`.
async fn local_entry_version(db: &Db, id: &str) -> Result<Option<Option<i64>>, OutboxError> {
    #[derive(Deserialize, SurrealValue)]
    struct VersionRow {
        #[serde(default)]
        entry_version: Option<i64>,
    }

    let mut response = db
        .query("SELECT entry_version FROM type::record('diary_entries', $id)")
        .bind(("id", id.to_string()))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let rows: Vec<VersionRow> = response.take(0).map_err(db_error)?;
    Ok(rows.into_iter().next().map(|row| row.entry_version))
}

async fn local_record_exists(db: &Db, id: &str) -> Result<bool, OutboxError> {
    Ok(local_entry_version(db, id).await?.is_some())
}

async fn local_occupant(db: &Db, id: &str) -> Result<Option<Occupant>, OutboxError> {
    match local_entry_version(db, id).await? {
        None => Ok(None),
        Some(Some(version)) if version > i64::from(CURRENT_WIRE_VERSION) => {
            Ok(Some(Occupant::Opaque))
        }
        Some(_) => Ok(local_by_id(db, id)
            .await?
            .map(|local| Occupant::Known(local.entry))),
    }
}

async fn create_row(db: &Db, id: &str, row: LocalRow) -> Result<(), OutboxError> {
    db.query("CREATE ONLY type::record('diary_entries', $id) CONTENT $row")
        .bind(("id", id.to_string()))
        .bind(("row", row))
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    Ok(())
}

/// The standing v1 drain: port every `diary_outbox` row into the single
/// store (same placement rules as [`import`]), then delete the ported rows
/// one by one. Delete-after-write plus the placement dedupe makes a crash
/// anywhere re-runnable.
async fn drain_v1(db: &Db) -> Result<(), OutboxError> {
    #[derive(Deserialize, SurrealValue)]
    struct V1Row {
        qid: String,
        written_at: i64,
        body: String,
        state: String,
        reason: Option<String>,
        enqueued_at: i64,
    }

    let mut response = db
        .query(
            "SELECT record::id(id) AS qid, written_at, body, state, reason, enqueued_at \
             FROM diary_outbox ORDER BY enqueued_at ASC, qid ASC",
        )
        .await
        .map_err(db_error)?
        .check()
        .map_err(db_error)?;
    let rows: Vec<V1Row> = response.take(0).map_err(db_error)?;
    for row in rows {
        if !row.body.is_empty() {
            let state = if row.state == STATE_FAILED {
                STATE_FAILED
            } else {
                STATE_PENDING
            };
            place_preserving(
                db,
                &ComposedEntry::new(row.written_at, row.body),
                state,
                row.reason.as_deref(),
                row.enqueued_at,
            )
            .await?;
        }
        db.query("DELETE type::record('diary_outbox', $qid)")
            .bind(("qid", row.qid))
            .await
            .map_err(db_error)?
            .check()
            .map_err(db_error)?;
    }
    Ok(())
}

fn db_error(error: surrealdb::Error) -> OutboxError {
    OutboxError::Db(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::future::ready;

    use crate::entry::SavedRef;

    use super::*;

    /// Every test opens `mem://` — a fresh, empty store per call, reached
    /// through the identical `Surreal<Any>` + `open()` path the device uses
    /// for `indxdb://diary`. That sameness is what these tests certify.
    async fn store() -> Db {
        open("mem://").await.expect("mem engine opens")
    }

    fn composed(written_at: i64, body: &str) -> ComposedEntry {
        ComposedEntry::new(written_at, body)
    }

    async fn enqueue(
        db: &Db,
        written_at: i64,
        body: &str,
        enqueued_at: i64,
    ) -> Result<LocalEntry, OutboxError> {
        super::enqueue(db, composed(written_at, body), enqueued_at).await
    }

    /// A server acceptance, as `classify_response` would build it.
    fn saved(id: &str, written_at: i64) -> SendOutcome {
        SendOutcome::Saved(SavedRef {
            id: id.to_string(),
            written_at,
        })
    }

    fn key(epoch: i64) -> String {
        entry_key(epoch).expect("test epoch projects")
    }

    /// Directly read one row in ANY state (queued() hides synced ones).
    async fn row(db: &Db, id: &str) -> Option<LocalEntry> {
        local_by_id(db, id).await.expect("read succeeds")
    }

    #[tokio::test]
    async fn open_is_idempotent_and_drain_reruns_quietly() {
        let db = store().await;
        db.query(SCHEMA).await.unwrap().check().unwrap();
        db.query(V1_SCHEMA).await.unwrap().check().unwrap();
        drain_v1(&db).await.unwrap();
        assert!(queued(&db).await.unwrap().is_empty());
    }

    /// PR10's preceding single-table worker persisted this exact row without
    /// `write_fingerprint`. Opening the upgraded store must backfill the whole
    /// canonical write before `queued_writes` reads it, then the ordinary CAS
    /// transition must deliver it instead of stranding the outbox.
    #[tokio::test]
    async fn concurrent_open_backfills_and_flushes_a_pre_fingerprint_single_store_row() {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns(NAMESPACE).use_db(DATABASE).await.unwrap();
        db.query(
            "DEFINE TABLE diary_entries SCHEMAFULL PERMISSIONS NONE;
             DEFINE FIELD written_at ON diary_entries TYPE int;
             DEFINE FIELD body ON diary_entries TYPE string;
             DEFINE FIELD state ON diary_entries TYPE string;
             DEFINE FIELD reason ON diary_entries TYPE option<string>;
             DEFINE FIELD enqueued_at ON diary_entries TYPE int;",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        let written_at = 1_753_640_000;
        let id = key(written_at);
        db.query(
            "CREATE ONLY type::record('diary_entries', $id) \
             SET written_at = $written_at, body = $body, state = 'pending', \
                 enqueued_at = $enqueued_at",
        )
        .bind(("id", id.clone()))
        .bind(("written_at", written_at))
        .bind(("body", "survives the upgrade".to_string()))
        .bind(("enqueued_at", 7_i64))
        .await
        .unwrap()
        .check()
        .unwrap();

        let (page, worker) = tokio::join!(initialize(&db), initialize(&db));
        page.unwrap();
        worker.unwrap();
        let queued = queued_writes(&db).await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(
            queued[0].write_fingerprint,
            write_fingerprint(&queued[0].entry.composed, 7, LEGACY_ENTRY_VERSION)
        );
        #[derive(Deserialize, SurrealValue)]
        struct VersionRow {
            entry_version: i64,
        }
        let mut response = db
            .query("SELECT entry_version FROM type::record('diary_entries', $id)")
            .bind(("id", id.clone()))
            .await
            .unwrap()
            .check()
            .unwrap();
        let versions: Vec<VersionRow> = response.take(0).unwrap();
        assert_eq!(versions[0].entry_version, LEGACY_ENTRY_VERSION);

        let report = flush(&db, |_| ready(saved(&id, written_at))).await.unwrap();
        assert_eq!((report.saved, report.pending, report.failed), (1, 0, 0));
        assert_eq!(row(&db, &id).await.unwrap().state, STATE_SYNCED);
    }

    #[tokio::test]
    async fn fingerprint_backfill_never_stamps_a_same_id_replacement() {
        let db = store().await;
        let written_at = 1_753_640_000;
        let id = key(written_at);
        let selected = DiaryEntry::from_parts(id.clone(), written_at, "selected old value");
        let enqueued_at = 7;
        let stale_fingerprint =
            write_fingerprint(&selected.composed, enqueued_at, LEGACY_ENTRY_VERSION);

        // This is the value the migration selected. Replace it before its
        // conditional UPDATE lands, exactly like an active old page can.
        db.query(
            "CREATE ONLY type::record('diary_entries', $id) \
             SET written_at = $written_at, body = $body, entry_version = 1, \
                 state = 'pending', enqueued_at = $enqueued_at;
             DELETE type::record('diary_entries', $id);
             CREATE ONLY type::record('diary_entries', $id) \
             SET written_at = $written_at, body = $replacement, entry_version = 1, \
                 state = 'pending', enqueued_at = $enqueued_at;",
        )
        .bind(("id", id.clone()))
        .bind(("written_at", written_at))
        .bind(("body", selected.body.clone()))
        .bind(("replacement", "replacement value".to_string()))
        .bind(("enqueued_at", enqueued_at))
        .await
        .unwrap()
        .check()
        .unwrap();

        install_legacy_fingerprint(
            &db,
            &selected,
            enqueued_at,
            LEGACY_ENTRY_VERSION,
            stale_fingerprint,
        )
        .await
        .unwrap();
        backfill_row_metadata(&db).await.unwrap();
        let queued = queued_writes(&db).await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].entry.body, "replacement value");
        assert_eq!(
            queued[0].write_fingerprint,
            write_fingerprint(&queued[0].entry.composed, enqueued_at, LEGACY_ENTRY_VERSION,)
        );
    }

    /// During a semantic rollout the old worker and new page share this
    /// table. A generation-1 worker must never flush a generation-2 row via
    /// its body-only projection and then acknowledge away the missing field.
    #[tokio::test]
    async fn flush_ignores_rows_from_a_newer_entry_generation() {
        let db = store().await;
        let written_at = 1_753_640_000;
        let id = key(written_at);
        create_row(
            &db,
            &id,
            LocalRow::new(
                ComposedEntry::new(written_at, "future semantics"),
                STATE_PENDING,
                None,
                7,
            ),
        )
        .await
        .unwrap();
        db.query(
            "UPDATE type::record('diary_entries', $id) \
             SET entry_version = $version, write_fingerprint = NONE",
        )
        .bind(("id", id.clone()))
        .bind(("version", i64::from(CURRENT_WIRE_VERSION) + 1))
        .await
        .unwrap()
        .check()
        .unwrap();

        let sends = RefCell::new(0_u32);
        let report = flush(&db, |_| {
            *sends.borrow_mut() += 1;
            ready(SendOutcome::Retry)
        })
        .await
        .unwrap();
        assert_eq!(*sends.borrow(), 0);
        assert_eq!((report.saved, report.pending, report.failed), (0, 0, 0));
        assert_eq!(report.blocked, None);
        #[derive(Deserialize, SurrealValue)]
        struct FingerprintRow {
            body: String,
            state: String,
            #[serde(default)]
            write_fingerprint: Option<String>,
        }
        let mut response = db
            .query(
                "SELECT body, state, write_fingerprint \
                 FROM type::record('diary_entries', $id)",
            )
            .bind(("id", id.clone()))
            .await
            .unwrap()
            .check()
            .unwrap();
        let metadata: Vec<FingerprintRow> = response.take(0).unwrap();
        assert_eq!(metadata[0].body, "future semantics");
        assert_eq!(metadata[0].state, STATE_PENDING);
        assert_eq!(metadata[0].write_fingerprint, None);
        assert!(entry(&db, &id).await.unwrap().is_none());
        assert!(queued(&db).await.unwrap().is_empty());

        // Reconciliation treats the unknown row as occupied, and discard is
        // not authorized to mutate a generation this worker cannot render.
        assert_eq!(
            reconcile(
                &db,
                &[DiaryEntry::from_parts(
                    id.clone(),
                    written_at,
                    "truncated old projection",
                )],
            )
            .await
            .unwrap(),
            0
        );
        discard(&db, &id).await.unwrap();
        assert!(local_record_exists(&db, &id).await.unwrap());

        // Placement sees an opaque occupant, never a body-only replay twin.
        let placed = enqueue(&db, written_at, "future semantics", 8)
            .await
            .unwrap();
        assert_eq!(placed.id, key(written_at + 1));
    }

    /// A preceding page can remain open after the current worker initialized
    /// the shared table. Its row lacks both new metadata fields; the next
    /// flush backfills them in place before deserializing or sending it.
    #[tokio::test]
    async fn flush_backfills_a_stale_writer_after_current_open() {
        let db = store().await;
        let written_at = 1_753_640_000;
        let id = key(written_at);
        db.query(
            "CREATE ONLY type::record('diary_entries', $id) \
             SET written_at = $written_at, body = $body, state = 'pending', \
                 enqueued_at = $enqueued_at",
        )
        .bind(("id", id.clone()))
        .bind(("written_at", written_at))
        .bind(("body", "written by the preceding page".to_string()))
        .bind(("enqueued_at", 7_i64))
        .await
        .unwrap()
        .check()
        .unwrap();

        let report = flush(&db, |entry| {
            ready(saved(&key(entry.written_at), entry.written_at))
        })
        .await
        .unwrap();
        assert_eq!((report.saved, report.pending, report.failed), (1, 0, 0));
        assert_eq!(row(&db, &id).await.unwrap().state, STATE_SYNCED);
    }

    #[tokio::test]
    async fn enqueue_normalizes_predicts_the_permalink_and_round_trips() {
        let db = store().await;
        let entry = enqueue(&db, 1_753_640_000, "Dear diary,\r\nIt me.\r\n", 7)
            .await
            .unwrap();
        assert_eq!(entry.body, "Dear diary,\nIt me.");
        assert_eq!(entry.state, STATE_PENDING);
        // The id is the Entry Key predicted for this second.
        assert_eq!(entry.id, key(1_753_640_000));
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed.len(), 1);
        // The explicit projection must surface the absent `reason` as None
        // instead of dropping the field (the `SELECT *` NONE-omission trap).
        assert_eq!(listed[0].reason, None);
        assert_eq!(listed[0].id, entry.id);
        assert_eq!(listed[0].written_at, 1_753_640_000);
        assert_eq!(listed[0].enqueued_at, 7);
    }

    /// `CONTENT $row` relies on the pinned SDK mapping `Option::None` to
    /// SurrealQL `NONE` (not JSON null). Exercise both directions and a
    /// full-row replacement so a dependency change cannot silently revive
    /// the old per-optional-field query branching.
    #[tokio::test]
    async fn typed_content_rows_round_trip_and_clear_optional_fields() {
        let db = store().await;
        let id = key(1_753_640_000);
        create_row(
            &db,
            &id,
            LocalRow::new(
                ComposedEntry::new(1_753_640_000, "preserved"),
                STATE_FAILED,
                Some("first reason".to_string()),
                7,
            ),
        )
        .await
        .unwrap();
        assert_eq!(
            row(&db, &id).await.unwrap().reason.as_deref(),
            Some("first reason")
        );

        db.query("UPDATE type::record('diary_entries', $id) CONTENT $row")
            .bind(("id", id.clone()))
            .bind((
                "row",
                LocalRow::new(
                    ComposedEntry::new(1_753_640_000, "preserved"),
                    STATE_SYNCED,
                    None,
                    7,
                ),
            ))
            .await
            .unwrap()
            .check()
            .unwrap();
        let updated = row(&db, &id).await.unwrap();
        assert_eq!(updated.reason, None);
        assert_eq!(updated.state, STATE_SYNCED);
        assert_eq!(updated.body, "preserved");
    }

    #[tokio::test]
    async fn enqueue_refuses_only_empty_text() {
        let db = store().await;
        for bad in ["", "  \r\n\t "] {
            assert!(matches!(
                enqueue(&db, 1_753_640_000, bad, 1).await,
                Err(OutboxError::InvalidBody)
            ));
        }
        assert!(queued(&db).await.unwrap().is_empty());
    }

    /// A double-tap that slips the emptied-textarea guard converges on ONE
    /// row — the probe's per-key dedupe, not a second entry the server
    /// would then store twice.
    #[tokio::test]
    async fn enqueue_dedupes_a_same_second_twin() {
        let db = store().await;
        let first = enqueue(&db, 1_753_640_000, "tap", 10).await.unwrap();
        let twin = enqueue(&db, 1_753_640_000, "tap", 25).await.unwrap();
        assert_eq!(twin.id, first.id);
        assert_eq!(twin.enqueued_at, 10, "the original row is the entry");
        assert_eq!(queued(&db).await.unwrap().len(), 1);
    }

    /// Two different thoughts in one second get consecutive seconds — and
    /// therefore distinct predicted Entry Keys at enqueue time.
    #[tokio::test]
    async fn enqueue_probes_a_different_body_forward() {
        let db = store().await;
        let first = enqueue(&db, 1_753_640_000, "one", 10).await.unwrap();
        let second = enqueue(&db, 1_753_640_000, "two", 20).await.unwrap();
        assert_eq!(first.written_at, 1_753_640_000);
        assert_eq!(second.written_at, 1_753_640_001);
        assert_eq!(second.id, key(1_753_640_001));
    }

    /// Over-length text must QUEUE (the server's 422 marks it failed with
    /// its text preserved) — refusing it here would bounce the entry into
    /// the lossy form-POST fallback, and offline that means silent loss.
    #[tokio::test]
    async fn enqueue_accepts_over_length_text_for_the_server_to_judge() {
        use crate::entry::MAX_ENTRY_CHARS;
        let db = store().await;
        let oversized = "a".repeat(MAX_ENTRY_CHARS + 1);
        enqueue(&db, 1_753_640_000, &oversized, 10).await.unwrap();
        let report = flush(&db, |_| ready(SendOutcome::Rejected(422)))
            .await
            .unwrap();
        assert_eq!(report.failed, 1);
        let left = queued(&db).await.unwrap();
        assert_eq!(left[0].state, STATE_FAILED);
        assert_eq!(left[0].body, oversized, "failed text must survive intact");
    }

    #[tokio::test]
    async fn discard_removes_queued_rows_and_spares_synced_history() {
        let db = store().await;
        let queued_entry = enqueue(&db, 1_753_640_000, "keep me", 1).await.unwrap();
        discard(&db, &queued_entry.id).await.unwrap();
        discard(&db, &queued_entry.id).await.unwrap();
        assert!(queued(&db).await.unwrap().is_empty());

        let delivered = enqueue(&db, 1_753_640_100, "delivered", 2).await.unwrap();
        flush(&db, |wire: ComposedEntry| {
            ready(saved(&key(wire.written_at), wire.written_at))
        })
        .await
        .unwrap();
        discard(&db, &delivered.id).await.unwrap();
        assert!(
            row(&db, &delivered.id).await.is_some(),
            "synced history is out of discard's reach"
        );
    }

    /// The heart of the single store: a delivered entry flips to synced IN
    /// PLACE. No snapshot taken at any moment during the flush can watch it
    /// vanish — the gap the old delete-on-save queue forced the page to
    /// paper over with provisional bubbles.
    #[tokio::test]
    async fn flush_sends_oldest_first_and_flips_state_in_place() {
        let db = store().await;
        // Enqueued out of order on purpose; enqueued_at decides.
        let second = enqueue(&db, 1_753_640_200, "second", 20).await.unwrap();
        let first = enqueue(&db, 1_753_640_100, "first", 10).await.unwrap();
        let third = enqueue(&db, 1_753_640_300, "third", 30).await.unwrap();
        let sent = RefCell::new(Vec::new());
        let report = flush(&db, |wire: ComposedEntry| {
            sent.borrow_mut().push(wire.body.clone());
            ready(saved(&key(wire.written_at), wire.written_at))
        })
        .await
        .unwrap();
        assert_eq!(*sent.borrow(), ["first", "second", "third"]);
        assert_eq!(report.saved, 3);
        assert_eq!(
            (report.pending, report.failed, report.blocked),
            (0, 0, None)
        );
        assert_eq!(report.saved_refs.len(), 3);
        assert_eq!(report.saved_refs[0].qid, first.id);
        assert_eq!(
            report.saved_refs[0].saved.id, first.id,
            "no bump: same identity"
        );
        // Queued view is empty — but every row still exists, synced.
        assert!(queued(&db).await.unwrap().is_empty());
        for entry in [&first, &second, &third] {
            let kept = row(&db, &entry.id).await.expect("row survives delivery");
            assert_eq!(kept.state, STATE_SYNCED);
            assert_eq!(kept.body, entry.body);
        }
    }

    /// A queued row may have been written by an older worker whose canonical
    /// entry shape serialized differently. Flush must compare against that
    /// row's persisted generation token, not recompute one with today's type.
    #[tokio::test]
    async fn flush_uses_the_fingerprint_stored_with_the_queued_write() {
        let db = store().await;
        let written_at = 1_753_640_000;
        let entry = enqueue(&db, written_at, "written by an older worker", 10)
            .await
            .unwrap();
        let stored_fingerprint = "f".repeat(64);
        assert_ne!(
            stored_fingerprint,
            write_fingerprint(
                &entry.composed,
                entry.enqueued_at,
                i64::from(CURRENT_WIRE_VERSION),
            ),
            "the fixture must not accidentally be today's recomputed token"
        );
        db.query(
            "UPDATE type::record('diary_entries', $id) \
             SET write_fingerprint = $fingerprint",
        )
        .bind(("id", entry.id.clone()))
        .bind(("fingerprint", stored_fingerprint))
        .await
        .unwrap()
        .check()
        .unwrap();

        let report = flush(&db, |wire: ComposedEntry| {
            ready(saved(&key(wire.written_at), wire.written_at))
        })
        .await
        .unwrap();

        assert_eq!(report.saved, 1);
        assert_eq!(report.saved_refs.len(), 1);
        assert_eq!(row(&db, &entry.id).await.unwrap().state, STATE_SYNCED);
    }

    #[tokio::test]
    async fn flush_stops_on_retryable_trouble() {
        let db = store().await;
        enqueue(&db, 1_753_640_100, "first", 10).await.unwrap();
        enqueue(&db, 1_753_640_200, "second", 20).await.unwrap();
        enqueue(&db, 1_753_640_300, "third", 30).await.unwrap();
        let script = RefCell::new(VecDeque::from([
            saved(&key(1_753_640_100), 1_753_640_100),
            SendOutcome::Retry,
        ]));
        let report = flush(&db, |_| {
            let outcome = script.borrow_mut().pop_front().expect("third never sent");
            ready(outcome)
        })
        .await
        .unwrap();
        assert_eq!((report.saved, report.pending, report.failed), (1, 2, 0));
        assert_eq!(report.blocked, Some(Blocked::Net));
        // The stopped entries survive, order intact, for the next flush.
        let left: Vec<String> = queued(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.entry.composed.content.body)
            .collect();
        assert_eq!(left, ["second", "third"]);
    }

    #[tokio::test]
    async fn flush_stops_quietly_when_signed_out() {
        let db = store().await;
        enqueue(&db, 1_753_640_100, "private", 10).await.unwrap();
        let report = flush(&db, |_| ready(SendOutcome::Auth)).await.unwrap();
        assert_eq!(report.blocked, Some(Blocked::Auth));
        assert_eq!(report.pending, 1);
    }

    #[tokio::test]
    async fn flush_marks_rejections_failed_and_moves_on() {
        let db = store().await;
        enqueue(&db, 1_753_640_100, "rejected", 10).await.unwrap();
        enqueue(&db, 1_753_640_200, "accepted", 20).await.unwrap();
        let script = RefCell::new(VecDeque::from([
            SendOutcome::Rejected(422),
            saved(&key(1_753_640_200), 1_753_640_200),
        ]));
        let report = flush(&db, |_| {
            let outcome = script.borrow_mut().pop_front().unwrap();
            ready(outcome)
        })
        .await
        .unwrap();
        assert_eq!((report.saved, report.pending, report.failed), (1, 0, 1));
        assert_eq!(report.blocked, None);
        assert_eq!(report.saved_refs[0].saved.id, key(1_753_640_200));
        let left = queued(&db).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].state, STATE_FAILED);
        assert_eq!(left[0].reason.as_deref(), Some("rejected (HTTP 422)"));
        // A failed entry is kept for manual copy, never re-sent.
        let report = flush(&db, |_| ready(saved("never", 0))).await.unwrap();
        assert_eq!(report.saved, 0);
        assert_eq!(queued(&db).await.unwrap().len(), 1);
    }

    /// `flush` reads its work before awaiting the transport. If the page
    /// discards that write and places different text under the same predicted
    /// key while the request is in flight, the acceptance belongs to the old
    /// generation, never to its replacement.
    #[tokio::test]
    async fn a_saved_stale_send_never_marks_its_same_id_replacement_synced() {
        let db = store().await;
        let written_at = 1_753_640_000;
        let original = enqueue(&db, written_at, "sent", 10).await.unwrap();
        let original_id = original.id.clone();
        let db_during_send = db.clone();
        let send_id = original_id.clone();
        let report = flush(&db, move |_| {
            let db = db_during_send.clone();
            let old_id = original_id.clone();
            let server_id = send_id.clone();
            async move {
                discard(&db, &old_id).await.unwrap();
                let replacement = enqueue(&db, written_at, "replacement", 20).await.unwrap();
                assert_eq!(replacement.id, old_id);
                saved(&server_id, written_at)
            }
        })
        .await
        .unwrap();

        assert_eq!(report.saved, 1, "the server still accepted the old write");
        assert!(
            report.saved_refs.is_empty(),
            "a stale qid must not acknowledge its replacement"
        );
        assert_eq!((report.pending, report.failed), (1, 0));
        let replacement = row(&db, &original.id).await.unwrap();
        assert_eq!(replacement.state, STATE_PENDING);
        assert_eq!(replacement.body, "replacement");
    }

    #[tokio::test]
    async fn a_rejected_stale_send_never_fails_its_same_id_replacement() {
        let db = store().await;
        let written_at = 1_753_640_000;
        let original = enqueue(&db, written_at, "rejected", 10).await.unwrap();
        let original_id = original.id.clone();
        let db_during_send = db.clone();
        let report = flush(&db, move |_| {
            let db = db_during_send.clone();
            let old_id = original_id.clone();
            async move {
                discard(&db, &old_id).await.unwrap();
                let replacement = enqueue(&db, written_at, "replacement", 20).await.unwrap();
                assert_eq!(replacement.id, old_id);
                SendOutcome::Rejected(422)
            }
        })
        .await
        .unwrap();

        assert_eq!((report.saved, report.pending, report.failed), (0, 1, 0));
        let replacement = row(&db, &original.id).await.unwrap();
        assert_eq!(replacement.state, STATE_PENDING);
        assert_eq!(replacement.reason, None);
        assert_eq!(replacement.body, "replacement");
    }

    /// The server's cross-device probe bumped the entry a second forward:
    /// the local row moves to the server's identity, and the report maps
    /// the predicted id to the real one for the page's single fix-up.
    #[tokio::test]
    async fn flush_rekeys_a_server_bumped_entry() {
        let db = store().await;
        let entry = enqueue(&db, 1_753_640_000, "bumped", 10).await.unwrap();
        let server_id = key(1_753_640_001);
        let report = flush(&db, |_| ready(saved(&server_id, 1_753_640_001)))
            .await
            .unwrap();
        assert_eq!(report.saved_refs[0].qid, entry.id);
        assert_eq!(report.saved_refs[0].saved.id, server_id);
        assert!(row(&db, &entry.id).await.is_none(), "old key released");
        let moved = row(&db, &server_id).await.expect("row lives at server id");
        assert_eq!(moved.state, STATE_SYNCED);
        assert_eq!(moved.written_at, 1_753_640_001);
        assert_eq!(moved.body, "bumped");
    }

    /// The destination is free but the source key was reused while the send
    /// awaited. Create-first movement materializes the accepted old write at
    /// the server key and the fingerprint CAS leaves the replacement queued.
    #[tokio::test]
    async fn a_bump_to_a_free_key_preserves_a_same_id_replacement() {
        let db = store().await;
        let written_at = 1_753_640_000;
        let original = enqueue(&db, written_at, "sent and bumped", 10)
            .await
            .unwrap();
        let old_id = original.id.clone();
        let server_id = key(written_at + 1);
        let db_during_send = db.clone();
        let replacement_id = old_id.clone();
        let accepted_id = server_id.clone();
        let report = flush(&db, move |_| {
            let db = db_during_send.clone();
            let old_id = replacement_id.clone();
            let server_id = accepted_id.clone();
            async move {
                discard(&db, &old_id).await.unwrap();
                let replacement = enqueue(&db, written_at, "replacement", 20).await.unwrap();
                assert_eq!(replacement.id, old_id);
                saved(&server_id, written_at + 1)
            }
        })
        .await
        .unwrap();

        assert_eq!(report.saved, 1);
        assert!(report.saved_refs.is_empty());
        assert_eq!((report.pending, report.failed), (1, 0));
        let replacement = row(&db, &old_id).await.unwrap();
        assert_eq!(replacement.state, STATE_PENDING);
        assert_eq!(replacement.body, "replacement");
        let accepted = row(&db, &server_id).await.unwrap();
        assert_eq!(accepted.state, STATE_SYNCED);
        assert_eq!(accepted.body, "sent and bumped");
        assert_eq!(accepted.written_at, written_at + 1);
    }

    /// Double collision: the server bumps an entry onto a second that a
    /// DIFFERENT local pending row already occupies. The delivered row is
    /// released (its text is safe server-side; the next-in-lock pull will
    /// place it) and the pending neighbor is never overwritten — queued
    /// diary text is never silently clobbered.
    #[tokio::test]
    async fn a_bump_never_clobbers_a_pending_neighbor() {
        let db = store().await;
        let bumped = enqueue(&db, 1_753_640_000, "mine", 10).await.unwrap();
        let neighbor = enqueue(&db, 1_753_640_001, "neighbor", 20).await.unwrap();
        let script = RefCell::new(VecDeque::from([
            // The server saved "mine" at T+1 (some other device's entry
            // occupied T there)…
            saved(&key(1_753_640_001), 1_753_640_001),
            // …and the flush stops before the neighbor is attempted.
            SendOutcome::Retry,
        ]));
        let report = flush(&db, |_| {
            let outcome = script.borrow_mut().pop_front().unwrap();
            ready(outcome)
        })
        .await
        .unwrap();
        assert_eq!(report.saved, 1);
        assert!(
            report.saved_refs.is_empty(),
            "a collision release has no local row safe to acknowledge"
        );
        assert!(
            row(&db, &bumped.id).await.is_none(),
            "delivered row released"
        );
        let kept = row(&db, &neighbor.id).await.expect("neighbor survives");
        assert_eq!(kept.state, STATE_PENDING);
        assert_eq!(kept.body, "neighbor");
    }

    /// The collision-release fallback is itself a CAS: if the source was
    /// replaced while the accepted write was in flight, neither that
    /// replacement nor the row occupying the bumped destination is deleted.
    #[tokio::test]
    async fn a_bump_collision_never_releases_a_source_replacement() {
        let db = store().await;
        let written_at = 1_753_640_000;
        let original = enqueue(&db, written_at, "mine", 10).await.unwrap();
        let neighbor = enqueue(&db, written_at + 1, "neighbor", 20).await.unwrap();
        let original_id = original.id.clone();
        let server_id = neighbor.id.clone();
        let db_during_send = db.clone();
        let replacement_id = original_id.clone();
        let accepted_id = server_id.clone();
        let report = flush(&db, move |sent| {
            let db = db_during_send.clone();
            let old_id = replacement_id.clone();
            let server_id = accepted_id.clone();
            async move {
                if sent.body != "mine" {
                    return SendOutcome::Retry;
                }
                discard(&db, &old_id).await.unwrap();
                let replacement = enqueue(&db, written_at, "replacement", 30).await.unwrap();
                assert_eq!(replacement.id, old_id);
                saved(&server_id, written_at + 1)
            }
        })
        .await
        .unwrap();

        assert_eq!(report.saved, 1);
        assert_eq!(report.blocked, Some(Blocked::Net));
        assert!(report.saved_refs.is_empty());
        assert_eq!((report.pending, report.failed), (2, 0));
        let replacement = row(&db, &original_id).await.unwrap();
        assert_eq!(replacement.state, STATE_PENDING);
        assert_eq!(replacement.body, "replacement");
        let kept_neighbor = row(&db, &server_id).await.unwrap();
        assert_eq!(kept_neighbor.state, STATE_PENDING);
        assert_eq!(kept_neighbor.body, "neighbor");
    }

    #[tokio::test]
    async fn the_v1_drain_ports_and_empties_the_old_queue() {
        let db = store().await;
        for (written_at, body, state, reason, enqueued_at) in [
            (1_753_640_000_i64, "old pending", "pending", None, 1_i64),
            (
                1_753_640_060,
                "old failure",
                "failed",
                Some("rejected (HTTP 422)"),
                2,
            ),
        ] {
            let statement = match reason {
                Some(_) => {
                    "CREATE diary_outbox SET written_at = $written_at, body = $body, \
                     state = $state, reason = $reason, enqueued_at = $enqueued_at"
                }
                None => {
                    "CREATE diary_outbox SET written_at = $written_at, body = $body, \
                     state = $state, enqueued_at = $enqueued_at"
                }
            };
            let mut query = db
                .query(statement)
                .bind(("written_at", written_at))
                .bind(("body", body.to_string()))
                .bind(("state", state.to_string()))
                .bind(("enqueued_at", enqueued_at));
            if let Some(reason) = reason {
                query = query.bind(("reason", reason.to_string()));
            }
            query.await.unwrap().check().unwrap();
        }
        drain_v1(&db).await.unwrap();
        // Re-running the drain (a crash replay) changes nothing.
        drain_v1(&db).await.unwrap();
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].body, "old pending");
        assert_eq!(listed[0].state, STATE_PENDING);
        assert_eq!(
            listed[0].id,
            key(1_753_640_000),
            "ported rows get projected Entry Keys"
        );
        assert_eq!(listed[1].state, STATE_FAILED);
        assert_eq!(listed[1].reason.as_deref(), Some("rejected (HTTP 422)"));
        #[derive(Deserialize, SurrealValue)]
        struct CountRow {
            count: i64,
        }
        let mut response = db
            .query("SELECT count() FROM diary_outbox GROUP ALL")
            .await
            .unwrap()
            .check()
            .unwrap();
        let counts: Vec<CountRow> = response.take(0).unwrap();
        assert!(
            counts.is_empty() || counts[0].count == 0,
            "v1 queue emptied"
        );
    }

    /// A row whose timestamp cannot project still ports — under a synthetic
    /// key, failed, with its text intact. Never dropped, never an error.
    #[tokio::test]
    async fn unprojectable_rows_port_as_synthetic_failures() {
        let db = store().await;
        db.query(
            "CREATE diary_outbox SET written_at = $written_at, body = $body, \
             state = 'pending', enqueued_at = 9",
        )
        .bind(("written_at", i64::MIN))
        .bind(("body", "clock went sideways".to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
        drain_v1(&db).await.unwrap();
        drain_v1(&db).await.unwrap();
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].state, STATE_FAILED);
        assert_eq!(listed[0].reason.as_deref(), Some("unprojectable timestamp"));
        assert_eq!(listed[0].body, "clock went sideways");
        assert!(listed[0].id.starts_with("failed-"));
    }

    #[tokio::test]
    async fn import_preserves_and_dedupes() {
        let db = store().await;
        let legacy = vec![
            LegacyEntry {
                written_at: 1_753_640_000,
                body: "old pending".to_string(),
                state: Some(STATE_PENDING.to_string()),
                reason: None,
                enqueued_at: Some(1),
            },
            LegacyEntry {
                written_at: 1_753_640_060,
                body: "old failure".to_string(),
                state: Some(STATE_FAILED.to_string()),
                reason: Some("rejected (HTTP 422)".to_string()),
                enqueued_at: Some(2),
            },
        ];
        assert_eq!(import(&db, &legacy).await.unwrap(), 2);
        // A crash between import and the caller's delete replays the whole
        // batch; the placement dedupe absorbs it.
        assert_eq!(import(&db, &legacy).await.unwrap(), 0);
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].body, "old pending");
        assert_eq!(listed[0].state, STATE_PENDING);
        assert_eq!(listed[1].state, STATE_FAILED);
        assert_eq!(listed[1].reason.as_deref(), Some("rejected (HTTP 422)"));
    }

    #[tokio::test]
    async fn import_tolerates_the_old_shape() {
        // Real legacy records carry qid and enqueued_at; a hand-cleared
        // store might hold less. Unknown fields are ignored, absent
        // optionals default.
        let raw = r#"[
            {"qid": 3, "written_at": 1753640000, "body": "with qid", "state": "pending",
             "reason": null, "enqueued_at": 1753640000000},
            {"written_at": 1753640060, "body": "bare"}
        ]"#;
        let legacy: Vec<LegacyEntry> = serde_json::from_str(raw).unwrap();
        let db = store().await;
        assert_eq!(import(&db, &legacy).await.unwrap(), 2);
        let listed = queued(&db).await.unwrap();
        assert_eq!(listed[0].body, "bare"); // enqueued_at defaults to 0
        assert_eq!(listed[1].body, "with qid");
        assert_eq!(listed[1].state, STATE_PENDING);
    }

    /// The report is the BroadcastChannel message the page has always read;
    /// `qid` is the predicted Entry Key now (a path-shaped string).
    #[test]
    fn reports_serialize_in_the_broadcast_shape() {
        let report = FlushReport {
            saved: 1,
            pending: 1,
            failed: 0,
            blocked: Some(Blocked::Net),
            saved_refs: vec![SavedRefMapping {
                qid: "2026-07-27T14-30-45-04-00".to_string(),
                saved: SavedRef {
                    id: "2026-07-27T14-30-46-04-00".to_string(),
                    written_at: 1_753_640_000,
                },
            }],
            pulled: Some(2),
        };
        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            r#"{"saved":1,"pending":1,"failed":0,"blocked":"net","saved_refs":[{"qid":"2026-07-27T14-30-45-04-00","id":"2026-07-27T14-30-46-04-00","written_at":1753640000}],"pulled":2}"#
        );
        let quiet = FlushReport {
            saved: 0,
            pending: 0,
            failed: 0,
            blocked: None,
            saved_refs: Vec::new(),
            pulled: None,
        };
        assert_eq!(
            serde_json::to_string(&quiet).unwrap(),
            r#"{"saved":0,"pending":0,"failed":0,"blocked":null,"saved_refs":[],"pulled":null}"#
        );
    }
}
