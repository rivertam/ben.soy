-- Exact visitor/day read model and the raw lookup used by absolute rebuilds.
DEFINE INDEX IF NOT EXISTS analytics_visitor_days_day
    ON analytics_visitor_days FIELDS utc_day;
DEFINE INDEX IF NOT EXISTS analytics_events_visitor_time
    ON analytics_events FIELDS visitor_id, occurred_at;

-- The application supplies a versioned, absolute payload computed from all
-- raw rows for this visitor/day. Replacing the whole record makes retries and
-- concurrent delivery idempotent; no delta is ever applied.
DEFINE FUNCTION fn::analytics::rebuild_visitor_day(
    $utc_day: int,
    $visitor_id: string,
    $payload: none | string,
    $rebuilt_at: int,
    $revision: none | int
) {
    IF $payload = NONE {
        UPSERT type::record('analytics_fact_dirty', crypto::sha256(
            <string>$utc_day + ':' + $visitor_id
        )) CONTENT {
            utc_day: $utc_day,
            visitor_id: $visitor_id,
            touched_at: $rebuilt_at,
            revision: (revision ?? 0) + 1
        };
        RETURN false;
    };
    LET $dirty = SELECT VALUE revision FROM analytics_fact_dirty
        WHERE utc_day = $utc_day AND visitor_id = $visitor_id;
    IF ($revision = 0 AND array::len($dirty) = 0)
        OR array::first($dirty) = $revision {
        UPSERT type::record('analytics_visitor_days', crypto::sha256(
            <string>$utc_day + ':' + $visitor_id
        )) CONTENT {
            utc_day: $utc_day,
            visitor_id: $visitor_id,
            payload: $payload,
            rebuilt_at: $rebuilt_at
        };
        DELETE analytics_fact_dirty WHERE utc_day = $utc_day
            AND visitor_id = $visitor_id AND revision = $revision;
        RETURN true;
    };
    RETURN false;
};

-- CREATE and UPDATE both run this event. It records only the affected key;
-- deletes are intentionally ignored so later raw retention cannot subtract
-- already-retained exact facts.
DEFINE EVENT analytics_events_rebuild_visitor_day ON TABLE analytics_events
WHEN $event IN ['CREATE', 'UPDATE'] THEN {
    fn::analytics::rebuild_visitor_day(
        <int>($after.occurred_at / 86400),
        $after.visitor_id,
        NONE,
        time::unix(),
        NONE
    );
};
