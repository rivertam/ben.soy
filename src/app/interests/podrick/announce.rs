//! Job 1: announce a newly published lift.
//!
//! The rules, in the order they matter:
//!
//! 1. **Never announce history.** The first run seeds a watermark from the
//!    newest workout that already exists and announces nothing. Only workouts
//!    strictly newer than it are ever eligible, and the watermark lives in the
//!    database, so a restart cannot replay the archive.
//! 2. **Claim each lift once.** A create-only claim keyed by the workout id is
//!    taken *before* the Discord POST, so competing first claims converge.
//!    Unconfirmed retries are deliberately unleased, which is why deployment
//!    stays at one replica.
//! 3. **Prefer a late announcement to a lost one.** A claim whose POST never
//!    confirmed stays unposted and is retried on the next tick. An accepted
//!    POST whose response was lost can require manual duplicate cleanup.
//!
//! Message *content* comes from the public API rather than the database, so
//! the Eastern projection and the permanent path keep their single
//! implementation under `lifting/archive`. The message itself is a plain
//! set list rendered by the same shared formatter as `lifting/share.rs`.

pub use benjisponge::workout_text::Workout as ApiWorkout;
#[cfg(test)]
use benjisponge::workout_text::{
    Set as ApiSet, format_clock, format_date, format_duration, format_scaled, roman, set_line,
};
use benjisponge::{data::Db, workout_text};
use serde::Deserialize;
use std::time::Duration;

use crate::db::{self, AnnounceCandidate, Claim};
use crate::discord::{Discord, DiscordError, MAX_MESSAGE_CHARS};
use crate::eastern::{self, EasternInstant};

/// How many workouts one tick will announce. A burst larger than this is not
/// lost — the next tick picks up where this one stopped.
const MAX_PER_TICK: usize = 5;

#[derive(Debug)]
pub enum AnnounceError {
    Database(String),
    Api(String),
    Discord(DiscordError),
}

impl std::fmt::Display for AnnounceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnnounceError::Database(error) => write!(f, "database: {error}"),
            AnnounceError::Api(error) => write!(f, "site api: {error}"),
            AnnounceError::Discord(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AnnounceError {}

/// What one tick did, for logging and for `--once`'s exit status.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// The watermark was created by this tick; nothing was announced.
    pub seeded_watermark: Option<String>,
    pub announced: Vec<String>,
    pub retried: Vec<String>,
    pub failed: Vec<String>,
    pub retry_after: Option<Duration>,
}

impl TickReport {
    pub fn is_quiet(&self) -> bool {
        self.seeded_watermark.is_none()
            && self.announced.is_empty()
            && self.retried.is_empty()
            && self.failed.is_empty()
    }
}

/// The retryable/fatal split for one workout's post.
enum Attempt {
    Posted,
    Failed {
        detail: String,
        retry_after: Option<Duration>,
    },
}

pub struct Announcer {
    pub discord: Discord,
    pub client: reqwest::Client,
    pub api_origin: String,
    pub channel_id: String,
    pub dry_run: bool,
}

