-- The application reconciles these table and field definitions on its first
-- database connection. Indexes are forward-only migrations in
-- data/schema_migrations/: DEFINE INDEX OVERWRITE destroys and rebuilds an
-- existing index on SurrealDB 3.2.3, even when its definition is unchanged.

DEFINE TABLE OVERWRITE spire_runs SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON spire_runs TYPE string;
-- `game`/`run_id` are optional only so rows written before multi-game support
-- remain readable. Reads project missing values as sts2 / record::id(id).
DEFINE FIELD OVERWRITE game ON spire_runs TYPE option<string>;
DEFINE FIELD OVERWRITE run_id ON spire_runs TYPE option<string>;
DEFINE FIELD OVERWRITE date ON spire_runs TYPE string;
DEFINE FIELD OVERWRITE start_time ON spire_runs TYPE int;
DEFINE FIELD OVERWRITE character ON spire_runs TYPE string;
DEFINE FIELD OVERWRITE win ON spire_runs TYPE bool;
DEFINE FIELD OVERWRITE abandoned ON spire_runs TYPE bool;
DEFINE FIELD OVERWRITE ascension ON spire_runs TYPE int;
DEFINE FIELD OVERWRITE acts ON spire_runs TYPE int;
DEFINE FIELD OVERWRITE floors ON spire_runs TYPE int;
DEFINE FIELD OVERWRITE killed_by ON spire_runs TYPE option<string>;
DEFINE FIELD OVERWRITE kill_kind ON spire_runs TYPE option<string>;
DEFINE FIELD OVERWRITE run_time ON spire_runs TYPE int;
DEFINE FIELD OVERWRITE seed ON spire_runs TYPE string;
DEFINE FIELD OVERWRITE game_mode ON spire_runs TYPE string;
DEFINE FIELD OVERWRITE build_id ON spire_runs TYPE string;
DEFINE FIELD OVERWRITE added_at ON spire_runs TYPE int;

DEFINE TABLE OVERWRITE spire_run_raws SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON spire_run_raws TYPE string;
DEFINE FIELD OVERWRITE game ON spire_run_raws TYPE option<string>;
DEFINE FIELD OVERWRITE run_id ON spire_run_raws TYPE option<string>;
DEFINE FIELD OVERWRITE raw ON spire_run_raws TYPE string;

DEFINE TABLE OVERWRITE spire_meta SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON spire_meta TYPE string;
DEFINE FIELD OVERWRITE k ON spire_meta TYPE string;
DEFINE FIELD OVERWRITE v ON spire_meta TYPE int;

DEFINE TABLE OVERWRITE workouts SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON workouts TYPE string;
DEFINE FIELD OVERWRITE title ON workouts TYPE string;
DEFINE FIELD OVERWRITE raw_title ON workouts TYPE string;
DEFINE FIELD OVERWRITE started_at_utc ON workouts TYPE string;
DEFINE FIELD OVERWRITE started_at_local ON workouts TYPE string;
DEFINE FIELD OVERWRITE eastern_offset_minutes ON workouts TYPE int;
DEFINE FIELD OVERWRITE duration_seconds ON workouts TYPE int;
DEFINE FIELD OVERWRITE duration_suspicious ON workouts TYPE bool;
DEFINE FIELD OVERWRITE notes ON workouts TYPE option<string>;
DEFINE FIELD OVERWRITE description ON workouts TYPE option<string>;
DEFINE FIELD OVERWRITE source ON workouts TYPE string;
DEFINE FIELD OVERWRITE imported_at ON workouts TYPE int;

DEFINE TABLE OVERWRITE exercises SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON exercises TYPE string;
DEFINE FIELD OVERWRITE name ON exercises TYPE string;

