DEFINE INDEX IF NOT EXISTS running_activities_source_identity
    ON running_activities FIELDS source, source_activity_id UNIQUE;
DEFINE INDEX IF NOT EXISTS running_activities_started_at_utc
    ON running_activities FIELDS started_at_utc;
