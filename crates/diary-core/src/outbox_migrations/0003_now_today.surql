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
DEFINE TABLE OVERWRITE diary_days SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE day ON diary_days TYPE string;
DEFINE FIELD OVERWRITE body ON diary_days TYPE string;
DEFINE FIELD OVERWRITE used_ms ON diary_days TYPE int ASSERT $value >= 0 AND $value <= 900000;
DEFINE FIELD OVERWRITE closed ON diary_days TYPE bool;
DEFINE FIELD OVERWRITE closed_at ON diary_days TYPE option<int>;
DEFINE FIELD OVERWRITE updated_at ON diary_days TYPE int;
DEFINE FIELD OVERWRITE revision ON diary_days TYPE int;
DEFINE FIELD OVERWRITE generation ON diary_days TYPE int;
DEFINE FIELD OVERWRITE state ON diary_days TYPE string;
DEFINE FIELD OVERWRITE reserved_ms ON diary_days TYPE int;
DEFINE TABLE OVERWRITE diary_local SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE device ON diary_local TYPE string;
DEFINE FIELD OVERWRITE generation ON diary_local TYPE int;
DEFINE FIELD OVERWRITE releasing ON diary_local TYPE bool;

-- One timestamp per distinct emoji; no deleted entry content is retained.
DEFINE TABLE OVERWRITE diary_emojis SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE emoji ON diary_emojis TYPE string;
DEFINE FIELD OVERWRITE used_at_ms ON diary_emojis TYPE int;
