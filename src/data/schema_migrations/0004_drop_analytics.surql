-- Tear down the retired first-party analytics stack. IF EXISTS keeps this
-- safe on fresh databases that never defined these objects, and on production
-- databases that already applied epochs 2–3 under the old definitions.
REMOVE EVENT IF EXISTS analytics_events_rebuild_visitor_day ON TABLE analytics_events;
REMOVE FUNCTION IF EXISTS fn::analytics::rebuild_visitor_day;
REMOVE INDEX IF EXISTS analytics_events_kind_time ON TABLE analytics_events;
REMOVE INDEX IF EXISTS analytics_events_time ON TABLE analytics_events;
REMOVE INDEX IF EXISTS analytics_events_session ON TABLE analytics_events;
REMOVE INDEX IF EXISTS analytics_events_session_time ON TABLE analytics_events;
REMOVE INDEX IF EXISTS analytics_events_visitor_time ON TABLE analytics_events;
REMOVE INDEX IF EXISTS analytics_visitor_days_day ON TABLE analytics_visitor_days;
REMOVE TABLE IF EXISTS analytics_events;
REMOVE TABLE IF EXISTS analytics_visitor_days;
REMOVE TABLE IF EXISTS analytics_fact_backfill;
REMOVE TABLE IF EXISTS analytics_fact_dirty;
REMOVE TABLE IF EXISTS analytics_visitor_aliases;
REMOVE TABLE IF EXISTS analytics_sessions;
REMOVE TABLE IF EXISTS analytics_identities;