-- Alternate source names for one canonical exercise. Imports resolve these
-- before writing sets or taxonomy; the public snapshot also resolves them so
-- a row added directly in the database takes effect on existing history.
-- Application writes keep aliases direct: an alias may not itself be a
-- canonical exercise or point at another alias.
DEFINE TABLE OVERWRITE exercise_aliases SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON exercise_aliases TYPE string;
DEFINE FIELD OVERWRITE alias_name ON exercise_aliases TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 200;
DEFINE FIELD OVERWRITE canonical_name ON exercise_aliases TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 200;
DEFINE FIELD OVERWRITE updated_at ON exercise_aliases TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;

DEFINE TABLE OVERWRITE exercise_tags SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON exercise_tags TYPE string;
DEFINE FIELD OVERWRITE exercise_name ON exercise_tags TYPE string;
DEFINE FIELD OVERWRITE kind ON exercise_tags TYPE string;
DEFINE FIELD OVERWRITE value ON exercise_tags TYPE string;

DEFINE TABLE OVERWRITE sets SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON sets TYPE string;
DEFINE FIELD OVERWRITE workout_id ON sets TYPE string;
DEFINE FIELD OVERWRITE exercise_name ON sets TYPE string;
DEFINE FIELD OVERWRITE raw_exercise_name ON sets TYPE string;
DEFINE FIELD OVERWRITE ordinal ON sets TYPE int;
DEFINE FIELD OVERWRITE exercise_note ON sets TYPE option<string>;
DEFINE FIELD OVERWRITE superset_id ON sets TYPE option<int>;
DEFINE FIELD OVERWRITE weight_milli ON sets TYPE option<int>;
DEFINE FIELD OVERWRITE weight_unit ON sets TYPE string;
DEFINE FIELD OVERWRITE reps ON sets TYPE option<int>;
DEFINE FIELD OVERWRITE effort_hundredths ON sets TYPE option<int>;
-- True failure is the endpoint beyond numeric RPE 10 on the effort axis.
-- Application writes keep this mutually exclusive with effort_hundredths;
-- migration 8 moves the legacy FAILURE_SET representation here.
-- Optional at the storage boundary so schema reconciliation can run before
-- the forward migration fills existing rows. Current writers always send it.
DEFINE FIELD OVERWRITE failure ON sets TYPE option<bool>;
DEFINE FIELD OVERWRITE distance_milli ON sets TYPE option<int>;
DEFINE FIELD OVERWRITE set_time_seconds ON sets TYPE option<int>;
DEFINE FIELD OVERWRITE set_type ON sets TYPE string;
DEFINE FIELD OVERWRITE incomplete ON sets TYPE bool;

DEFINE TABLE OVERWRITE fitness_meta SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON fitness_meta TYPE string;
DEFINE FIELD OVERWRITE k ON fitness_meta TYPE string;
DEFINE FIELD OVERWRITE v ON fitness_meta TYPE int;

-- Route-free running summaries imported from Garmin Connect's public share
-- surface or entered manually as distance + elapsed time. The deterministic
-- record key is sha-256 of `<source>\n<source identity>`: Garmin supplies its
-- numeric activity id, while a manual form carries a 64-hex submission token
-- so retrying it cannot double-log a run. Garmin imports keep only a canonical
-- activity link alongside the summary. The public embed also exposes the
-- complete GPS trace and account/device metadata; those are deliberately never
-- stored. Pace remains derived from integer duration + distance.
DEFINE TABLE OVERWRITE running_activities SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON running_activities TYPE string
    ASSERT string::matches(record::id($value), '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE source ON running_activities TYPE string
    ASSERT $value IN ['garmin-connect', 'manual'];
DEFINE FIELD OVERWRITE source_activity_id ON running_activities TYPE string
    ASSERT string::matches($value, '^([0-9]{1,20}|[0-9a-f]{64})$');
DEFINE FIELD OVERWRITE source_url ON running_activities TYPE option<string>
    ASSERT $value IS NONE OR string::matches($value, '^https://connect[.]garmin[.]com/app/activity/[0-9]{1,20}$');
DEFINE FIELD OVERWRITE title ON running_activities TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 200;
DEFINE FIELD OVERWRITE activity_type ON running_activities TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 80;
DEFINE FIELD OVERWRITE started_at_utc ON running_activities TYPE string
    ASSERT string::matches($value, '^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}$');
DEFINE FIELD OVERWRITE started_at_local ON running_activities TYPE string
    ASSERT string::matches($value, '^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}$');
DEFINE FIELD OVERWRITE eastern_offset_minutes ON running_activities TYPE int
    ASSERT $value IN [-300, -240];
DEFINE FIELD OVERWRITE duration_milliseconds ON running_activities TYPE int
    ASSERT $value >= 1 AND $value <= 604800000;
DEFINE FIELD OVERWRITE moving_duration_milliseconds ON running_activities TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 1 AND $value <= 604800000);
DEFINE FIELD OVERWRITE distance_millimeters ON running_activities TYPE int
    ASSERT $value >= 1 AND $value <= 1000000000;
