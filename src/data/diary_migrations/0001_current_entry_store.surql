-- Establish the canonical server store at schema epoch 1. Existing body-only
-- rows already have this shape and remain in place.
--
-- The claimless branch is a one-rollout bridge for a direct-sync session
-- minted before schema epochs existed. It can only carry the epoch-1 shape;
-- all tokens minted by current code carry an exact `diary_schema_epoch`.
DEFINE TABLE OVERWRITE diary_entries SCHEMAFULL
    PERMISSIONS
    FOR select WHERE $access = 'diary_sync'
        AND ($token.diary_schema_epoch = 1
            OR $token.diary_schema_epoch = NONE)
    FOR create WHERE $access = 'diary_sync'
        AND ($token.diary_schema_epoch = 1
            OR $token.diary_schema_epoch = NONE);
DEFINE FIELD OVERWRITE id ON diary_entries TYPE string
    ASSERT string::matches(record::id($value),
        '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}[+-][0-9]{2}-[0-9]{2}$');
DEFINE FIELD OVERWRITE written_at ON diary_entries TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;
DEFINE FIELD OVERWRITE body ON diary_entries TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 65536;
DEFINE INDEX OVERWRITE diary_entries_written_at ON diary_entries FIELDS written_at;

-- Clean up review deployments of the discarded per-row generation design.
DEFINE FIELD OVERWRITE entry_version ON diary_entries TYPE option<int>;
UPDATE diary_entries UNSET entry_version;
REMOVE FIELD IF EXISTS entry_version ON diary_entries;
