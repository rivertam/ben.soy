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
DEFINE FIELD OVERWRITE distance_milli ON sets TYPE option<int>;
DEFINE FIELD OVERWRITE set_time_seconds ON sets TYPE option<int>;
DEFINE FIELD OVERWRITE set_type ON sets TYPE string;
DEFINE FIELD OVERWRITE incomplete ON sets TYPE bool;

DEFINE TABLE OVERWRITE fitness_meta SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON fitness_meta TYPE string;
DEFINE FIELD OVERWRITE k ON fitness_meta TYPE string;
DEFINE FIELD OVERWRITE v ON fitness_meta TYPE int;

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

DEFINE TABLE OVERWRITE analytics_events SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON analytics_events TYPE string
    ASSERT string::matches(record::id($value), '^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$');
DEFINE FIELD OVERWRITE visitor_id ON analytics_events TYPE string
    ASSERT string::matches($value, '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE session_id ON analytics_events TYPE string
    ASSERT string::matches($value, '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE occurred_at ON analytics_events TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;
DEFINE FIELD OVERWRITE kind ON analytics_events TYPE string
    ASSERT $value IN ['pageview', 'engagement', 'outbound']
        AND (
            ($value = 'outbound' AND target_host IS NOT NONE)
            OR ($value != 'outbound' AND target_host IS NONE)
        );
DEFINE FIELD OVERWRITE page_path ON analytics_events TYPE string
    ASSERT string::len($value) >= 1
        AND string::len($value) <= 300
        AND string::starts_with($value, '/')
        AND !string::starts_with($value, '//')
        AND !string::contains($value, '?')
        AND !string::contains($value, '#')
        AND !string::contains($value, '\\')
        AND string::matches($value, '^[^\\x00-\\x1F\\x7F]*$')
        AND !string::starts_with($value, '/api/')
        AND !string::starts_with($value, '/_topcoat/');
DEFINE FIELD OVERWRITE referrer_kind ON analytics_events TYPE string
    ASSERT $value IN ['direct', 'internal', 'search', 'social', 'ai', 'referral']
        AND (
            ($value = 'direct'
                AND referrer_host IS NONE
                AND referrer_path IS NONE)
            OR ($value = 'internal' AND referrer_host IS NONE)
            OR ($value IN ['search', 'social', 'ai', 'referral']
                AND referrer_host IS NOT NONE
                AND referrer_path IS NONE)
        );
DEFINE FIELD OVERWRITE referrer_host ON analytics_events TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1
        AND string::len($value) <= 253
        AND $value = string::lowercase($value)
        AND string::matches($value, '^[^\\s/?#]+$')
    );
DEFINE FIELD OVERWRITE referrer_path ON analytics_events TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1
        AND string::len($value) <= 300
        AND string::starts_with($value, '/')
        AND !string::starts_with($value, '//')
        AND !string::contains($value, '?')
        AND !string::contains($value, '#')
        AND !string::contains($value, '\\')
        AND string::matches($value, '^[^\\x00-\\x1F\\x7F]*$')
    );
DEFINE FIELD OVERWRITE country_code ON analytics_events TYPE option<string>
    ASSERT $value IS NONE OR string::matches($value, '^[A-Z]{2}$');
DEFINE FIELD OVERWRITE timezone ON analytics_events TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1
        AND string::len($value) <= 64
        AND string::matches($value, '^[A-Za-z0-9_/+-]+$')
    );
DEFINE FIELD OVERWRITE language ON analytics_events TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1
        AND string::len($value) <= 24
        AND string::matches($value, '^[A-Za-z0-9-]+$')
    );
DEFINE FIELD OVERWRITE device_kind ON analytics_events TYPE string
    ASSERT $value IN ['desktop', 'mobile', 'tablet', 'unknown'];
DEFINE FIELD OVERWRITE browser ON analytics_events TYPE string
    ASSERT $value IN ['Edge', 'Firefox', 'Opera', 'Chrome', 'Safari', 'Other'];
DEFINE FIELD OVERWRITE operating_system ON analytics_events TYPE string
    ASSERT $value IN ['iOS', 'Android', 'Windows', 'macOS', 'Linux', 'Other'];
