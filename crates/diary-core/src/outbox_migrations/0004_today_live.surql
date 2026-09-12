DEFINE TABLE OVERWRITE diary_entries SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE written_at ON diary_entries TYPE int;
DEFINE FIELD OVERWRITE body ON diary_entries TYPE string;
DEFINE FIELD OVERWRITE reply_to ON diary_entries TYPE option<string>;
DEFINE FIELD OVERWRITE state ON diary_entries TYPE string
    ASSERT $value IN ['synced', 'pending', 'failed'];
DEFINE FIELD OVERWRITE reason ON diary_entries TYPE option<string>;
DEFINE FIELD OVERWRITE enqueued_at ON diary_entries TYPE int;
DEFINE FIELD OVERWRITE write_fingerprint ON diary_entries TYPE option<string>
    ASSERT $value = NONE OR string::matches($value, '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE emoji ON diary_entries TYPE option<string>;
DEFINE FIELD OVERWRITE occurred_at ON diary_entries TYPE option<int>;
DEFINE FIELD OVERWRITE saved_at_ms ON diary_entries TYPE option<int>;
DEFINE FIELD OVERWRITE revision ON diary_entries TYPE option<string>;
DEFINE FIELD OVERWRITE edit ON diary_entries TYPE option<object>;
DEFINE FIELD OVERWRITE edit.id ON diary_entries TYPE string;
DEFINE FIELD OVERWRITE edit.base ON diary_entries TYPE option<string>;
DEFINE FIELD OVERWRITE edit.deleted ON diary_entries TYPE bool;
DEFINE FIELD OVERWRITE edit.previous ON diary_entries TYPE array<string> DEFAULT [];
-- Legacy local Today rows are left intact for recovery. Today no longer
-- mirrors or queues reflections; its live API is the only write path.

-- One timestamp per distinct emoji; no deleted entry content is retained.
DEFINE TABLE OVERWRITE diary_emojis SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE emoji ON diary_emojis TYPE string;
DEFINE FIELD OVERWRITE used_at_ms ON diary_emojis TYPE int;
