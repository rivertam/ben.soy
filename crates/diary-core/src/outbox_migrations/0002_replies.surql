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
