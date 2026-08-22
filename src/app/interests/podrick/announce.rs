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
//! set list mirroring `lifting/share.rs` — see `render` for why that format
//! is duplicated rather than called, and why it omits personal records.

use benjisponge::data::Db;
use serde::Deserialize;
use std::time::Duration;

use crate::db::{self, AnnounceCandidate, Claim};
use crate::discord::{Discord, DiscordError, MAX_MESSAGE_CHARS};
use crate::eastern::{self, EasternInstant};

/// How many workouts one tick will announce. A burst larger than this is not
/// lost — the next tick picks up where this one stopped.
const MAX_PER_TICK: usize = 5;

/// Set types the share sheet annotates inline; every other type is unmarked.
const WARMUP_SET: &str = "WARMUP_SET";
const FAILURE_SET: &str = "FAILURE_SET";

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

#[derive(Debug, Default, Deserialize)]
pub struct ApiWorkout {
    pub title: String,
    #[serde(default)]
    pub started_at_local: String,
    #[serde(default)]
    pub ended_at_local: String,
    #[serde(default)]
    pub duration_seconds: i64,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sets: Vec<ApiSet>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ApiSet {
    pub exercise_name: String,
    #[serde(default)]
    pub set_type: String,
    #[serde(default)]
    pub superset_id: Option<i64>,
    #[serde(default)]
    pub weight_milli: Option<i64>,
    #[serde(default)]
    pub weight_unit: String,
    #[serde(default)]
    pub reps: Option<i64>,
    #[serde(default)]
    pub effort_hundredths: Option<i64>,
    #[serde(default)]
    pub distance_milli: Option<i64>,
    #[serde(default)]
    pub set_time_seconds: Option<i64>,
}

/// Build the Discord message: a plain-text set list ending in the permalink.
///
/// This deliberately mirrors the site's own share sheet
/// (`lifting/share.rs::share_text`) line for line — same meta line, same
/// `1. 135 lbs × 6 @ RPE 8` grammar, same blank-line grouping — because both
/// exist to be a Strong/Lyfta-style rendition of the same workout, and two
/// gratuitously different plain-text formats for one workout would be worse
/// than one duplicated formatter.
///
/// It is a separate implementation rather than a call into `share.rs` because
/// that function reaches `WorkoutCard` -> `filters` -> the snapshot engine,
/// none of which belongs in a bot, and because the outputs genuinely differ:
///
/// - **No personal records.** The site's sheet tags PR sets; this does not.
///   The permalink is one click away and shows them properly, and a channel
///   post that crows about four record categories per exercise is noise.
/// - Discord needs a bold title, markdown escaping, and a 2000-character cap.
///
/// Kept pure so the copy is tuned against tests, not a live channel.
pub fn render(workout: &ApiWorkout, path: &str, origin: &str) -> String {
    let link = format!("{origin}/fitness/lift/{path}");
    let header = message_header(workout);
    let groups = exercise_groups(&workout.sets);

    let mut body: Vec<String> = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        body.push(String::new());
        body.extend(group_lines(group, index + 1));
    }

    let full = [
        header.clone(),
        body.clone(),
        vec![String::new(), link.clone()],
    ]
    .concat();
    let message = full.join("\n");
    if message.chars().count() <= MAX_MESSAGE_CHARS {
        return message;
    }
    truncated(&header, &groups, &link)
}

/// Title and the facts line, matching the share sheet's first two lines minus
/// its volume-points total (site jargon that means nothing in a chat).
///
/// The count is working sets only. A warm-up is not a set you did, it is how
/// you got ready to do one — it scores zero volume points on the site for the
/// same reason — so counting them would inflate every workout.
fn message_header(workout: &ApiWorkout) -> Vec<String> {
    let mut header = vec![format!("**{}**", escape_markdown(&workout.title))];

    let mut facts = Vec::new();
    if let Some(date) = format_date(&workout.started_at_local) {
        facts.push(date);
    }
    if let Some(range) = format_time_range(&workout.started_at_local, &workout.ended_at_local) {
        facts.push(range);
    }
    if workout.duration_seconds > 0 {
        facts.push(format_duration(workout.duration_seconds));
    }
    let working = workout.sets.iter().filter(|set| is_working(set)).count();
    facts.push(plural(working, "working set", "working sets"));
    header.push(facts.join(" · "));

    // Description and notes are the lifter's own words; the share sheet keeps
    // them and so does this.
    for extra in [&workout.description, &workout.notes] {
        if let Some(text) = extra.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            header.push(String::new());
            header.push(escape_markdown(text));
        }
    }
    header
}

