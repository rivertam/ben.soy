//! RSS 2.0 feed at `/feed.xml`, generated from the logbook registry — every
//! entry, long or short, becomes an `<item>`, so publishing to the log
//! publishes to the feed. Slay the Spire victories (and only victories —
//! deaths stay on `/spire`) join the feed at render time from the synced run
//! database, as do workouts explicitly published through the authenticated
//! manual-entry path and running activities (the same sets the home
//! timeline shows). Not a page: it renders no shell and stays out of
//! `site_routes()` (the 404 index is for pages).

use topcoat::{
    Result,
    context::{Cx, app_context},
    router::route,
};

use benjisponge::data::{Data, running_models::RunningActivity};
use jiff::Timestamp;

use crate::app::interests::{
    lifting::archive::{api::Workout, scoring, snapshot::PublishedWorkout, store::FitnessStore},
    running,
    spire::runs::{self as spire_runs, Run, fmt_duration},
};
use crate::content::logbook::{Entry, LOG};

/// Where absolute links point. `SITE_ORIGIN` overrides the default at
/// runtime; the default is the site's canonical public origin.
fn origin() -> String {
    std::env::var("SITE_ORIGIN").unwrap_or_else(|_| "https://ben.soy".to_string())
}

#[route(GET "/feed.xml")]
async fn feed(cx: &Cx) -> Result<([(&'static str, &'static str); 2], String)> {
    let spire = spire_runs::load(app_context::<Data>(cx)).await;
    let run_log = running::load(app_context::<Data>(cx)).await;
    let workouts = match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => snapshot.published_workouts(),
        Err(error) => {
            // Dynamic data must never take down the static feed. Match the
            // Spire loader's stale-or-empty failure behavior.
            eprintln!("fitness feed snapshot failed: {error}");
            Vec::new()
        }
    };
    Ok((
        [
            ("Content-Type", "application/rss+xml; charset=utf-8"),
            // Fresh Spire runs, workouts, and running activities appear
            // within a minute; CDN honors
            // s-maxage (see docs/railway-deploy.md).
            ("Cache-Control", "public, max-age=0, s-maxage=60"),
        ],
        rss_xml(&origin(), &spire.runs, &workouts, &run_log.activities),
    ))
}

/// One feed item, from any source, ready to sort and emit.
struct FeedItem {
    date: String,
    /// RFC 2822 publication instant. Curated entries are day-granular and use
    /// midnight UTC; database-backed items preserve their exact source time.
    pub_date: String,
    /// Curated logbook entries outrank dynamic items on the same date.
    curated: bool,
    /// Tie-break among dynamic items sharing a date; 0 for logbook entries.
    start_time: i64,
    title: String,
    link: String,
    description: String,
    guid: String,
}