DEFINE FIELD OVERWRITE ascent_millimeters ON running_activities TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 100000000);
DEFINE FIELD OVERWRITE imported_at ON running_activities TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;

-- Health Connect's authoritative daily step aggregate, delivered by Health.md.
-- A day's deterministic record key makes retries idempotent; unlike immutable
-- workouts/runs, the total is intentionally replaced when a later export sees
-- a watch sync or source-priority correction. The source export watermark is
-- milliseconds since Unix epoch so a delayed older request cannot regress it.
DEFINE TABLE OVERWRITE daily_steps SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON daily_steps TYPE string
    ASSERT string::matches(record::id($value), '^\\d{4}-\\d{2}-\\d{2}$');
DEFINE FIELD OVERWRITE date ON daily_steps TYPE string
    ASSERT string::matches($value, '^\\d{4}-\\d{2}-\\d{2}$');
DEFINE FIELD OVERWRITE steps ON daily_steps TYPE int
    ASSERT $value >= 0 AND $value <= 1000000;
DEFINE FIELD OVERWRITE calendar_timezone ON daily_steps TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 100;
DEFINE FIELD OVERWRITE exported_at_ms ON daily_steps TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799999;
DEFINE FIELD OVERWRITE imported_at ON daily_steps TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;

-- The granular muscle vocabulary as data. Rust's
-- lifting::muscle_taxonomy::MUSCLE_GROUPS stays the rendering source of
-- truth (order, labels, grouping); these rows mirror it so weighted
-- connections have a referent, reconciled idempotently at snapshot load.
-- The ASSERT lists below are test-enforced against that module.
DEFINE TABLE OVERWRITE muscles SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON muscles TYPE string;
DEFINE FIELD OVERWRITE name ON muscles TYPE string
    ASSERT $value IN ['anterior-delts', 'lateral-delts', 'posterior-delts',
        'upper-traps', 'mid-traps', 'lower-traps',
        'lats', 'rhomboids', 'spinal-erectors',
        'upper-chest', 'mid-chest', 'lower-chest', 'serratus-anterior',
        'biceps', 'brachialis', 'triceps', 'forearm-flexors', 'forearm-extensors',
        'abs', 'obliques', 'hip-flexors',
        'quads', 'hamstrings', 'adductors',
        'glute-max', 'glute-med',
        'gastrocnemius', 'soleus'];
DEFINE FIELD OVERWRITE label ON muscles TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 40;
DEFINE FIELD OVERWRITE muscle_group ON muscles TYPE string
    ASSERT $value IN ['shoulders', 'traps', 'back', 'chest', 'arms',
        'core', 'legs', 'glutes', 'calves'];
DEFINE FIELD OVERWRITE ordinal ON muscles TYPE int
    ASSERT $value >= 0 AND $value <= 100;