DEFINE FIELD OVERWRITE viewport_kind ON analytics_events TYPE string
    ASSERT $value IN ['phone', 'tablet', 'desktop', 'wide', 'unknown'];
DEFINE FIELD OVERWRITE navigation_kind ON analytics_events TYPE option<string>
    ASSERT $value IS NONE
        OR $value IN ['navigate', 'reload', 'back_forward', 'prerender'];
DEFINE FIELD OVERWRITE local_hour ON analytics_events TYPE option<int>
    ASSERT (
        $value IS NONE AND local_weekday IS NONE
    ) OR (
        $value >= 0 AND $value <= 23
        AND local_weekday >= 0 AND local_weekday <= 6
    );
DEFINE FIELD OVERWRITE local_weekday ON analytics_events TYPE option<int>
    ASSERT $value IS NONE OR (
        $value >= 0 AND $value <= 6 AND local_hour IS NOT NONE
    );
DEFINE FIELD OVERWRITE engagement_seconds ON analytics_events TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 7200);
DEFINE FIELD OVERWRITE scroll_percent ON analytics_events TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 100);
DEFINE FIELD OVERWRITE lcp_milliseconds ON analytics_events TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 120000);
DEFINE FIELD OVERWRITE cls_thousandths ON analytics_events TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 100000);
DEFINE FIELD OVERWRITE navigation_milliseconds ON analytics_events TYPE option<int>
    ASSERT $value IS NONE OR ($value >= 0 AND $value <= 120000);
DEFINE FIELD OVERWRITE target_host ON analytics_events TYPE option<string>
    ASSERT (
        (kind = 'outbound' AND $value IS NOT NONE)
        OR (kind != 'outbound' AND $value IS NONE)
    ) AND (
        $value IS NONE OR (
            string::len($value) >= 1
            AND string::len($value) <= 253
            AND $value = string::lowercase($value)
            AND string::matches($value, '^[^\\s/?#]+$')
        )
    );
DEFINE FIELD OVERWRITE utm_source ON analytics_events TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1
        AND string::len($value) <= 80
        AND string::matches($value, '^[^\\x00-\\x1F\\x7F]*$')
    );
DEFINE FIELD OVERWRITE utm_medium ON analytics_events TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1
        AND string::len($value) <= 80
        AND string::matches($value, '^[^\\x00-\\x1F\\x7F]*$')
    );
DEFINE FIELD OVERWRITE utm_campaign ON analytics_events TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1
        AND string::len($value) <= 80
        AND string::matches($value, '^[^\\x00-\\x1F\\x7F]*$')
    );

DEFINE TABLE OVERWRITE analytics_visitor_aliases SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON analytics_visitor_aliases TYPE string
    ASSERT string::matches(record::id($value), '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE visitor_id ON analytics_visitor_aliases TYPE string
    ASSERT string::matches($value, '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE created_at ON analytics_visitor_aliases TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;

DEFINE TABLE OVERWRITE analytics_sessions SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON analytics_sessions TYPE string
    ASSERT string::matches(record::id($value), '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE session_id ON analytics_sessions TYPE string
    ASSERT string::matches($value, '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE last_seen_at ON analytics_sessions TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;

DEFINE TABLE OVERWRITE analytics_identities SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE id ON analytics_identities TYPE string
    ASSERT string::matches(record::id($value), '^[0-9a-f]{64}$');
DEFINE FIELD OVERWRITE display_name ON analytics_identities TYPE string
    ASSERT string::len($value) >= 1 AND string::len($value) <= 80
        AND $value = string::trim($value)
        AND string::matches($value, '^[^\\x00-\\x1F\\x7F]*$');
DEFINE FIELD OVERWRITE note ON analytics_identities TYPE option<string>
    ASSERT $value IS NONE OR (
        string::len($value) >= 1
        AND string::len($value) <= 400
        AND $value = string::trim($value)
        AND string::matches($value, '^[^\\x00-\\x1F\\x7F]*$')
    );
DEFINE FIELD OVERWRITE first_submitted_at ON analytics_identities TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799;
DEFINE FIELD OVERWRITE updated_at ON analytics_identities TYPE int
    ASSERT $value >= 0 AND $value <= 253402300799
        AND $value >= first_submitted_at;

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