/// The whole feed as a string. Pure — origin and dynamic rows in, XML out.
/// Losses and abandoned runs are filtered here so callers can pass the full
/// Spire log; workout publication was already filtered by the snapshot.
pub fn rss_xml(
    origin: &str,
    runs: &[Run],
    workouts: &[PublishedWorkout],
    running_activities: &[RunningActivity],
) -> String {
    let mut items: Vec<FeedItem> = Vec::new();

    for (index, entry) in LOG.iter().enumerate() {
        let (title, link, description) = match entry {
            Entry::Essay {
                title,
                teaser,
                slug,
                ..
            } => (
                (*title).to_string(),
                format!("{origin}/thoughts/{slug}"),
                (*teaser).to_string(),
            ),
            Entry::Note { body, slug, .. } => (
                truncate(body, 80),
                format!("{origin}/thoughts/{slug}"),
                (*body).to_string(),
            ),
            Entry::Update {
                stamp,
                label,
                body,
                href,
                ..
            } => {
                let link = if href.starts_with('/') {
                    format!("{origin}{href}")
                } else {
                    (*href).to_string()
                };
                (
                    format!("[{stamp}] {label} — {body}"),
                    link,
                    (*body).to_string(),
                )
            }
        };
        items.push(FeedItem {
            date: entry.date().to_string(),
            pub_date: rfc2822(entry.date()),
            curated: true,
            start_time: 0,
            title,
            link,
            description,
            guid: format!("{}/log/{:04}", origin, LOG.len() - index),
        });
    }

    for run in runs.iter().filter(|r| r.win && !r.abandoned) {
        items.push(FeedItem {
            date: run.date.clone(),
            pub_date: rfc2822_timestamp(run.start_time, &run.date),
            curated: false,
            start_time: run.start_time,
            title: format!(
                "[spire] {} win — {}, Ascension {}",
                run.game_label(),
                run.character,
                run.ascension
            ),
            link: format!("{origin}/spire"),
            description: format!(
                "{} · {} victory at Ascension {} — {} floors in {}.",
                run.game_label(),
                run.character,
                run.ascension,
                run.floors,
                fmt_duration(run.run_time)
            ),
            guid: if run.game == "sts1" {
                format!("{origin}/spire/run/sts1/{}", run.id)
            } else {
                format!("{origin}/spire/run/{}", run.id)
            },
        });
    }

    for published in workouts {
        let workout = &published.workout;
        let link = format!("{origin}/fitness/lift/{}", workout.path);
        items.push(FeedItem {
            date: published.date.clone(),
            pub_date: rfc2822_timestamp(published.start_time, &published.date),
            curated: false,
            start_time: published.start_time,
            title: format!("[fitness] lift — {}", workout.title),
            link,
            description: workout_description(workout),
            // The UTC-derived archive id is the immutable identity anchor;
            // unlike the reader-facing Eastern path, it can never change if
            // timezone projection rules do.
            guid: format!("{origin}/lifting/workout/{}", workout.id),
        });
    }

    for activity in running_activities {
        let path = running::public_url(activity);
        let start_time = running::start_time_seconds(activity);
        items.push(FeedItem {
            date: running::activity_date(activity).to_string(),
            pub_date: rfc2822_timestamp(start_time, running::activity_date(activity)),
            curated: false,
            start_time,
            title: format!("[fitness] run — {}", activity.title),
            link: format!("{origin}{path}"),
            description: running::feed_description(activity),
            // The storage id is derived from the Garmin or manual source
            // identity, while the reader-facing path is derived from local
            // time. Keep the id as the immutable feed identity anchor.
            guid: format!("{origin}/running/activity/{}", activity.id),
        });
    }

    // Newest first; a curated entry leads dynamic entries on its date, then
    // exact source instants interleave lifts, fitness runs, and Spire runs.
    items.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| b.curated.cmp(&a.curated))
            .then_with(|| b.start_time.cmp(&a.start_time))
    });

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n");
    xml.push_str("<channel>\n");
    xml.push_str("<title>Ben Berman — logbook</title>\n");
    // The timeline is the home page; guids keep their historical /log/NNNN
    // shape so readers don't re-surface every item as new.
    xml.push_str(&format!("<link>{}/</link>\n", escape(origin)));
    xml.push_str("<description>Everything gets an entry, long or short.</description>\n");
    xml.push_str("<language>en-us</language>\n");
    xml.push_str(&format!(
        "<atom:link href=\"{}/feed.xml\" rel=\"self\" type=\"application/rss+xml\"/>\n",
        escape(origin)
    ));
    for item in &items {
        xml.push_str("<item>\n");
        xml.push_str(&format!("<title>{}</title>\n", escape(&item.title)));
        xml.push_str(&format!("<link>{}</link>\n", escape(&item.link)));
        xml.push_str(&format!(
            "<description>{}</description>\n",
            escape(&item.description)
        ));
        xml.push_str(&format!(
            "<guid isPermaLink=\"false\">{}</guid>\n",
            escape(&item.guid)
        ));
        xml.push_str(&format!("<pubDate>{}</pubDate>\n", item.pub_date));
        xml.push_str("</item>\n");
    }
    xml.push_str("</channel>\n</rss>\n");
    xml
}

/// One-line set summary used by `/feed.xml`.
pub(crate) fn workout_description(workout: &Workout) -> String {
    let mut seen = std::collections::HashSet::new();
    let exercises: Vec<&str> = workout
        .sets
        .iter()
        .filter_map(|set| {
            seen.insert(set.exercise_name.as_str())
                .then_some(set.exercise_name.as_str())
        })
        .collect();
    let set_count = workout.sets.len();
    let exercise_count = exercises.len();
    let mut description = format!(
        "{set_count} {} across {exercise_count} {} in {}",
        plural(set_count, "set", "sets"),
        plural(exercise_count, "exercise", "exercises"),
        workout_duration(workout.duration_seconds),
    );
    if exercises.is_empty() {
        description.push('.');
    } else {
        description.push_str(": ");
        description.push_str(&human_list(&exercises));
        description.push('.');
    }
    description
}