impl Announcer {
    /// One pass: recover unconfirmed claims, then announce anything new.
    pub async fn tick(&self, db: &Db, now: i64) -> Result<TickReport, AnnounceError> {
        let mut report = TickReport::default();

        // The watermark is the whole no-backfill guarantee. Seeding it is the
        // only thing a first run does.
        let watermark = match db::meta(db, db::ANNOUNCE_WATERMARK)
            .await
            .map_err(database)?
        {
            Some(watermark) => watermark,
            None => {
                let newest = db::newest_workout_start(db)
                    .await
                    .map_err(database)?
                    // An empty archive still needs a floor. The epoch start is
                    // safe: there is nothing before it to announce.
                    .unwrap_or_else(|| "1970-01-01 00:00:00".to_string());
                // A dry run must not write the watermark. Seeding is a
                // once-ever, irreversible decision about what counts as
                // history, and `--dry-run` against production would silently
                // make it.
                if self.dry_run {
                    report.seeded_watermark = Some(newest.clone());
                    newest
                } else {
                    let seeded = db::init_meta(db, db::ANNOUNCE_WATERMARK, &newest)
                        .await
                        .map_err(database)?;
                    report.seeded_watermark = Some(seeded.clone());
                    seeded
                }
            }
        };

        // Crash recovery first: a claim with no message id is a workout this
        // bot already took responsibility for but never confirmed posting.
        for claim in db::unposted_claims(db, MAX_PER_TICK)
            .await
            .map_err(database)?
        {
            let Some(workout) = db::workout(db, &claim.workout_id).await.map_err(database)? else {
                // The workout was deleted out from under the claim. Nothing to
                // post; leave the row as the record that it happened.
                continue;
            };
            match self.attempt(db, &workout, now).await? {
                Attempt::Posted => report.retried.push(workout.id),
                Attempt::Failed {
                    detail,
                    retry_after,
                } => {
                    report.failed.push(format!("{}: {detail}", workout.id));
                    if let Some(after) = retry_after {
                        report.retry_after = Some(
                            report
                                .retry_after
                                .map_or(after, |current| current.max(after)),
                        );
                        return Ok(report);
                    }
                }
            }
        }

        for workout in db::workouts_after(db, &watermark, MAX_PER_TICK)
            .await
            .map_err(database)?
        {
            // Claiming is a write, so a dry run skips it and previews instead.
            if !self.dry_run {
                let path = public_path(&workout);
                if db::claim(db, &workout.id, &path, &self.channel_id, now)
                    .await
                    .map_err(database)?
                    == Claim::Taken
                {
                    continue;
                }
            }
            match self.attempt(db, &workout, now).await? {
                Attempt::Posted => report.announced.push(workout.id),
                Attempt::Failed {
                    detail,
                    retry_after,
                } => {
                    report.failed.push(format!("{}: {detail}", workout.id));
                    if let Some(after) = retry_after {
                        report.retry_after = Some(
                            report
                                .retry_after
                                .map_or(after, |current| current.max(after)),
                        );
                        return Ok(report);
                    }
                }
            }
        }
        Ok(report)
    }

    /// Post one workout, separating "try again next tick" from "stop".
    ///
    /// A rejected token or a missing channel permission is operator error: it
    /// will fail identically forever, so it propagates and takes the process
    /// down where a restart policy and the logs will surface it. A timeout or
    /// a 5xx just leaves the claim unposted for the next tick.
    async fn attempt(
        &self,
        db: &Db,
        workout: &AnnounceCandidate,
        now: i64,
    ) -> Result<Attempt, AnnounceError> {
        match self.post(db, workout, now).await {
            Ok(()) => Ok(Attempt::Posted),
            Err(error) => {
                if !self.dry_run {
                    db::record_attempt(db, &workout.id)
                        .await
                        .map_err(database)?;
                }
                if let AnnounceError::Discord(discord) = &error
                    && !discord.is_retryable()
                {
                    return Err(error);
                }
                let retry_after = match &error {
                    AnnounceError::Discord(discord) => discord.retry_after(),
                    _ => None,
                };
                Ok(Attempt::Failed {
                    detail: error.to_string(),
                    retry_after,
                })
            }
        }
    }

    /// Render and post one claimed workout, then confirm it.
    async fn post(
        &self,
        db: &Db,
        workout: &AnnounceCandidate,
        now: i64,
    ) -> Result<(), AnnounceError> {
        let path = public_path(workout);
        let detail = self.fetch_workout(&path).await?;
        // Times come from the workout's own Eastern fields, not the clock:
        // the message states when the lift happened, not when it was posted.
        let message = render(&detail, &path, &self.api_origin);
        if self.dry_run {
            println!("--- would post to channel {} ---", self.channel_id);
            println!("{message}");
            return Ok(());
        }
        let message_id = self
            .discord
            .post_message(&self.channel_id, &message)
            .await
            .map_err(AnnounceError::Discord)?;
        db::mark_posted(db, &workout.id, &message_id, now)
            .await
            .map_err(database)
    }

    async fn fetch_workout(&self, path: &str) -> Result<ApiWorkout, AnnounceError> {
        let url = format!("{}/api/fitness/workouts/by-path/{path}", self.api_origin);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|error| AnnounceError::Api(error.to_string()))?;
        if !response.status().is_success() {
            return Err(AnnounceError::Api(format!(
                "{} returned {}",
                url,
                response.status()
            )));
        }
        let envelope: ApiEnvelope = response
            .json()
            .await
            .map_err(|error| AnnounceError::Api(error.to_string()))?;
        envelope
            .workout
            .ok_or_else(|| AnnounceError::Api(format!("{url} returned no workout")))
    }
}

