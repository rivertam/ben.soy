//! Health Connect daily-step ingestion and direct reads.
//!
//! Health.md is the Android bridge: its compatibility API export contains one
//! Health Connect aggregate per calendar day. Unlike lifting and running
//! history, a daily total is expected to change when a watch syncs late, so
//! the deterministic date row is an authoritative upsert. `exported_at_ms`
//! prevents a delayed retry from putting an older total back over a newer one.

use std::collections::{HashMap, HashSet};

use benjisponge::data::{Data, Db};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

pub const BODY_LIMIT_BYTES: usize = 256 * 1024;
pub const RECENT_DAYS_LIMIT: usize = 35;
const MAX_DAILY_RECORDS: usize = 400;
const MAX_STEPS_PER_DAY: i64 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepPayload {
    exported_at_ms: i64,
    received: usize,
    omitted: usize,
    failed_dates: usize,
    days: Vec<IncomingStepDay>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IncomingStepDay {
    date: String,
    steps: i64,
    calendar_timezone: String,
}

#[derive(Debug, Deserialize)]
struct HealthMdEnvelope {
    schema: String,
    schema_version: u32,
    daily_record_schema: String,
    daily_record_schema_version: u32,
    exported_at: String,
    source: String,
    date_range: HealthMdDateRange,
    record_count: usize,
    records: Vec<HealthMdDailyRecord>,
    failed_date_details: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct HealthMdDateRange {
    start: String,
    end: String,
}

#[derive(Debug, Deserialize)]
struct HealthMdDailyRecord {
    date: String,
    #[serde(rename = "type")]
    kind: String,
    schema: String,
    schema_version: u32,
    time_context: HealthMdTimeContext,
    #[serde(default)]
    activity: Option<HealthMdActivity>,
}

#[derive(Debug, Deserialize)]
struct HealthMdTimeContext {
    calendar_timezone: String,
}

#[derive(Debug, Deserialize)]
struct HealthMdActivity {
    #[serde(default)]
    steps: Option<i64>,
}

/// The intentionally small public projection. Import timestamps and source
/// timezone stay internal; the public API exposes only the same daily fact the
/// page renders.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue, PartialEq, Eq)]
pub struct StepDay {
    pub date: String,
    pub steps: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue, PartialEq, Eq)]
struct StoredStepDay {
    date: String,
    steps: i64,
    calendar_timezone: String,
    exported_at_ms: i64,
    imported_at: i64,
}

#[derive(Clone, Debug, Deserialize, SurrealValue, PartialEq, Eq)]
struct ExistingStepDay {
    date: String,
    steps: i64,
    calendar_timezone: String,
    exported_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StepSeries {
    pub days: Vec<StepDay>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct StepImportReceipt {
    pub received: usize,
    pub accepted: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub stale: usize,
    pub omitted: usize,
    pub failed_dates: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepImportOutcome {
    pub receipt: StepImportReceipt,
}

pub fn parse_healthmd_payload(value: &serde_json::Value) -> Result<StepPayload, String> {
    let envelope: HealthMdEnvelope = serde_json::from_value(value.clone())
        .map_err(|_| "body must match the Health.md compatibility API export".to_string())?;

    if envelope.schema != "healthmd.api_export" || envelope.schema_version != 1 {
        return Err("unsupported Health.md API export schema".to_string());
    }
    if envelope.daily_record_schema != "healthmd.health_data"
        || envelope.daily_record_schema_version != 4
    {
        return Err("unsupported Health.md daily record schema".to_string());
    }
    if envelope.source != "android" {
        return Err("Health.md export source must be android".to_string());
    }
    if envelope.record_count != envelope.records.len() {
        return Err("record_count does not match records".to_string());
    }
    if envelope.records.len() > MAX_DAILY_RECORDS
        || envelope.failed_date_details.len() > MAX_DAILY_RECORDS
    {
        return Err(format!(
            "export may contain at most {MAX_DAILY_RECORDS} records or failed dates"
        ));
    }

    let exported_at_ms = envelope
        .exported_at
        .parse::<jiff::Timestamp>()
        .map_err(|_| "exported_at must be an RFC 3339 timestamp".to_string())?
        .as_millisecond();
    if exported_at_ms < 0 {
        return Err("exported_at must not predate 1970".to_string());
    }

    validate_iso_date(&envelope.date_range.start)
        .ok_or_else(|| "date_range.start must be YYYY-MM-DD".to_string())?;
    validate_iso_date(&envelope.date_range.end)
        .ok_or_else(|| "date_range.end must be YYYY-MM-DD".to_string())?;
    if envelope.date_range.start > envelope.date_range.end {
        return Err("date_range.start must not be after date_range.end".to_string());
    }

    let mut seen = HashSet::with_capacity(envelope.records.len());
    let mut days = Vec::with_capacity(envelope.records.len());
    let mut omitted = 0;
    for record in envelope.records {
        if record.schema != "healthmd.health_data" || record.schema_version != 4 {
            return Err("record uses an unsupported Health.md daily schema".to_string());
        }
        if record.kind != "health-data" {
            return Err("record type must be health-data".to_string());
        }
        validate_iso_date(&record.date)
            .ok_or_else(|| "record date must be YYYY-MM-DD".to_string())?;
        if record.date < envelope.date_range.start || record.date > envelope.date_range.end {
            return Err("record date falls outside date_range".to_string());
        }
        if !seen.insert(record.date.clone()) {
            return Err("records contain a duplicate date".to_string());
        }
        validate_timezone(&record.time_context.calendar_timezone)?;

        let Some(steps) = record.activity.and_then(|activity| activity.steps) else {
            omitted += 1;
            continue;
        };
        if !(0..=MAX_STEPS_PER_DAY).contains(&steps) {
            return Err(format!(
                "record steps must be between 0 and {MAX_STEPS_PER_DAY}"
            ));
        }
        days.push(IncomingStepDay {
            date: record.date,
            steps,
            calendar_timezone: record.time_context.calendar_timezone,
        });
    }

    Ok(StepPayload {
        exported_at_ms,
        received: envelope.record_count,
        omitted,
        failed_dates: envelope.failed_date_details.len(),
        days,
    })
}

fn validate_iso_date(value: &str) -> Option<jiff::civil::Date> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return None;
    }
    let year = value[..4].parse().ok()?;
    let month = value[5..7].parse().ok()?;
    let day = value[8..].parse().ok()?;
    jiff::civil::Date::new(year, month, day).ok()
}

fn validate_timezone(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'\\')
    {
        return Err("record calendar timezone is invalid".to_string());
    }
    Ok(())
}

pub async fn load(data: &Data, limit: usize) -> anyhow::Result<Vec<StepDay>> {
    let db = data.db().await?;
    Ok(load_recent(&db, limit).await?)
}

pub async fn load_recent(db: &Db, limit: usize) -> surrealdb::Result<Vec<StepDay>> {
    let limit = i64::try_from(limit.min(RECENT_DAYS_LIMIT)).unwrap_or(RECENT_DAYS_LIMIT as i64);
    let mut response = db
        .query(
            "SELECT date, steps
             FROM daily_steps
             ORDER BY date DESC
             LIMIT $limit;",
        )
        .bind(("limit", limit))
        .await?
        .check()?;
    response.take(0)
}

pub async fn apply_import(
    db: &Db,
    payload: &StepPayload,
    imported_at: i64,
) -> surrealdb::Result<StepImportOutcome> {
    if payload.days.is_empty() {
        return Ok(StepImportOutcome {
            receipt: StepImportReceipt {
                received: payload.received,
                accepted: 0,
                added: 0,
                updated: 0,
                unchanged: 0,
                stale: 0,
                omitted: payload.omitted,
                failed_dates: payload.failed_dates,
            },
        });
    }

    let dates: Vec<String> = payload.days.iter().map(|day| day.date.clone()).collect();
    let mut response = db
        .query(
            "SELECT date, steps, calendar_timezone, exported_at_ms
             FROM daily_steps
             WHERE date IN $dates;",
        )
        .bind(("dates", dates))
        .await?
        .check()?;
    let existing: HashMap<String, ExistingStepDay> = response
        .take::<Vec<ExistingStepDay>>(0)?
        .into_iter()
        .map(|row| (row.date.clone(), row))
        .collect();

    let mut rows = Vec::with_capacity(payload.days.len());
    let mut added = 0;
    let mut updated = 0;
    let mut unchanged = 0;
    let mut stale = 0;
    for day in &payload.days {
        match existing.get(&day.date) {
            None => added += 1,
            Some(stored) if payload.exported_at_ms <= stored.exported_at_ms => {
                if payload.exported_at_ms == stored.exported_at_ms
                    && day.steps == stored.steps
                    && day.calendar_timezone == stored.calendar_timezone
                {
                    unchanged += 1;
                } else {
                    stale += 1;
                }
                continue;
            }
            Some(stored)
                if day.steps == stored.steps
                    && day.calendar_timezone == stored.calendar_timezone =>
            {
                // Advance the source watermark even though the public fact is
                // unchanged. Otherwise an older delayed request could still
                // replace it later.
                unchanged += 1;
            }
            Some(_) => updated += 1,
        }
        rows.push(StoredStepDay {
            date: day.date.clone(),
            steps: day.steps,
            calendar_timezone: day.calendar_timezone.clone(),
            exported_at_ms: payload.exported_at_ms,
            imported_at,
        });
    }

    if !rows.is_empty() {
        // Recheck the watermark inside the transaction as well as above. If
        // two requests race, the older transaction either sees the winner or
        // conflicts; it never deliberately overwrites a newer export.
        db.query(
            "BEGIN TRANSACTION;
             FOR $row IN $rows {
                 LET $stored_exported_at_ms = (
                     SELECT VALUE exported_at_ms
                     FROM type::record('daily_steps', $row.date)
                 )[0] ?? -1;
                 IF $stored_exported_at_ms < $row.exported_at_ms {
                     UPSERT type::record('daily_steps', $row.date) CONTENT $row;
                 };
             };
             COMMIT TRANSACTION;",
        )
        .bind(("rows", rows))
        .await?
        .check()?;
    }

    Ok(StepImportOutcome {
        receipt: StepImportReceipt {
            received: payload.received,
            accepted: payload.days.len(),
            added,
            updated,
            unchanged,
            stale,
            omitted: payload.omitted,
            failed_dates: payload.failed_dates,
        },
    })
}

#[cfg(test)]
mod tests {
    use surrealdb::engine::any;

    use super::*;

    const TEST_SCHEMA: &str = include_str!("../../../../schema.surql");

    fn envelope(records: serde_json::Value, exported_at: &str) -> serde_json::Value {
        serde_json::json!({
            "schema": "healthmd.api_export",
            "schema_version": 1,
            "daily_record_schema": "healthmd.health_data",
            "daily_record_schema_version": 4,
            "exported_at": exported_at,
            "source": "android",
            "date_range": { "start": "2026-08-20", "end": "2026-08-21" },
            "record_count": records.as_array().unwrap().len(),
            "records": records,
            "failed_date_details": []
        })
    }

    fn record(date: &str, steps: Option<i64>) -> serde_json::Value {
        let activity = steps.map_or_else(
            || serde_json::json!({}),
            |steps| serde_json::json!({ "steps": steps }),
        );
        serde_json::json!({
            "date": date,
            "type": "health-data",
            "schema": "healthmd.health_data",
            "schema_version": 4,
            "time_context": {
                "calendar_timezone": "America/New_York",
                "timestamp_timezone": "UTC"
            },
            "unit_system": "metric",
            "units": { "steps": "count" },
            "activity": activity
        })
    }

    async fn database() -> Db {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns("fitness").use_db("fitness").await.unwrap();
        db.query(TEST_SCHEMA).await.unwrap().check().unwrap();
        db
    }

    #[test]
    fn parses_steps_and_preserves_partial_export_counts() {
        let mut value = envelope(
            serde_json::json!([
                record("2026-08-20", Some(12_345)),
                record("2026-08-21", None)
            ]),
            "2026-08-22T12:34:56.789Z",
        );
        value["failed_date_details"] = serde_json::json!([{
            "date": "2026-08-19T04:00:00Z",
            "reason": "device_locked"
        }]);

        let payload = parse_healthmd_payload(&value).unwrap();
        assert_eq!(payload.exported_at_ms, 1_787_402_096_789);
        assert_eq!(payload.received, 2);
        assert_eq!(payload.omitted, 1);
        assert_eq!(payload.failed_dates, 1);
        assert_eq!(
            payload.days,
            vec![IncomingStepDay {
                date: "2026-08-20".into(),
                steps: 12_345,
                calendar_timezone: "America/New_York".into(),
            }]
        );
    }

    #[test]
    fn rejects_ambiguous_or_out_of_contract_records() {
        let duplicate = envelope(
            serde_json::json!([record("2026-08-20", Some(1)), record("2026-08-20", Some(2))]),
            "2026-08-22T12:00:00Z",
        );
        assert_eq!(
            parse_healthmd_payload(&duplicate).unwrap_err(),
            "records contain a duplicate date"
        );

        let mut wrong_count = envelope(
            serde_json::json!([record("2026-08-20", Some(1))]),
            "2026-08-22T12:00:00Z",
        );
        wrong_count["record_count"] = serde_json::json!(2);
        assert_eq!(
            parse_healthmd_payload(&wrong_count).unwrap_err(),
            "record_count does not match records"
        );

        let too_many = envelope(
            serde_json::json!([record("2026-08-20", Some(1_000_001))]),
            "2026-08-22T12:00:00Z",
        );
        assert!(
            parse_healthmd_payload(&too_many)
                .unwrap_err()
                .contains("record steps must be between")
        );
    }

    #[tokio::test]
    async fn newer_exports_upsert_and_older_exports_cannot_regress_a_day() {
        let db = database().await;
        let first = parse_healthmd_payload(&envelope(
            serde_json::json!([record("2026-08-20", Some(10_000))]),
            "2026-08-22T10:00:00Z",
        ))
        .unwrap();
        let outcome = apply_import(&db, &first, 1).await.unwrap();
        assert_eq!(outcome.receipt.added, 1);

        let same = parse_healthmd_payload(&envelope(
            serde_json::json!([record("2026-08-20", Some(10_000))]),
            "2026-08-22T11:00:00Z",
        ))
        .unwrap();
        let outcome = apply_import(&db, &same, 2).await.unwrap();
        assert_eq!(outcome.receipt.unchanged, 1);

        let stale = parse_healthmd_payload(&envelope(
            serde_json::json!([record("2026-08-20", Some(9_000))]),
            "2026-08-22T10:30:00Z",
        ))
        .unwrap();
        let outcome = apply_import(&db, &stale, 3).await.unwrap();
        assert_eq!(outcome.receipt.stale, 1);

        let correction = parse_healthmd_payload(&envelope(
            serde_json::json!([record("2026-08-20", Some(10_250))]),
            "2026-08-22T12:00:00Z",
        ))
        .unwrap();
        let outcome = apply_import(&db, &correction, 4).await.unwrap();
        assert_eq!(outcome.receipt.updated, 1);
        assert_eq!(
            load_recent(&db, 35).await.unwrap(),
            vec![StepDay {
                date: "2026-08-20".into(),
                steps: 10_250,
            }]
        );
    }

    #[tokio::test]
    async fn recent_read_is_newest_first_and_bounded() {
        let db = database().await;
        let payload = parse_healthmd_payload(&envelope(
            serde_json::json!([
                record("2026-08-20", Some(8_000)),
                record("2026-08-21", Some(9_000))
            ]),
            "2026-08-22T12:00:00Z",
        ))
        .unwrap();
        apply_import(&db, &payload, 1).await.unwrap();

        assert_eq!(
            load_recent(&db, 1).await.unwrap(),
            vec![StepDay {
                date: "2026-08-21".into(),
                steps: 9_000,
            }]
        );
    }
}