struct ExerciseGroup<'a> {
    name: &'a str,
    superset_id: Option<i64>,
    sets: Vec<&'a ApiSet>,
}

/// Consecutive runs of one exercise, exactly how the workout page blocks them.
/// Runs are not merged across a gap: repeating an exercise later in the session
/// is a separate block on the page and reads as one here too.
fn exercise_groups(sets: &[ApiSet]) -> Vec<ExerciseGroup<'_>> {
    let mut groups: Vec<ExerciseGroup> = Vec::new();
    for set in sets {
        match groups.last_mut() {
            Some(group) if group.name == set.exercise_name => group.sets.push(set),
            _ => groups.push(ExerciseGroup {
                name: &set.exercise_name,
                superset_id: set.superset_id,
                sets: vec![set],
            }),
        }
    }
    groups
}

/// `I. Incline Bench Press`, numbered by the exercise's position in the
/// workout.
///
/// Roman rather than Arabic because Discord reads a leading `1.` as markdown
/// ordered-list syntax. When the headings were Arabic they joined the set
/// lines into one continuous list and Discord renumbered the lot. A Roman
/// heading is plain text, so each exercise's sets form their own short list
/// that starts at 1 — which is the numbering we actually want.
/// The stored exercise name is used verbatim. A leading `3. ` would render as
/// `III. 3. Pec Fly`, but that is a storage bug — `strip_exercise_number`
/// rejects numbered headings at import — and compensating for it here would
/// only hide it. See the note in `docs/podrick.md`.
fn group_heading(group: &ExerciseGroup<'_>, position: usize) -> String {
    let numbered = format!("{}. {}", roman(position), escape_markdown(group.name));
    match group.superset_id {
        Some(id) => format!("{numbered} · superset {id}"),
        None => numbered,
    }
}

/// Whether a set counts. Warm-ups are the only excluded type, matching the
/// records engine and the site's zero-volume-point scoring.
fn is_working(set: &ApiSet) -> bool {
    set.set_type != WARMUP_SET
}

/// One exercise block: its Roman-numbered heading, then one line per set.
///
/// Working sets are numbered `1, 2, 3` and warm-ups are marked `W.`, so the
/// working sets count up independently of how many warm-ups preceded them.
fn group_lines(group: &ExerciseGroup<'_>, position: usize) -> Vec<String> {
    let mut lines = vec![group_heading(group, position)];
    let mut working = 0usize;
    for set in &group.sets {
        let marker = if is_working(set) {
            working += 1;
            format!("{working}.")
        } else {
            "W.".to_string()
        };
        lines.push(format!("{marker} {}", set_line(set)));
    }
    lines
}

/// One set: `135 lbs × 6 · failure @ RPE 8`, mirroring the share sheet. The
/// warm-up annotation is gone — the `W.` marker already says it.
fn set_line(set: &ApiSet) -> String {
    let mut line = prescription(set);
    if set.set_type == FAILURE_SET {
        line.push_str(" · failure");
    }
    if let Some(effort) = set.effort_hundredths {
        line.push_str(&format!(" @ RPE {}", format_scaled(effort, 100)));
    }
    line
}

