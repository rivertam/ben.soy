-- Add soft reply relationships. A Reply Target is a canonical permalink id;
-- it may outlive, or temporarily precede, the row it points at.
--
-- The claimless branch remains a narrow bridge for a pre-epoch session, but
-- it can see and create top-level entries only. Explicit epoch-1 and future
-- sessions are denied; current epoch-2 sessions carry the complete shape.
DEFINE TABLE OVERWRITE diary_entries SCHEMAFULL
    PERMISSIONS
    FOR select WHERE $access = 'diary_sync'
        AND ($token.diary_schema_epoch = 2
            OR ($token.diary_schema_epoch = NONE AND reply_to = NONE))
    FOR create WHERE $access = 'diary_sync'
        AND ($token.diary_schema_epoch = 2
            OR ($token.diary_schema_epoch = NONE AND reply_to = NONE));
DEFINE FIELD OVERWRITE id ON diary_entries TYPE string
    ASSERT string::matches(record::id($value),
        '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}[+-][0-9]{2}-[0-9]{2}$');
DEFINE FIELD OVERWRITE written_at ON diary_entries TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;
DEFINE FIELD OVERWRITE body ON diary_entries TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 65536;
DEFINE FIELD OVERWRITE reply_to ON diary_entries TYPE option<string>
    ASSERT $value = NONE OR string::matches($value,
        '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}[+-][0-9]{2}-[0-9]{2}$');
DEFINE INDEX OVERWRITE diary_entries_written_at ON diary_entries FIELDS written_at;

-- Keep the current-state migration able to reconcile review deployments of
-- the discarded per-row generation design without retaining that metadata.
DEFINE FIELD OVERWRITE entry_version ON diary_entries TYPE option<int>;
UPDATE diary_entries UNSET entry_version;
REMOVE FIELD IF EXISTS entry_version ON diary_entries;