/// Total volume points across a workout's sets — used by the home timeline.
pub(crate) fn workout_volume_points(workout: &Workout) -> u32 {
    workout.sets.iter().fold(0_u32, |total, set| {
        total.saturating_add(scoring::set_volume_points(
            set.set_type.as_str(),
            set.effort_hundredths,
            set.failure,
        ))
    })
}

fn workout_duration(seconds: u64) -> String {
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

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

fn human_list(values: &[&str]) -> String {
    match values {
        [] => String::new(),
        [only] => (*only).to_string(),
        [first, second] => format!("{first} and {second}"),
        _ => format!(
            "{}, and {}",
            values[..values.len() - 1].join(", "),
            values[values.len() - 1]
        ),
    }
}

/// XML-escape everything interpolated into the feed.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Cut to at most `max` chars (a char boundary by construction) with an
/// ellipsis when anything was dropped.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max).collect();
        format!("{}…", cut.trim_end())
    }
}

/// `YYYY-MM-DD` → RFC 2822 at midnight UTC, e.g. `Sun, 12 Jul 2026 00:00:00
/// +0000`. Weekday via Sakamoto's method — no date crate in the tree. Inputs
/// are shape-checked upstream (logbook tests, filtered run dates, and the
/// fitness snapshot's Eastern projection).
fn rfc2822(iso: &str) -> String {
    let year: i32 = iso[0..4].parse().expect("feed date year");
    let month: usize = iso[5..7].parse().expect("feed date month");
    let day: u32 = iso[8..10].parse().expect("feed date day");

    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let y = if month < 3 { year - 1 } else { year };
    let weekday_index =
        (y + y / 4 - y / 100 + y / 400 + OFFSETS[month - 1] + day as i32).rem_euclid(7);

    format!(
        "{}, {:02} {} {} 00:00:00 +0000",
        WEEKDAYS[weekday_index as usize],
        day,
        MONTHS[month - 1],
        year
    )
}