/// Roman numeral for a 1-based set position.
fn roman(value: usize) -> String {
    const NUMERALS: [(usize, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut remaining = value;
    let mut output = String::new();
    for (amount, numeral) in NUMERALS {
        while remaining >= amount {
            output.push_str(numeral);
            remaining -= amount;
        }
    }
    output
}

fn prescription(set: &ApiSet) -> String {
    let unit = if set.weight_unit.is_empty() {
        "lbs"
    } else {
        &set.weight_unit
    };
    match (set.weight_milli, set.reps) {
        (Some(load), Some(reps)) => format!("{} {unit} × {reps}", format_scaled(load, 1_000)),
        (Some(load), None) => format!("{} {unit}", format_scaled(load, 1_000)),
        (None, Some(reps)) => format!("{reps} reps"),
        (None, None) => match (set.distance_milli, set.set_time_seconds) {
            (Some(distance), _) => format!("distance {}", format_scaled(distance, 1_000)),
            (None, Some(seconds)) => format_duration(seconds),
            (None, None) => "logged".to_string(),
        },
    }
}

/// Drop whole exercise blocks from the end until the message fits, and say how
/// many sets were left out. The title, the facts line, and the permalink are
/// never dropped — they are the point of the post, and the link has the rest.
fn truncated(header: &[String], groups: &[ExerciseGroup<'_>], link: &str) -> String {
    let total_sets: usize = groups.iter().map(|group| group.sets.len()).sum();
    let mut kept: Vec<String> = Vec::new();
    let mut shown_sets = 0usize;

    for (index, group) in groups.iter().enumerate() {
        let block = [vec![String::new()], group_lines(group, index + 1)].concat();
        let omitted = total_sets - (shown_sets + group.sets.len());
        let candidate = [
            header.to_vec(),
            kept.clone(),
            block.clone(),
            vec![
                String::new(),
                format!("… and {omitted} more"),
                String::new(),
                link.to_string(),
            ],
        ]
        .concat()
        .join("\n");
        if candidate.chars().count() > MAX_MESSAGE_CHARS {
            break;
        }
        kept.extend(block);
        shown_sets += group.sets.len();
    }

    let mut lines = [header.to_vec(), kept].concat();
    let omitted = total_sets - shown_sets;
    if omitted > 0 {
        lines.push(String::new());
        lines.push(format!("… and {omitted} more"));
    }
    lines.push(String::new());
    lines.push(link.to_string());
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Formatting, matching `lifting/format.rs` so the two renditions agree.

/// Scaled integer to a plain decimal: `135_000/1_000` -> `135`,
/// `137_500/1_000` -> `137.5`. Mirrors `format::format_scaled`, including its
/// thousands separators.
fn format_scaled(value: i64, scale: i64) -> String {
    if value < 0 {
        return value.to_string();
    }
    let whole = value / scale;
    let remainder = value % scale;
    let mut output = format_integer(whole);
    if remainder > 0 {
        let width = scale.ilog10() as usize;
        let fraction = format!("{remainder:0width$}")
            .trim_end_matches('0')
            .to_string();
        output.push('.');
        output.push_str(&fraction);
    }
    output
}

fn format_integer(value: i64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

/// `1h 04m` / `34m 00s` / `12s`, mirroring `format::format_duration`.
fn format_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let hours = seconds / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `YYYY-MM-DD HH:MM:SS` split into its numeric parts.
fn parse_local(value: &str) -> Option<(i64, usize, i64, i64, i64)> {
    let bytes = value.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' {
        return None;
    }
    let year = value[0..4].parse().ok()?;
    let month: usize = value[5..7].parse().ok()?;
    let day = value[8..10].parse().ok()?;
    let hour = value[11..13].parse().ok()?;
    let minute = value[14..16].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some((year, month, day, hour, minute))
}

/// `Jul 27, 2026`.
fn format_date(local: &str) -> Option<String> {
    let (year, month, day, _, _) = parse_local(local)?;
    Some(format!("{} {day}, {year}", MONTHS[month - 1]))
}

/// `1:42 PM–2:16 PM`, or just the start when the end is unusable.
fn format_time_range(start: &str, end: &str) -> Option<String> {
    let start = format_clock(start)?;
    Some(match format_clock(end) {
        Some(end) => format!("{start}–{end}"),
        None => start,
    })
}

fn format_clock(local: &str) -> Option<String> {
    let (_, _, _, hour, minute) = parse_local(local)?;
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let hour12 = match hour % 12 {
        0 => 12,
        other => other,
    };
    Some(format!("{hour12}:{minute:02} {suffix}"))
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {many}")
    }
}

/// Neutralize Discord markdown in text this bot did not author. A workout
/// title or note is user input as far as the channel is concerned.
fn escape_markdown(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '*' | '_' | '~' | '`' | '|' | '\\' | '>' | '#') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
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
                    set_type: "FAILURE_SET".to_string(),
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