-- One weighted exercise↔muscle connection. ratio_hundredths scales a set's
-- volume points for that muscle: 100 = full credit, 50 = half. Absence of a
-- row means no credit; zero is not storable. The record key is
-- lifting::archive::db::exercise_muscle_id(exercise_name, muscle), so the
-- pair is the identity and rewriting a ratio is an UPSERT of the same
-- record. `source` records provenance — seeding is insert-only at exercise
-- granularity and only the admin form writes 'admin' rows, which are
-- authoritative and never reseeded.
DEFINE TABLE OVERWRITE exercise_muscles SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON exercise_muscles TYPE string
    ASSERT string::matches(record::id($value), '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE exercise_name ON exercise_muscles TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 200;
DEFINE FIELD OVERWRITE muscle ON exercise_muscles TYPE string
    ASSERT $value IN ['anterior-delts', 'lateral-delts', 'posterior-delts',
        'upper-traps', 'mid-traps', 'lower-traps',
        'lats', 'rhomboids', 'spinal-erectors',
        'upper-chest', 'mid-chest', 'lower-chest', 'serratus-anterior',
        'biceps', 'brachialis', 'triceps', 'forearm-flexors', 'forearm-extensors',
        'abs', 'obliques', 'hip-flexors',
        'quads', 'hamstrings', 'adductors',
        'glute-max', 'glute-med',
        'gastrocnemius', 'soleus'];
DEFINE FIELD OVERWRITE ratio_hundredths ON exercise_muscles TYPE int
    ASSERT $value >= 1 AND $value <= 100;
DEFINE FIELD OVERWRITE source ON exercise_muscles TYPE string
    ASSERT $value IN ['seed', 'derived', 'admin'];
DEFINE FIELD OVERWRITE updated_at ON exercise_muscles TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;

-- Annotate-only gym interruptions (illness, travel, …). Inclusive Eastern
-- calendar dates plus a free-text note; never affect volume points, records,
-- or training-focus pace. Opaque string ids keep identity across edits.
DEFINE TABLE OVERWRITE fitness_interruptions SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON fitness_interruptions TYPE string
    ASSERT string::matches(record::id($value), '^[0-9a-f]{32}$');
DEFINE FIELD OVERWRITE from_date ON fitness_interruptions TYPE string
    ASSERT string::matches($value, '^\\d{4}-\\d{2}-\\d{2}$');
DEFINE FIELD OVERWRITE to_date ON fitness_interruptions TYPE option<string>
    ASSERT $value IS NONE OR string::matches($value, '^\\d{4}-\\d{2}-\\d{2}$');
DEFINE FIELD OVERWRITE note ON fitness_interruptions TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 200;
DEFINE FIELD OVERWRITE emoji ON fitness_interruptions TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 16;
DEFINE FIELD OVERWRITE updated_at ON fitness_interruptions TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;

-- Hidden-page allowlists (docs/auth.md). One row grants one email one page;
-- the record key is content::access::grant_id(page_path, email), so the pair
-- is the identity and re-granting is an UPSERT of the same record. Emails are
-- stored lowercased; the asserts mirror content::access::normalize_email and
-- plausible_grant_path.
DEFINE TABLE OVERWRITE hidden_page_grants SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON hidden_page_grants TYPE string
    ASSERT string::matches(record::id($value), '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE page_path ON hidden_page_grants TYPE string
    ASSERT string::len($value) >= 2
        AND string::len($value) <= 120
        AND string::starts_with($value, '/')
        AND !string::starts_with($value, '//')
        AND string::matches($value, '^[\\x21-\\x7e]+$');
DEFINE FIELD OVERWRITE email ON hidden_page_grants TYPE string
    ASSERT string::len($value) >= 6
        AND string::len($value) <= 254
        AND $value = string::lowercase($value)
        AND string::matches($value, '^[!-?A-~]+@[!-?A-~]+\\.[!-?A-~]+$');
DEFINE FIELD OVERWRITE granted_at ON hidden_page_grants TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;

-- Podrick, the Discord bot (docs/podrick.md). Every `podrick_*` table is
-- written only by the `podrick` binary; the site reads them to render
-- /podrick and never writes them.

-- One row per workout Podrick has taken responsibility for announcing. The
-- record key IS the workout id, so the claim is create-only: exactly one
-- worker can ever own a given workout's announcement. A row with no
-- `message_id` is a claim whose Discord POST did not confirm — the next tick
-- retries it, which is what makes a crash mid-post safe rather than a lost or
-- duplicated announcement.
DEFINE TABLE OVERWRITE podrick_announcements SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON podrick_announcements TYPE string;
DEFINE FIELD OVERWRITE workout_id ON podrick_announcements TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 128;
-- The workout's canonical public path segment, stored rather than derived so
-- the site can link an announcement without reopening the Eastern projection.
DEFINE FIELD OVERWRITE workout_path ON podrick_announcements TYPE string
    ASSERT string::matches($value,
        '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}[+-][0-9]{2}-[0-9]{2}$');
DEFINE FIELD OVERWRITE channel_id ON podrick_announcements TYPE string
    ASSERT string::matches($value, '^[0-9]{1,20}$');
DEFINE FIELD OVERWRITE message_id ON podrick_announcements TYPE option<string>
    ASSERT $value IS NONE OR string::matches($value, '^[0-9]{1,20}$');
DEFINE FIELD OVERWRITE claimed_at ON podrick_announcements TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;
DEFINE FIELD OVERWRITE posted_at ON podrick_announcements TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 253402300799);
DEFINE FIELD OVERWRITE attempts ON podrick_announcements TYPE int
    ASSERT $value >= 0 AND $value <= 1000000;

-- Source facts for Pants Off: one row per Discord message by one of the three
-- participants in #no-pants-talk. The Discord snowflake is both the record key
-- and the stable source identity. Classification is deliberately derived from
-- `posted_at` in Rust using America/New_York, so a rule correction never
-- requires rewriting history and duplicate 6:07 posts remain auditable while
-- still making one claim.
DEFINE TABLE OVERWRITE podrick_pants_messages SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON podrick_pants_messages TYPE string;
DEFINE FIELD OVERWRITE message_id ON podrick_pants_messages TYPE string
    ASSERT string::matches($value, '^[0-9]{1,20}$');
DEFINE FIELD OVERWRITE channel_id ON podrick_pants_messages TYPE string
    ASSERT string::matches($value, '^[0-9]{1,20}$');
DEFINE FIELD OVERWRITE author_id ON podrick_pants_messages TYPE string
    ASSERT string::matches($value, '^[0-9]{1,20}$');
DEFINE FIELD OVERWRITE posted_at ON podrick_pants_messages TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;

-- Retryable side effects produced from live Pants Off messages. Posts report
-- infarctions in their channel; reactions put a worm on each participant's
-- qualifying source message when a kwerm/asynkwerm completes. Deterministic
-- record keys claim an action before Discord is called. Reactions are
-- naturally idempotent PUTs, and posts carry a Discord-enforced nonce.
DEFINE TABLE OVERWRITE podrick_pants_actions SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON podrick_pants_actions TYPE string;
DEFINE FIELD OVERWRITE action_kind ON podrick_pants_actions TYPE string
    ASSERT $value IN ['post', 'reaction'];
DEFINE FIELD OVERWRITE reason ON podrick_pants_actions TYPE string
    ASSERT $value IN ['infarction', 'kwerm_am', 'kwerm_pm', 'asynkwerm'];
DEFINE FIELD OVERWRITE target_channel_id ON podrick_pants_actions TYPE string
    ASSERT string::matches($value, '^[0-9]{1,20}$');
DEFINE FIELD OVERWRITE source_message_id ON podrick_pants_actions TYPE string
    ASSERT string::matches($value, '^[0-9]{1,20}$');
DEFINE FIELD OVERWRITE content ON podrick_pants_actions TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 2000;
DEFINE FIELD OVERWRITE claimed_at ON podrick_pants_actions TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;
DEFINE FIELD OVERWRITE completed_at ON podrick_pants_actions TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 253402300799);
DEFINE FIELD OVERWRITE output_message_id ON podrick_pants_actions TYPE option<string>
    ASSERT $value IS NONE OR string::matches($value, '^[0-9]{1,20}$');