fn database(error: surrealdb::Error) -> AnnounceError {
    AnnounceError::Database(error.to_string())
}

/// The canonical public path for a stored workout.
///
/// `started_at_local` and `eastern_offset_minutes` are written by the server
/// at import; this only reassembles them, exactly as
/// `eastern::public_path` does for a computed instant.
fn public_path(workout: &AnnounceCandidate) -> String {
    eastern::public_path(&EasternInstant {
        local: workout.started_at_local.clone(),
        offset_minutes: workout.eastern_offset_minutes as i32,
    })
}

// ---------------------------------------------------------------------------
// The public API's workout envelope, reduced to what a message needs. Unknown
// fields are ignored, so the reader contract can grow without breaking Podrick.

#[derive(Debug, Deserialize)]
struct ApiEnvelope {
    workout: Option<ApiWorkout>,
}

/// Build the Discord message: a plain-text set list ending in the permalink.
///
/// The shared formatter owns all content and grouping. Podrick contributes
/// only its absolute permalink and Discord's 2000-character transport cap.
pub fn render(workout: &ApiWorkout, path: &str, origin: &str) -> String {
    let link = format!("{origin}/fitness/lift/{path}");
    workout_text::format(workout, &link, Some(MAX_MESSAGE_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(exercise: &str, weight: Option<i64>, reps: Option<i64>, effort: Option<i64>) -> ApiSet {
        ApiSet {
            exercise_name: exercise.to_string(),
            set_type: "NORMAL_SET".to_string(),
            weight_unit: "lbs".to_string(),
            weight_milli: weight,
            reps,
            effort_hundredths: effort,
            ..ApiSet::default()
        }
    }

    fn workout() -> ApiWorkout {
        ApiWorkout {
            title: "I missed 9am gym".to_string(),
            started_at_local: "2026-07-21 10:39:04".to_string(),
            ended_at_local: "2026-07-21 11:14:14".to_string(),
            duration_seconds: 2110,
            sets: vec![
                ApiSet {
                    set_type: "WARMUP_SET".to_string(),
                    ..set("Incline Bench Press", Some(45_000), Some(10), None)
                },
                set("Incline Bench Press", Some(145_000), Some(3), Some(800)),
                ApiSet {
                    failure: true,
                    ..set("Cable Crossover", Some(25_000), Some(9), None)
                },
            ],
            ..ApiWorkout::default()
        }
    }

    /// The whole point of the format: it reads like the site's share sheet.
    #[test]
    fn a_message_is_a_plain_set_list_ending_in_the_permalink() {
        let message = render(&workout(), "2026-07-21T10-39-04-04-00", "https://ben.soy");
        assert_eq!(
            message,
            "\
**I missed 9am gym**
Jul 21, 2026 · 10:39 AM–11:14 AM · 35m 10s · 2 working sets

I. Incline Bench Press
W. 45 lbs × 10
1. 145 lbs × 3 @ RPE 8

II. Cable Crossover
1. 25 lbs × 9 · failure

https://ben.soy/fitness/lift/2026-07-21T10-39-04-04-00"
        );
    }

    /// Discord reads a leading `1.` as ordered-list syntax. Set lines are
    /// meant to be a list and number correctly on their own; the danger is an
    /// exercise heading joining them, which merges every block into one list
    /// and renumbers the whole message. Headings must never be list syntax.
    #[test]
    fn exercise_headings_are_never_markdown_list_syntax() {
        let mut workout = workout();
        workout.sets = (0..12)
            .map(|index| {
                let mut set = set(&format!("Exercise {index}"), Some(225_000), Some(5), None);
                if index < 3 {
                    set.set_type = "WARMUP_SET".to_string();
                }
                set
            })
            .collect();
        let message = render(&workout, "p", "https://x.test");

        for line in message.lines().filter(|line| line.contains("Exercise ")) {
            assert!(
                !line.starts_with(char::is_numeric),
                "heading would merge into the set list: {line:?}"
            );
        }
        assert!(message.contains("I. Exercise 0"));
        assert!(message.contains("IX. Exercise 8"));
        assert!(message.contains("XII. Exercise 11"));
        // Each exercise here has one set, so warm-ups never take a number.
        assert_eq!(message.matches("\nW. ").count(), 3);
        assert!(message.contains("9 working sets"));
    }

    #[test]
    fn working_sets_number_from_one_after_any_warm_ups() {
        let mut workout = workout();
        workout.sets = (0..5)
            .map(|index| {
                let mut set = set("Squat", Some(225_000), Some(5), None);
                if index < 2 {
                    set.set_type = "WARMUP_SET".to_string();
                }
                set
            })
            .collect();
        let message = render(&workout, "p", "https://x.test");
        assert!(
            message.contains("I. Squat\nW. 225 lbs × 5\nW. 225 lbs × 5\n1. 225 lbs × 5\n2. "),
            "{message}"
        );
        assert!(message.contains("3 working sets"));
    }

    #[test]
    fn warm_ups_count_toward_nothing() {
        let mut workout = workout();
        for set in &mut workout.sets {
            set.set_type = "WARMUP_SET".to_string();
        }
        let message = render(&workout, "p", "https://x.test");
        assert!(message.contains("· 0 working sets"), "{message}");
        // No set number is issued when every set is preparation.
        assert!(!message.contains("\n1. "), "{message}");
        assert_eq!(message.matches("\nW. ").count(), 3);
    }

    /// Exercise names are rendered verbatim. A name like `3. Pec Fly` cannot
    /// be imported — `strip_exercise_number` rejects a numbered heading — so
    /// Podrick does not compensate for one. A stray prefix stays visible as
    /// the storage bug it is instead of being quietly tidied in the channel.
    #[test]
    fn exercise_names_are_rendered_verbatim() {
        let mut workout = workout();
        workout.sets = vec![set("St. Bench Row", Some(175_000), Some(7), None)];
        let message = render(&workout, "p", "https://x.test");
        assert!(message.contains("I. St. Bench Row"), "{message}");
    }

    #[test]
    fn roman_numerals_cover_realistic_set_counts() {
        let cases = [
            (1, "I"),
            (4, "IV"),
            (5, "V"),
            (9, "IX"),
            (10, "X"),
            (14, "XIV"),
            (19, "XIX"),
            (40, "XL"),
            (49, "XLIX"),
        ];
        for (value, expected) in cases {
            assert_eq!(roman(value), expected, "roman({value})");
        }
    }

    #[test]
    fn personal_records_never_reach_the_channel() {
        // The API sends records on every set; the message must ignore them
        // entirely and let the permalink do that job.
        let raw = r#"{"workout":{"title":"Pshaw","started_at_local":"2026-07-27 13:42:00",
            "ended_at_local":"2026-07-27 14:16:00","duration_seconds":2040,
            "sets":[{"exercise_name":"Bench","set_type":"NORMAL_SET","weight_unit":"lbs",
            "weight_milli":135000,"reps":6,"effort_hundredths":800,
            "records":[{"level":"gold","kind":"1rm"},{"level":"gold","kind":"reps"}]}]}}"#;
        let envelope: ApiEnvelope = serde_json::from_str(raw).unwrap();
        let message = render(&envelope.workout.unwrap(), "p", "https://x.test");
        assert!(message.contains("1. 135 lbs × 6 @ RPE 8"));
        for banned in ["PR", "gold", "1rm", "🥇", "record"] {
            assert!(!message.contains(banned), "{banned} leaked into: {message}");
        }
    }

    #[test]
    fn repeating_an_exercise_later_starts_a_new_block() {
        let mut workout = workout();
        workout
            .sets
            .push(set("Incline Bench Press", Some(155_000), Some(2), None));
        let message = render(&workout, "p", "https://x.test");
        // Two separately numbered headings, each restarting its sets at 1.
        assert_eq!(message.matches("Incline Bench Press").count(), 2);
        assert!(message.contains("I. Incline Bench Press"));
        assert!(message.ends_with(
            "III. Incline Bench Press\n1. 155 lbs × 2\n\nhttps://x.test/fitness/lift/p"
        ));
    }

    #[test]
    fn supersets_are_labelled_like_the_share_sheet() {
        let mut workout = workout();
        workout.sets[1].superset_id = Some(2);
        workout.sets[1].exercise_name = "Row".to_string();
        let message = render(&workout, "p", "https://x.test");
        assert!(message.contains("Row · superset 2"), "{message}");
    }

    #[test]
    fn loads_efforts_and_durations_match_the_site_formatters() {
        assert_eq!(format_scaled(135_000, 1_000), "135");
        assert_eq!(format_scaled(137_500, 1_000), "137.5");
        assert_eq!(format_scaled(1_250_000, 1_000), "1,250");
        assert_eq!(format_scaled(800, 100), "8");
        assert_eq!(format_scaled(850, 100), "8.5");
        assert_eq!(format_scaled(-45_125, 1_000), "-45.125");
        assert_eq!(format_scaled(-1_250_000, 1_000), "-1,250");
        assert_eq!(format_duration(2110), "35m 10s");
        assert_eq!(format_duration(3_840), "1h 04m");
        assert_eq!(format_duration(12), "12s");
    }

    #[test]
    fn clock_and_date_formatting_covers_noon_and_midnight() {
        assert_eq!(
            format_date("2026-07-21 10:39:04").as_deref(),
            Some("Jul 21, 2026")
        );
        assert_eq!(
            format_clock("2026-07-21 00:05:00").as_deref(),
            Some("12:05 AM")
        );
        assert_eq!(
            format_clock("2026-07-21 12:00:00").as_deref(),
            Some("12:00 PM")
        );
        assert_eq!(
            format_clock("2026-07-21 13:42:00").as_deref(),
            Some("1:42 PM")
        );
        // Unparseable input drops the fact rather than printing garbage.
        assert_eq!(format_date("").as_deref(), None);
        assert_eq!(format_clock("2026-13-21 10:00:00").as_deref(), None);
    }

    #[test]
    fn a_set_with_no_load_or_reps_still_says_something() {
        let mut bodyweight = set("Plank", None, None, None);
        bodyweight.set_time_seconds = Some(90);
        assert_eq!(set_line(&bodyweight), "1m 30s");

        let mut carry = set("Farmer Carry", None, None, None);
        carry.distance_milli = Some(40_000);
        assert_eq!(set_line(&carry), "distance 40");

        assert_eq!(set_line(&set("Mystery", None, None, None)), "logged");
        assert_eq!(set_line(&set("Pull Up", None, Some(8), None)), "8 reps");
    }

    #[test]
    fn description_and_notes_ride_along_escaped() {
        let mut workout = workout();
        workout.description = Some("Deload **week**".to_string());
        workout.notes = Some("felt fine".to_string());
        let message = render(&workout, "p", "https://x.test");
        assert!(message.contains("Deload \\*\\*week\\*\\*"), "{message}");
        assert!(message.contains("felt fine"));
    }

    #[test]
    fn markdown_in_a_title_cannot_style_the_channel() {
        let mut workout = workout();
        workout.title = "**Leg** _day_".to_string();
        let message = render(&workout, "p", "https://x.test");
        assert!(message.starts_with(r"**\*\*Leg\*\* \_day\_**"), "{message}");
    }

    #[test]
    fn an_oversized_workout_keeps_the_header_the_link_and_a_count() {
        let mut workout = workout();
        workout.sets = (0..400)
            .map(|index| {
                set(
                    &format!("Exercise number {index}"),
                    Some(135_000),
                    Some(8),
                    Some(800),
                )
            })
            .collect();
        let message = render(&workout, "p", "https://x.test");

        assert!(
            message.chars().count() <= MAX_MESSAGE_CHARS,
            "{} chars",
            message.chars().count()
        );
        assert!(message.starts_with("**I missed 9am gym**"));
        assert!(message.ends_with("https://x.test/fitness/lift/p"));
        assert!(message.contains("… and "), "{message}");
        // Nothing is half-rendered: the last kept block is complete.
        assert!(!message.contains("Exercise number 399"));
    }

    #[test]
    fn a_workout_that_exactly_fits_is_not_truncated() {
        let message = render(&workout(), "p", "https://x.test");
        assert!(!message.contains("… and "));
    }

    #[test]
    fn the_public_path_matches_the_stored_eastern_fields() {
        let workout = AnnounceCandidate {
            id: "fitness:2026-07-28T18:30:00".to_string(),
            title: "Push".to_string(),
            started_at_utc: "2026-07-28 22:30:00".to_string(),
            started_at_local: "2026-07-28 18:30:00".to_string(),
            eastern_offset_minutes: -240,
        };
        assert_eq!(public_path(&workout), "2026-07-28T18-30-00-04-00");
    }

    #[test]
    fn a_report_with_only_a_seeded_watermark_is_not_quiet() {
        let mut report = TickReport::default();
        assert!(report.is_quiet());
        report.seeded_watermark = Some("2026-07-28 22:30:00".to_string());
        assert!(!report.is_quiet());
    }
}
