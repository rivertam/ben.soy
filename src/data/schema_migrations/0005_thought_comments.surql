-- Whole comment trees load by thought and order chronologically. Keep this in
-- the forward-only ledger: DEFINE INDEX OVERWRITE rebuilds the physical index
-- on SurrealDB 3.2.3 even when its definition has not changed.
DEFINE INDEX IF NOT EXISTS thought_comments_thought_created_at
    ON thought_comments FIELDS thought_slug, created_at;