DEFINE FIELD OVERWRITE attempts ON podrick_pants_actions TYPE int
    ASSERT $value >= 0 AND $value <= 1000000;

-- String-valued cursors, keyed by `k`. `announce_watermark` holds the newest
-- workout `started_at_utc` that already existed the first time Podrick ran;
-- nothing at or before it is ever announced. That single row is what keeps
-- three years of imported history out of the channel. The `pants_*` keys hold
-- the immutable source-channel binding, history head, resumable
-- backwards-page position, and live snowflake cursor. Because every cursor
-- lives in the database rather than memory, a restart can neither replay the
-- archive nor skip the rest of a partially seeded channel.
DEFINE TABLE OVERWRITE podrick_meta SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON podrick_meta TYPE string;
DEFINE FIELD OVERWRITE k ON podrick_meta TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 64;
DEFINE FIELD OVERWRITE v ON podrick_meta TYPE string
    ASSERT string::len($value) <= 256;

-- Threaded comments on registered thoughts. Thoughts themselves remain
-- code-owned (`content::posts::POSTS`); every row carries that post's stable
-- slug so reads, replies, and tombstones stay inside one thought. Parent ids
-- are soft string references: a deleted parent remains as a tombstone, keeping
-- its descendants connected to the tree. `author_id` is the SHA-256 digest of
-- Google's stable subject claim; names are display snapshots, not identity.
DEFINE TABLE OVERWRITE thought_comments SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON thought_comments TYPE string
    ASSERT string::matches(record::id($value), '^[0-9a-f]{32}$');
