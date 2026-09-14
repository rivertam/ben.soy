//! Atomic owner settings, loaded in the same snapshot as the training history.

use std::collections::HashMap;

use anyhow::{Context, ensure};
use benjisponge::data::{
    Db,
    fitness_models::{MAX_MUSCLE_TARGET_CENTI_POINTS, MuscleTarget},
};

use super::super::muscle_taxonomy;

pub fn validated_map(rows: &[MuscleTarget]) -> anyhow::Result<HashMap<&'static str, u32>> {
    let mut targets = HashMap::new();
    for row in rows {
        let muscle =
            muscle_taxonomy::canonical_muscle(&row.muscle).context("unknown target muscle")?;
        ensure!(
            (0..=MAX_MUSCLE_TARGET_CENTI_POINTS).contains(&row.weekly_centi_points)
                && row.weekly_centi_points % 10 == 0,
            "target must be 0–10000 weekly points in increments of 0.1"
        );
        ensure!(
            targets
                .insert(muscle, row.weekly_centi_points as u32)
                .is_none(),
            "duplicate target muscle"
        );
    }
    Ok(targets)
}

/// The editor reads current settings directly: stale-on-error snapshots are
/// useful for public pages, but must not populate a replacement settings form.
pub async fn read(db: &Db) -> anyhow::Result<Vec<MuscleTarget>> {
    let mut response = db
        .query("SELECT record::id(id) AS muscle, weekly_centi_points FROM fitness_muscle_targets")
        .await?
        .check()?;
    let rows: Vec<MuscleTarget> = response.take(0)?;
    validated_map(&rows)?;
    Ok(rows)
}

/// Replace the complete settings form. An empty list deliberately clears all
/// goals; a zero-valued row remains present and never falls back to usual.
pub async fn save(db: &Db, rows: &[MuscleTarget], now: i64) -> anyhow::Result<()> {
    validated_map(rows)?;
    for attempt in 0..3 {
        let result = db
            .query(
                "BEGIN TRANSACTION;
                 DELETE fitness_muscle_targets RETURN NONE;
                 FOR $target IN $targets {
                     CREATE ONLY type::record('fitness_muscle_targets', $target.muscle)
                         SET weekly_centi_points = $target.weekly_centi_points,
                             updated_at = $now RETURN NONE;
                 };
                 UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1 RETURN NONE;
                 COMMIT TRANSACTION;",
            )
            .bind(("targets", rows.to_vec()))
            .bind(("now", now))
            .await
            .map_err(|error| error.to_string())
            .and_then(|mut response| {
                // Inspect every statement: cancellation errors can precede
                // the actual retryable transaction conflict.
                let errors = response.take_errors();
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors
                        .values()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "))
                }
            });
        match result {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 2 && error.contains("Transaction conflict:") => continue,
            Err(error) => anyhow::bail!(error),
        }
    }
    unreachable!("the final attempt returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interests::lifting::archive::{db, snapshot};

    fn target(muscle: &str, weekly_centi_points: i64) -> MuscleTarget {
        MuscleTarget {
            muscle: muscle.into(),
            weekly_centi_points,
        }
    }

    async fn database() -> Db {
        let db = surrealdb::engine::any::connect("mem://").await.unwrap();
        db.use_ns("targets").use_db("test").await.unwrap();
        db.query(include_str!("../../../../schema.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        db
    }

    #[tokio::test]
    async fn goals_round_trip_with_snapshot_version_and_clear_without_reseeding() {
        let db = database().await;
        assert!(read(&db).await.unwrap().is_empty());
        save(&db, &[target("biceps", 1250), target("quads", 0)], 1)
            .await
            .unwrap();
        let (
            version,
            workouts,
            sets,
            aliases,
            tags,
            weights,
            interruptions,
            exercises,
            references,
            targets,
        ) = db::load_archive(&db).await.unwrap();
        assert_eq!(version, 1);
        let snapshot = snapshot::build(
            version,
            workouts,
            sets,
            aliases,
            tags,
            weights,
            interruptions,
        )
        .unwrap()
        .with_catalog(exercises, references)
        .with_muscle_targets(targets)
        .unwrap();
        assert_eq!(snapshot.muscle_targets().get("biceps"), Some(&1250));
        assert_eq!(snapshot.muscle_targets().get("quads"), Some(&0));
        let focus = snapshot.training_focus("2026-09-12".parse().unwrap());
        assert_eq!(focus.recommendation.unwrap().muscle_id, "biceps");

        save(&db, &[target("abs", 1000)], 2).await.unwrap();
        assert_eq!(read(&db).await.unwrap(), [target("abs", 1000)]);
        assert_eq!(db::current_version(&db).await.unwrap(), 2);
        db.query(include_str!("../../../../schema.surql"))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(
            read(&db).await.unwrap(),
            [target("abs", 1000)],
            "schema bootstrap preserves settings"
        );
        save(&db, &[], 3).await.unwrap();
        db::reconcile_muscle_weights(&db, 4).await.unwrap();
        assert!(read(&db).await.unwrap().is_empty());
        assert_eq!(db::current_version(&db).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn failures_roll_back_settings_and_version_and_schema_enforces_bounds() {
        let db = database().await;
        save(&db, &[target("biceps", 1250)], 1).await.unwrap();
        assert!(save(&db, &[target("abs", 1000)], -1).await.is_err());
        assert_eq!(read(&db).await.unwrap(), [target("biceps", 1250)]);
        assert_eq!(db::current_version(&db).await.unwrap(), 1);
        for (muscle, points) in [
            ("chest", 100),
            ("abs", -10),
            ("abs", 1_000_010),
            ("abs", 11),
        ] {
            assert!(db.query("CREATE ONLY type::record('fitness_muscle_targets', $muscle) SET weekly_centi_points = $points, updated_at = 1")
                .bind(("muscle", muscle)).bind(("points", points))
                .await.unwrap().check().is_err(), "accepted {muscle}: {points}");
        }
        assert!(validated_map(&[target("abs", 100), target("abs", 200)]).is_err());
    }

    #[tokio::test]
    async fn concurrent_saves_commit_complete_forms_and_each_bump_the_version() {
        let db = database().await;
        let a = [target("abs", 1000), target("biceps", 2000)];
        let b = [target("quads", 3000)];
        let (first, second) = tokio::join!(save(&db, &a, 1), save(&db, &b, 2));
        first.unwrap();
        second.unwrap();
        assert_eq!(db::current_version(&db).await.unwrap(), 2);
        let rows = read(&db).await.unwrap();
        assert!(validated_map(&rows).unwrap() == validated_map(&a).unwrap() || rows == b);
    }
}
