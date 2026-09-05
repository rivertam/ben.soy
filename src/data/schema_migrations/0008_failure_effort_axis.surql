-- Failure used to be one of the set types even though it is an effort
-- outcome. Move it to its own mutually-exclusive effort marker. When a
-- legacy failure row also carries RPE, failure wins and the numeric value is
-- cleared; attempting to reconcile two descriptions of the same effort would
-- invent data that the source never supplied.
LET $legacy_failures = array::len(
    SELECT VALUE record::id(id) FROM sets WHERE set_type = 'FAILURE_SET'
);
UPDATE sets SET failure = false
    WHERE set_type != 'FAILURE_SET' RETURN NONE;
UPDATE sets SET
    failure = true,
    effort_hundredths = NONE,
    set_type = 'NORMAL_SET'
    WHERE set_type = 'FAILURE_SET' RETURN NONE;
IF $legacy_failures > 0 {
    UPSERT fitness_meta:version SET
        k = 'version', v = (v ?? 0) + 1 RETURN NONE;
};
