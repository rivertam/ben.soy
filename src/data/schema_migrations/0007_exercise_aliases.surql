-- Alias lookup is unique by alternate name; canonical-name lookup powers the
-- exercise page's editor. These are forward-only because even an unchanged
-- OVERWRITE rebuilds an index on SurrealDB 3.2.3.
DEFINE INDEX IF NOT EXISTS exercise_aliases_alias_name
    ON exercise_aliases FIELDS alias_name UNIQUE;
DEFINE INDEX IF NOT EXISTS exercise_aliases_canonical_name
    ON exercise_aliases FIELDS canonical_name;

-- Rename the archive's existing pullover-crunch movement while preserving
-- the old Strong/Lyfta spelling forever. On a clean database the alias is
-- installed without creating an orphan canonical row; the first import will
-- create that row with the source exercise's taxonomy.
LET $old_name = 'Barbell Pullover Crunches';
LET $new_name = 'Barbell Resurrection Lifts';
LET $old_exists = array::len(
    SELECT VALUE name FROM exercises WHERE name = $old_name
) > 0;
LET $old_weights = SELECT muscle, ratio_hundredths, source, updated_at
    FROM exercise_muscles WHERE exercise_name = $old_name;
IF $old_exists {
    CREATE ONLY type::record('exercises', $new_name)
        CONTENT { name: $new_name } RETURN NONE;
    UPDATE sets SET exercise_name = $new_name
        WHERE exercise_name = $old_name RETURN NONE;
    UPDATE exercise_tags SET exercise_name = $new_name
        WHERE exercise_name = $old_name RETURN NONE;
    FOR $weight IN $old_weights {
        UPSERT ONLY type::record(
            'exercise_muscles',
            crypto::sha256(string::concat($new_name, '\n', $weight.muscle))
        ) CONTENT {
            id: crypto::sha256(string::concat($new_name, '\n', $weight.muscle)),
            exercise_name: $new_name,
            muscle: $weight.muscle,
            ratio_hundredths: $weight.ratio_hundredths,
            source: $weight.source,
            updated_at: $weight.updated_at
        } RETURN NONE;
    };
    DELETE exercise_muscles WHERE exercise_name = $old_name RETURN NONE;
    DELETE type::record('exercises', $old_name) RETURN NONE;
    UPSERT fitness_meta:version SET
        k = 'version', v = (v ?? 0) + 1 RETURN NONE;
};
CREATE exercise_aliases CONTENT {
    alias_name: $old_name,
    canonical_name: $new_name,
    updated_at: time::unix()
} RETURN NONE;
CREATE exercise_aliases CONTENT {
    alias_name: 'Pullover Crunches',
    canonical_name: $new_name,
    updated_at: time::unix()
} RETURN NONE;

-- The archive currently calls this movement a standing military press; accept
-- the common barbell-overhead-press spelling when it does not already own
-- history. If it is already a canonical exercise, leave both identities
-- separate so the exercise page can show the explicit merge warning first.
LET $overhead_exists = array::len(
    SELECT VALUE name FROM exercises WHERE name = 'Barbell Overhead Press'
) > 0;
IF $overhead_exists = false {
    CREATE exercise_aliases CONTENT {
        alias_name: 'Barbell Overhead Press',
        canonical_name: 'Barbell Standing Military Press',
        updated_at: time::unix()
    } RETURN NONE;
};
