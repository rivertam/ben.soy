-- Now mutations use revision CAS and durable, body-clearing tombstones.
-- No claimless bridge: old readers cannot interpret edits or emoji-only rows.
DEFINE TABLE OVERWRITE diary_entries SCHEMAFULL
    PERMISSIONS FOR select, create, update WHERE $access = 'diary_sync'
        AND $token.diary_schema_epoch = 3;
DEFINE FIELD OVERWRITE id ON diary_entries TYPE string
    ASSERT string::matches(record::id($value),
        '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}[+-][0-9]{2}-[0-9]{2}$');
DEFINE FIELD OVERWRITE written_at ON diary_entries TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;
DEFINE FIELD OVERWRITE body ON diary_entries TYPE string
    ASSERT string::len($value) <= 65536;
DEFINE FIELD OVERWRITE reply_to ON diary_entries TYPE option<string>
    ASSERT $value = NONE OR string::matches($value,
        '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}[+-][0-9]{2}-[0-9]{2}$');
DEFINE INDEX IF NOT EXISTS diary_entries_written_at ON diary_entries FIELDS written_at;

DEFINE FIELD OVERWRITE emoji ON diary_entries TYPE option<string>;
DEFINE FIELD OVERWRITE occurred_at ON diary_entries TYPE option<int>;
DEFINE FIELD OVERWRITE saved_at_ms ON diary_entries TYPE option<int>;
DEFINE FIELD OVERWRITE revision ON diary_entries TYPE option<string>;
DEFINE FIELD OVERWRITE edit ON diary_entries TYPE option<object>;
DEFINE FIELD OVERWRITE edit.id ON diary_entries TYPE string;
DEFINE FIELD OVERWRITE edit.base ON diary_entries TYPE option<string>;
DEFINE FIELD OVERWRITE edit.deleted ON diary_entries TYPE bool;
DEFINE FIELD OVERWRITE edit.previous ON diary_entries TYPE array<string> DEFAULT [];

-- Daily writing and writer handoffs go through the owner-gated HTTP API.
-- Record-access direct-sync tokens cannot bypass the budget/closure checks.
DEFINE TABLE OVERWRITE diary_days SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE day ON diary_days TYPE string;
DEFINE FIELD OVERWRITE body ON diary_days TYPE string ASSERT string::len($value) <= 65536;
DEFINE FIELD OVERWRITE used_ms ON diary_days TYPE int ASSERT $value >= 0 AND $value <= 900000;
DEFINE FIELD OVERWRITE closed ON diary_days TYPE bool;
DEFINE FIELD OVERWRITE closed_at ON diary_days TYPE option<int>;
DEFINE FIELD OVERWRITE updated_at ON diary_days TYPE int;
DEFINE FIELD OVERWRITE revision ON diary_days TYPE int ASSERT $value >= 1;
DEFINE FIELD OVERWRITE generation ON diary_days TYPE int ASSERT $value >= 1;
DEFINE TABLE OVERWRITE diary_writer SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE device ON diary_writer TYPE string;
DEFINE FIELD OVERWRITE generation ON diary_writer TYPE int;
UPSERT diary_writer:today SET device = device ?? '', generation = generation ?? 0;

REMOVE FIELD IF EXISTS entry_version ON diary_entries;

-- One timestamp per distinct emoji; no deleted entry content is retained.
DEFINE TABLE OVERWRITE diary_emojis SCHEMAFULL PERMISSIONS FOR select, create, update WHERE $access = 'diary_sync' AND $token.diary_schema_epoch = 3;
DEFINE FIELD OVERWRITE emoji ON diary_emojis TYPE string;
DEFINE FIELD OVERWRITE used_at_ms ON diary_emojis TYPE int;