DEFINE FIELD OVERWRITE thought_slug ON thought_comments TYPE string
    ASSERT string::len($value) >= 1
        AND string::len($value) <= 120
        AND string::matches($value, '^[a-z0-9]+(-[a-z0-9]+)*$');
DEFINE FIELD OVERWRITE parent_id ON thought_comments TYPE option<string>
    ASSERT $value IS NONE OR string::matches($value, '^[0-9a-f]{32}$');
DEFINE FIELD OVERWRITE author_id ON thought_comments TYPE string
    ASSERT string::matches($value, '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE author_name ON thought_comments TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1 AND string::len($value) <= 100
    );
DEFINE FIELD OVERWRITE body ON thought_comments TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1 AND string::len($value) <= 5000
    );
DEFINE FIELD OVERWRITE created_at ON thought_comments TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;
DEFINE FIELD OVERWRITE deleted_at ON thought_comments TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 253402300799);

-- A missing settings row means comments are enabled. The record key is the
-- thought slug; the duplicated field makes stored configuration inspectable
-- without depending on SurrealDB's record-id shape.
DEFINE TABLE OVERWRITE thought_comment_settings SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON thought_comment_settings TYPE string
    ASSERT string::len(record::id($value)) >= 1
        AND string::len(record::id($value)) <= 120
        AND string::matches(record::id($value), '^[a-z0-9]+(-[a-z0-9]+)*$');
DEFINE FIELD OVERWRITE thought_slug ON thought_comment_settings TYPE string
    ASSERT string::len($value) >= 1
        AND string::len($value) <= 120
        AND string::matches($value, '^[a-z0-9]+(-[a-z0-9]+)*$');
DEFINE FIELD OVERWRITE comments_enabled ON thought_comment_settings TYPE bool;
DEFINE FIELD OVERWRITE updated_at ON thought_comment_settings TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;
-- Every create increments this shared per-thought value inside its transaction.
-- That turns otherwise-disjoint comment inserts into a write conflict, making
-- the transactional count and cooldown checks authoritative under SurrealDB's
-- optimistic snapshot isolation. Admin-only settings writes leave it intact.
DEFINE FIELD OVERWRITE create_version ON thought_comment_settings TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 253402300799);
