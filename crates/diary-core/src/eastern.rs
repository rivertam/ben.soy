//! Compatibility re-export of the lightweight shared Eastern-time module.
//!
//! Diary Entry Keys and Fitness workout paths deliberately compile the same
//! DST-aware projection without making the Fitness wasm module depend on the
//! diary's embedded database engine.

pub use eastern_time::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_archived_workout_keeps_its_historical_projection() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/d1/workout_triples.json"
        );
        let raw = std::fs::read_to_string(path).expect("fitness projection fixture");
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let rows = parsed[0]["results"].as_array().unwrap();
        assert_eq!(rows.len(), 360, "expected the full production corpus");
        for row in rows {
            let utc = row["started_at_utc"].as_str().unwrap();
            let projected = eastern_instant(utc, 0).unwrap();
            assert_eq!(projected.local, row["started_at_local"].as_str().unwrap());
            assert_eq!(
                projected.offset_minutes,
                row["eastern_offset_minutes"].as_i64().unwrap() as i32
            );
        }
    }
}
