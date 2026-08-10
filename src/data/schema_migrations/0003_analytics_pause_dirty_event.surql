-- Pause the per-write dirty marker while fact backfill is opt-in and idle.
-- Every beacon was UPSERT-ing analytics_fact_dirty; under load that amplified
-- websocket resets. Reintroduce the event in a later epoch when deliberately
-- finishing the visitor-day rollout (ANALYTICS_FACTS_BACKFILL=1).
REMOVE EVENT IF EXISTS analytics_events_rebuild_visitor_day ON TABLE analytics_events;