/// Exact database-backed publication time in UTC. The date-only fallback
/// keeps a malformed external row from taking down the entire feed; workout
/// timestamps have already passed the snapshot's strict timestamp parser.
fn rfc2822_timestamp(seconds: i64, fallback_date: &str) -> String {
    Timestamp::from_second(seconds)
        .map(|instant| instant.strftime("%a, %d %b %Y %H:%M:%S +0000").to_string())
        .unwrap_or_else(|_| rfc2822(fallback_date))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://example.test";

    fn run(id: &str, date: &str, win: bool) -> Run {
        Run {
            id: id.to_string(),
            game: "sts2".to_string(),
            date: date.to_string(),
            start_time: id.parse().unwrap(),
            character: "Necrobinder & <Friends>".to_string(),
            win,
            abandoned: false,
            ascension: 3,
            floors: 34,
            killed_by: None,
            kill_kind: None,
            run_time: 7534,
            game_mode: "standard".to_string(),
        }
    }

    fn lift(
        id: &str,
        date: &str,
        start_time: i64,
        title: &str,
        duration_seconds: u64,
        set_count: usize,
        exercises: &[&str],
    ) -> PublishedWorkout {
        let path = format!("{date}T10-38-00-04-00");
        let sets = (0..set_count)
            .map(|index| {
                let exercise = exercises
                    .get(index % exercises.len().max(1))
                    .copied()
                    .unwrap_or("Unknown exercise");
                crate::app::interests::lifting::archive::api::Set {
                    id: format!("{id}:{:04}", index + 1),
                    ordinal: (index + 1) as u32,
                    exercise_name: exercise.to_string(),
                    raw_exercise_name: exercise.to_string(),
                    exercise_note: None,
                    superset_id: None,
                    weight_milli: Some(95_000),
                    weight_unit: "lbs".to_string(),
                    reps: Some(8),
                    effort_hundredths: Some(1_000),
                    failure: false,
                    distance_milli: None,
                    set_time_seconds: None,
                    set_type: "NORMAL_SET".to_string(),
                    records: Vec::new(),
                }
            })
            .collect();
        PublishedWorkout {
            workout: Workout {
                id: id.to_string(),
                path,
                title: title.to_string(),
                raw_title: title.to_string(),
                started_at_local: format!("{date} 10:38:00"),
                ended_at_local: format!("{date} 10:50:00"),
                eastern_offset_minutes: -240,
                end_eastern_offset_minutes: -240,
                duration_seconds,
                duration_suspicious: false,
                notes: None,
                description: None,
                sets,
            },
            date: date.to_string(),
            start_time,
        }
    }

    fn fitness_run(id: &str, date: &str, utc_time: &str, title: &str) -> RunningActivity {
        RunningActivity {
            id: id.to_string(),
            source: "garmin-connect".to_string(),
            source_activity_id: "24065766206".to_string(),
            source_url: Some("https://connect.garmin.com/app/activity/24065766206".to_string()),
            title: title.to_string(),
            activity_type: "running".to_string(),
            started_at_utc: format!("{date} {utc_time}"),
            started_at_local: format!("{date} 19:45:00"),
            eastern_offset_minutes: -240,
            duration_milliseconds: 2_600_000,
            moving_duration_milliseconds: Some(2_550_000),
            distance_millimeters: 6_437_376,
            ascent_millimeters: Some(91_440),
            imported_at: 1,
        }
    }

    #[test]
    fn one_item_per_log_entry() {
        let xml = rss_xml(ORIGIN, &[], &[], &[]);
        assert_eq!(xml.matches("<item>").count(), LOG.len());
        assert_eq!(xml.matches("</item>").count(), LOG.len());
    }

    #[test]
    fn wins_join_the_feed_and_losses_stay_out() {
        let runs = [
            run("1784587453", "2026-07-20", true),
            run("1784500000", "2026-07-19", false),
        ];
        let xml = rss_xml(ORIGIN, &runs, &[], &[]);
        assert_eq!(xml.matches("<item>").count(), LOG.len() + 1);
        assert!(xml.contains(&format!("{ORIGIN}/spire/run/1784587453")));
        assert!(!xml.contains("1784500000"));
        assert!(xml.contains("<pubDate>Mon, 20 Jul 2026 22:44:13 +0000</pubDate>"));
    }

    #[test]
    fn published_workouts_join_the_feed_with_archive_identity() {
        let workouts = [lift(
            "fitness:2026-07-24T14:38:00",
            "2026-07-24",
            1_784_903_880,
            "Quickest Arms in the Wesf",
            720,
            12,
            &["Incline Bench Press", "Upright Row", "MTS Biceps Curl"],
        )];
        let xml = rss_xml(ORIGIN, &[], &workouts, &[]);

        assert_eq!(xml.matches("<item>").count(), LOG.len() + 1);
        assert!(xml.contains("<title>[fitness] lift — Quickest Arms in the Wesf</title>"));
        assert!(xml.contains(&format!(
            "<link>{ORIGIN}/fitness/lift/2026-07-24T10-38-00-04-00</link>"
        )));
        assert!(xml.contains(&format!(
            "<guid isPermaLink=\"false\">{ORIGIN}/lifting/workout/fitness:2026-07-24T14:38:00</guid>"
        )));
        assert!(xml.contains(
            "<description>12 sets across 3 exercises in 12m 00s: Incline Bench Press, \
             Upright Row, and MTS Biceps Curl.</description>"
        ));
        assert!(xml.contains("<pubDate>Fri, 24 Jul 2026 14:38:00 +0000</pubDate>"));
    }

    #[test]
    fn running_activities_join_the_feed_with_archive_identity() {
        const RUN_ID: &str = "6872944a6872944a6872944a6872944a6872944a6872944a6872944a6872944a";
        let activities = [fitness_run(RUN_ID, "2026-07-20", "23:45:00", "Evening Run")];
        let xml = rss_xml(ORIGIN, &[], &[], &activities);

        assert_eq!(xml.matches("<item>").count(), LOG.len() + 1);
        assert!(xml.contains("<title>[fitness] run — Evening Run</title>"));
        assert!(xml.contains(&format!(
            "<link>{ORIGIN}/fitness/run/2026-07-20T19-45-00-04-00/{RUN_ID}</link>"
        )));
        assert!(xml.contains(&format!(
            "<guid isPermaLink=\"false\">{ORIGIN}/running/activity/{RUN_ID}</guid>"
        )));
        assert!(
            xml.contains(
                "<description>4.00 mi in 43:20 at 10:50 /mi · 300 ft ascent.</description>"
            )
        );
        assert!(xml.contains("<pubDate>Mon, 20 Jul 2026 23:45:00 +0000</pubDate>"));
    }

    #[test]
    fn items_are_sorted_newest_first_with_curated_leading_ties() {
        // One win newer than every log entry, one sharing the newest log date.
        let runs = [
            run("1784587453", "2026-07-20", true),
            run("1752300000", "2026-07-12", true),
        ];
        let xml = rss_xml(ORIGIN, &runs, &[], &[]);
        let win_new = xml.find("/spire/run/1784587453").unwrap();
        let essay = xml.find("How bad are planes?").unwrap();
        let win_tied = xml.find("/spire/run/1752300000").unwrap();
        assert!(win_new < essay, "newest win leads the feed");
        assert!(essay < win_tied, "curated entry leads a same-date win");
    }

    #[test]
    fn dynamic_items_on_one_date_sort_by_exact_start_time() {
        let runs = [run("1784587453", "2026-07-20", true)];
        let workouts = [lift(
            "fitness:2026-07-20T15:00:00",
            "2026-07-20",
            1_784_590_000,
            "Later lift",
            600,
            1,
            &["Upright Row"],
        )];
        let activities = [fitness_run(
            "run-between",
            "2026-07-20",
            "23:00:00",
            "Middle run",
        )];
        let xml = rss_xml(ORIGIN, &runs, &workouts, &activities);
        let lift = xml
            .find("/lifting/workout/fitness:2026-07-20T15:00:00")
            .unwrap();
        let activity = xml.find("/running/activity/run-between").unwrap();
        let win = xml.find("/spire/run/1784587453").unwrap();
        assert!(
            lift < activity && activity < win,
            "dynamic entries interleave by exact time, independent of kind"
        );
        assert!(xml.contains("<pubDate>Mon, 20 Jul 2026 23:26:40 +0000</pubDate>"));
        assert!(xml.contains("<pubDate>Mon, 20 Jul 2026 23:00:00 +0000</pubDate>"));
        assert!(xml.contains("<pubDate>Mon, 20 Jul 2026 22:44:13 +0000</pubDate>"));
    }

    #[test]
    fn dynamic_fields_are_escaped() {
        let runs = [run("1784587453", "2026-07-20", true)];
        let workouts = [lift(
            "fitness:2026-07-21T14:38:00",
            "2026-07-21",
            1_784_644_280,
            "Arms & <Stuff>",
            720,
            1,
            &["Curl & Press <Machine>"],
        )];
        let xml = rss_xml(ORIGIN, &runs, &workouts, &[]);
        assert!(xml.contains("Necrobinder &amp; &lt;Friends&gt;"));
        assert!(xml.contains("[fitness] lift — Arms &amp; &lt;Stuff&gt;"));
        assert!(xml.contains("Curl &amp; Press &lt;Machine&gt;"));
        assert!(!xml.contains("<Friends>"));
        assert!(!xml.contains("<Stuff>"));
    }

    #[test]
    fn no_raw_ampersands_outside_entities() {
        let runs = [run("1784587453", "2026-07-20", true)];
        let xml = rss_xml(ORIGIN, &runs, &[], &[]);
        let mut rest = xml.as_str();
        while let Some(pos) = rest.find('&') {
            let tail = &rest[pos..];
            assert!(
                ["&amp;", "&lt;", "&gt;", "&quot;", "&apos;"]
                    .iter()
                    .any(|e| tail.starts_with(e)),
                "raw ampersand near: {}",
                &tail[..tail.len().min(40)]
            );
            rest = &rest[pos + 1..];
        }
    }

    #[test]
    fn pub_dates_are_rfc2822_with_correct_weekdays() {
        // Hand-checked calendar facts.
        assert_eq!(rfc2822("2026-07-12"), "Sun, 12 Jul 2026 00:00:00 +0000");
        assert_eq!(rfc2822("2019-03-30"), "Sat, 30 Mar 2019 00:00:00 +0000");
        assert_eq!(rfc2822("2018-07-10"), "Tue, 10 Jul 2018 00:00:00 +0000");
        assert_eq!(rfc2822("2024-11-08"), "Fri, 08 Nov 2024 00:00:00 +0000");
        assert_eq!(
            rfc2822_timestamp(1_784_903_880, "1970-01-01"),
            "Fri, 24 Jul 2026 14:38:00 +0000"
        );

        // Every emitted pubDate matches the RFC 2822 shape.
        let xml = rss_xml(ORIGIN, &[run("1784587453", "2026-07-20", true)], &[], &[]);
        for line in xml.lines().filter(|l| l.starts_with("<pubDate>")) {
            let date = line
                .trim_start_matches("<pubDate>")
                .trim_end_matches("</pubDate>");
            let ok = date.len() == 31
                && ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].contains(&&date[0..3])
                && &date[3..5] == ", "
                && date[5..7].chars().all(|c| c.is_ascii_digit())
                && date.ends_with(" +0000")
                && date[17..25].chars().enumerate().all(|(index, character)| {
                    matches!(index, 2 | 5) && character == ':'
                        || !matches!(index, 2 | 5) && character.is_ascii_digit()
                });
            assert!(ok, "not RFC 2822: {date}");
        }
    }

    #[test]
    fn guids_are_unique() {
        let runs = [
            run("1784587453", "2026-07-20", true),
            run("1784400000", "2026-07-18", true),
        ];
        let workouts = [lift(
            "fitness:2026-07-21T14:38:00",
            "2026-07-21",
            1_784_644_280,
            "Lift",
            720,
            1,
            &["Curl"],
        )];
        let activities = [fitness_run("run-guid", "2026-07-22", "23:45:00", "Run")];
        let xml = rss_xml(ORIGIN, &runs, &workouts, &activities);
        let guids: Vec<&str> = xml.lines().filter(|l| l.starts_with("<guid")).collect();
        assert_eq!(guids.len(), LOG.len() + 4);
        let mut deduped = guids.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), guids.len(), "duplicate guid");
    }

    #[test]
    fn same_timestamp_in_both_games_has_distinct_guids() {
        let sts2 = run("1784587453", "2026-07-20", true);
        let mut sts1 = sts2.clone();
        sts1.game = "sts1".to_string();
        let xml = rss_xml(ORIGIN, &[sts1, sts2], &[], &[]);
        assert!(xml.contains(&format!("{ORIGIN}/spire/run/sts1/1784587453")));
        assert!(xml.contains(&format!("{ORIGIN}/spire/run/1784587453")));
    }

    #[test]
    fn note_titles_truncate_on_char_boundary() {
        assert_eq!(truncate("short", 80), "short");
        let long = "ré".repeat(60);
        let cut = truncate(&long, 80);
        assert!(cut.ends_with('…'));
        assert_eq!(cut.chars().count(), 81);
    }

    #[test]
    fn escape_covers_the_five() {
        assert_eq!(escape(r#"a&b<c>d"e'f"#), "a&amp;b&lt;c&gt;d&quot;e&apos;f");
    }

    #[test]
    fn structure_is_sound() {
        let xml = rss_xml(ORIGIN, &[], &[], &[]);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<title>Ben Berman — logbook</title>"));
        assert!(xml.contains(&format!("<link>{ORIGIN}/</link>")));
        assert!(xml.contains(&format!(
            "<atom:link href=\"{ORIGIN}/feed.xml\" rel=\"self\" type=\"application/rss+xml\"/>"
        )));
        // Internal update hrefs got the origin prefix; externals kept theirs.
        assert!(xml.contains(&format!("<link>{ORIGIN}/keyboards</link>")));
        for cover in crate::content::drum_covers::DRUM_COVERS.iter() {
            assert!(xml.contains(&format!("<link>{}</link>", cover.watch_url)));
        }
        // Serial-derived guids span 0001..=count.
        assert!(xml.contains(&format!(
            "{ORIGIN}/log/{:04}",
            crate::content::logbook::LOG.len()
        )));
        assert!(xml.contains(&format!("{ORIGIN}/log/0001")));
        assert!(xml.trim_end().ends_with("</rss>"));
    }
}
