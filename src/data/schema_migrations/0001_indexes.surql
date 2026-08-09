-- Adopt the site's existing indexes without replacing them. On a fresh
-- database these build before records are imported; on an existing database
-- every statement is a metadata-only no-op. Later index changes belong in a
-- new migration and may use OVERWRITE deliberately.
DEFINE INDEX IF NOT EXISTS spire_runs_start_time
    ON spire_runs FIELDS start_time;
DEFINE INDEX IF NOT EXISTS workouts_started_at_utc
    ON workouts FIELDS started_at_utc;
DEFINE INDEX IF NOT EXISTS exercises_name
    ON exercises FIELDS name UNIQUE;
DEFINE INDEX IF NOT EXISTS exercise_tags_identity
    ON exercise_tags FIELDS exercise_name, kind, value UNIQUE;
DEFINE INDEX IF NOT EXISTS sets_workout_ordinal
    ON sets FIELDS workout_id, ordinal UNIQUE;
DEFINE INDEX IF NOT EXISTS sets_workout_id
    ON sets FIELDS workout_id;
DEFINE INDEX IF NOT EXISTS muscles_name
    ON muscles FIELDS name UNIQUE;
DEFINE INDEX IF NOT EXISTS exercise_muscles_identity
    ON exercise_muscles FIELDS exercise_name, muscle UNIQUE;
DEFINE INDEX IF NOT EXISTS hidden_page_grants_identity
    ON hidden_page_grants FIELDS page_path, email UNIQUE;
DEFINE INDEX IF NOT EXISTS analytics_events_kind_time
    ON analytics_events FIELDS kind, occurred_at;
DEFINE INDEX IF NOT EXISTS analytics_events_time
    ON analytics_events FIELDS occurred_at;
DEFINE INDEX IF NOT EXISTS analytics_events_session
    ON analytics_events FIELDS visitor_id, session_id, occurred_at;
DEFINE INDEX IF NOT EXISTS analytics_events_session_time
    ON analytics_events FIELDS session_id, occurred_at;
DEFINE INDEX IF NOT EXISTS podrick_announcements_claimed_at
    ON podrick_announcements FIELDS claimed_at;
DEFINE INDEX IF NOT EXISTS podrick_pants_messages_posted_at
    ON podrick_pants_messages FIELDS posted_at;
DEFINE INDEX IF NOT EXISTS podrick_pants_messages_author_posted_at
    ON podrick_pants_messages FIELDS author_id, posted_at;
DEFINE INDEX IF NOT EXISTS podrick_pants_actions_claimed_at
    ON podrick_pants_actions FIELDS claimed_at;
