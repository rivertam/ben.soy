//! Workout-specific Open Graph cards.
//!
//! Each public lift advertises a versioned PNG endpoint. The raster is built
//! from the same workout and exercise-muscle snapshot as the HTML page, so an
//! exercise rename or muscle-weight edit moves the image URL and cannot leave
//! a stale breakdown attached to a fresh page. The bitmap lettering is
//! deliberately self-contained: social-card rendering never depends on fonts
//! installed in the production image.

use std::fmt::Write;

use font8x8::{BASIC_FONTS, LATIN_FONTS, UnicodeFonts};
use resvg::{
    render,
    tiny_skia::{Paint, Pixmap, Rect, Transform},
    usvg::{Options, Tree},
};

use super::{
    archive::api::Workout,
    badge::{self, Badge},
    format::{format_duration, workout_timing},
    muscle_taxonomy,
    muscles::{BACK_PATHS, FRONT_PATHS, MuscleInvolvement, MusclePath, SILHOUETTE},
    results::{SetRow, WorkoutCard, workout_url},
};

pub(super) const WIDTH: u32 = 1200;
pub(super) const HEIGHT: u32 = 600;
pub(super) const CONTENT_TYPE: &str = "image/png";
// Bump when the rendered design changes, independently of workout data.
const RENDER_REVISION: u32 = 3;
const MAX_EXERCISES: usize = 4;
const MAX_BADGES: usize = 13;
const EXERCISE_TOP: i32 = 288;
const EXERCISE_PITCH: i32 = 64;
const BADGE_PITCH: i32 = 40;

const INK: (u8, u8, u8, u8) = (210, 218, 176, 255);
const INK_2: (u8, u8, u8, u8) = (174, 187, 135, 255);
const MUTED: (u8, u8, u8, u8) = (139, 154, 104, 255);
const OXIDE: (u8, u8, u8, u8) = (232, 163, 61, 255);

/// Render the share image. SVG remains an internal drawing description; the
/// public endpoint always returns PNG because chat clients support raster
/// Open Graph images much more consistently.
pub(super) fn render_png(workout: &Workout, involvement: &MuscleInvolvement) -> Vec<u8> {
    let copy = CardCopy::new(workout, involvement);
    let tree = Tree::from_str(&card_svg(involvement, &copy), &Options::default())
        .expect("the workout card SVG is application-authored");
    let mut pixmap = Pixmap::new(WIDTH, HEIGHT).expect("fixed social-card dimensions are valid");
    render(&tree, Transform::identity(), &mut pixmap.as_mut());
    draw_copy(&mut pixmap, &copy);
    pixmap
        .encode_png()
        .expect("fixed-size social card encodes as PNG")
}

pub(super) fn image_path(workout_path: &str, version: i64) -> String {
    format!(
        "{}/social.png?{}",
        workout_url(workout_path),
        image_query(version)
    )
}

pub(super) fn image_query(version: i64) -> String {
    format!("v={version}&r={RENDER_REVISION}")
}

pub(super) fn description(workout: &Workout, involvement: &MuscleInvolvement) -> String {
    let mut description = crate::app::feed::workout_description(workout);
    if involvement.is_empty() {
        return description;
    }

    if !involvement.primary.is_empty() {
        description.push_str(" Primary muscles: ");
        description.push_str(&human_list(&muscle_labels(&involvement.primary)));
        description.push('.');
    }
    if !involvement.secondary.is_empty() {
        description.push_str(" Secondary muscles: ");
        description.push_str(&human_list(&muscle_labels(&involvement.secondary)));
        description.push('.');
    }
    description
}

pub(super) fn image_alt(workout: &Workout, involvement: &MuscleInvolvement) -> String {
    format!(
        "Workout card for {}. {}",
        workout.title,
        description(workout, involvement)
    )
}

fn draw_copy(pixmap: &mut Pixmap, copy: &CardCopy<'_>) {
    draw_text(pixmap, "BEN.SOY / FITNESS / LIFT", 88, 68, 3, MUTED);
    draw_text_lines(pixmap, &copy.title, 88, 142, 3, 32, INK);

    draw_text(pixmap, &copy.date, 88, 220, 2, OXIDE);
    draw_text(pixmap, &copy.facts, 88, 250, 2, INK_2);
    for (index, exercise) in copy.exercises.iter().enumerate() {
        let y = EXERCISE_TOP + index as i32 * EXERCISE_PITCH;
        draw_text(pixmap, &exercise.name, 88, y, 2, INK_2);
        for (index, row) in exercise.rows.iter().enumerate() {
            let number = row
                .working_number
                .map_or_else(|| "W".into(), |n| format!("{n:02}"));
            let color = badge_color(badge::badge_for(row.set));
            let scale = 1.25_f32.min(18.0 / (number.len() as f32 * 7.0));
            draw_text_sized(
                pixmap,
                &number,
                (88 + index as i32 * BADGE_PITCH + 18) as f32
                    - number.len() as f32 * 7.0 * scale / 2.0,
                (y + 22 + 18) as f32 - 8.0 * scale / 2.0,
                scale,
                color,
            );
        }
        if exercise.omitted_sets > 0 {
            draw_text(
                pixmap,
                &format!("+{}", exercise.omitted_sets),
                88 + exercise.rows.len() as i32 * BADGE_PITCH,
                y + 33,
                2,
                MUTED,
            );
        }
    }
    if copy.omitted_exercises > 0 {
        draw_text(
            pixmap,
            &format!(
                "+{} MORE {}",
                copy.omitted_exercises,
                plural(copy.omitted_exercises, "EXERCISE", "EXERCISES")
            ),
            88,
            546,
            2,
            MUTED,
        );
    }

    draw_text(pixmap, "MUSCLE BREAKDOWN", 718, 68, 2, MUTED);
    draw_text(pixmap, "FRONT", 763, 399, 2, MUTED);
    draw_text(pixmap, "BACK", 929, 399, 2, MUTED);

    if copy.primary_count > 0 {
        draw_text(
            pixmap,
            &format!("PRIMARY / {}", copy.primary_count),
            718,
            422,
            2,
            OXIDE,
        );
        draw_text_lines(pixmap, &copy.primary, 718, 448, 2, 22, INK_2);
    }
    if copy.secondary_count > 0 {
        draw_text(
            pixmap,
            &format!("SECONDARY / {}", copy.secondary_count),
            718,
            498,
            2,
            MUTED,
        );
        draw_text_lines(pixmap, &copy.secondary, 718, 524, 2, 22, INK_2);
    }
}

fn card_svg(involvement: &MuscleInvolvement, copy: &CardCopy<'_>) -> String {
    let panel_height = HEIGHT - 56;
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}">
<defs>
  <pattern id="grid" width="48" height="48" patternUnits="userSpaceOnUse">
    <path d="M48 0H0V48" fill="none" stroke="#465232" stroke-width="1" opacity="0.42"/>
  </pattern>
  <linearGradient id="page" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#333c29"/>
    <stop offset="1" stop-color="#283021"/>
  </linearGradient>
</defs>
<rect width="{WIDTH}" height="{HEIGHT}" fill="#1e241a"/>
<rect x="30" y="28" width="1140" height="{panel_height}" rx="8" fill="url(#page)"/>
<rect x="30" y="28" width="1140" height="{panel_height}" rx="8" fill="url(#grid)"/>
<rect x="30" y="28" width="1140" height="8" rx="4" fill="#e8a33d"/>
<path d="M88 112H1112" stroke="#465232" stroke-width="2"/>
<path d="M675 132V526" stroke="#465232" stroke-width="2"/>
<g transform="translate(700 121) scale(0.72)">"##
    );
    push_figure(&mut svg, FRONT_PATHS, involvement);
    svg.push_str("</g><g transform=\"translate(868 121) scale(0.72)\">");
    push_figure(&mut svg, BACK_PATHS, involvement);
    svg.push_str("</g>");
    for (index, exercise) in copy.exercises.iter().enumerate() {
        let y = EXERCISE_TOP + index as i32 * EXERCISE_PITCH + 22;
        for (index, row) in exercise.rows.iter().enumerate() {
            let (red, green, blue, _) = badge_color(badge::badge_for(row.set));
            write!(
                svg,
                r#"<g transform="translate({} {y})" color="rgb({red},{green},{blue})">{}</g>"#,
                88 + index as i32 * BADGE_PITCH,
                badge::seal_shapes(row.set, "#2e3626"),
            )
            .expect("write to string");
        }
    }
    svg.push_str("</svg>");
    svg
}

fn badge_color(badge: Badge) -> (u8, u8, u8, u8) {
    match badge {
        Badge::Warmup => (145, 177, 191, 255),
        Badge::Failure => (222, 193, 108, 255),
        Badge::Rated | Badge::Unrated => INK_2,
    }
}

fn push_figure(svg: &mut String, paths: &'static [MusclePath], involvement: &MuscleInvolvement) {
    write!(
        svg,
        r##"<path fill="#303a28" stroke="#596840" stroke-width="1.5" d="{SILHOUETTE}"/>"##
    )
    .expect("write to string");
    for path in paths {
        let fill = if involvement.primary.contains(&path.muscle) {
            "#e8a33d"
        } else if involvement.secondary.contains(&path.muscle) {
            "#8f6634"
        } else {
            "#3a452f"
        };
        write!(
            svg,
            r##"<path fill="{fill}" stroke="#596840" stroke-width="0.75" d="{}"/>"##,
            path.d
        )
        .expect("write to string");
    }
}

struct CardExercise<'a> {
    name: String,
    rows: Vec<SetRow<'a>>,
    omitted_sets: usize,
}

struct CardCopy<'a> {
    title: Vec<String>,
    date: String,
    facts: String,
    exercises: Vec<CardExercise<'a>>,
    omitted_exercises: usize,
    primary_count: usize,
    primary: Vec<String>,
    secondary_count: usize,
    secondary: Vec<String>,
}

impl<'a> CardCopy<'a> {
    fn new(workout: &'a Workout, involvement: &MuscleInvolvement) -> Self {
        let timing = workout_timing(
            &workout.started_at_local,
            &workout.ended_at_local,
            workout.eastern_offset_minutes,
            workout.end_eastern_offset_minutes,
        );
        let groups: Vec<_> = WorkoutCard::from(workout)
            .blocks
            .into_iter()
            .flat_map(|block| block.groups)
            .collect();
        let omitted_exercises = groups.len().saturating_sub(MAX_EXERCISES);
        let exercises = groups
            .into_iter()
            .take(MAX_EXERCISES)
            .map(|group| {
                let shown = if group.rows.len() > MAX_BADGES {
                    MAX_BADGES - 1
                } else {
                    group.rows.len()
                };
                let omitted_sets = group.rows.len() - shown;
                CardExercise {
                    name: wrap_text(group.name, 40, 1)
                        .into_iter()
                        .next()
                        .unwrap_or_default(),
                    rows: group.rows.into_iter().take(shown).collect(),
                    omitted_sets,
                }
            })
            .collect();
        let working_sets = workout
            .sets
            .iter()
            .filter(|set| set.set_type != "WARMUP_SET")
            .count();
        let primary = muscle_labels(&involvement.primary);
        let secondary = muscle_labels(&involvement.secondary);

        Self {
            title: wrap_text(&workout.title, 26, 2),
            date: card_text(&timing.date),
            facts: card_text(&format!(
                "{} / {} WORKING {}",
                format_duration(workout.duration_seconds),
                working_sets,
                plural(working_sets, "set", "sets"),
            )),
            exercises,
            omitted_exercises,
            primary_count: primary.len(),
            primary: wrap_items(&primary, 28, 2),
            secondary_count: secondary.len(),
            secondary: wrap_items(&secondary, 28, 2),
        }
    }
}

fn muscle_labels(ids: &[&'static str]) -> Vec<&'static str> {
    ids.iter()
        .filter_map(|id| muscle_taxonomy::muscle_label(id))
        .collect()
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

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// Pack slash-separated labels without splitting a muscle name. When every
/// label cannot fit, keep whole leading labels and state how many remain; the
/// card's front/back map still visualizes the complete set.
fn wrap_items(values: &[&str], max_chars: usize, max_lines: usize) -> Vec<String> {
    if values.is_empty() || max_chars == 0 || max_lines == 0 {
        return Vec::new();
    }

    let values: Vec<String> = values.iter().map(|value| card_text(value)).collect();
    let mut lines: Vec<Vec<String>> = Vec::new();
    let mut included = 0;
    for value in &values {
        let current = lines.last().map_or(0, |line| item_line_len(line));
        let candidate = current + usize::from(current > 0) * 3 + value.chars().count();
        if candidate <= max_chars {
            if lines.is_empty() {
                lines.push(Vec::new());
            }
            lines.last_mut().unwrap().push(value.clone());
            included += 1;
        } else if lines.len() < max_lines && value.chars().count() <= max_chars {
            lines.push(vec![value.clone()]);
            included += 1;
        } else {
            break;
        }
    }

    if included == values.len() {
        return lines.into_iter().map(|line| line.join(" / ")).collect();
    }

    let mut omitted = values.len() - included;
    let last = lines
        .last_mut()
        .expect("a canonical muscle label fits the card width");
    loop {
        let suffix = format!(" / +{omitted}");
        if item_line_len(last) + suffix.chars().count() <= max_chars {
            let mut rendered = lines
                .into_iter()
                .map(|line| line.join(" / "))
                .collect::<Vec<_>>();
            rendered.last_mut().unwrap().push_str(&suffix);
            return rendered;
        }
        last.pop()
            .expect("one canonical muscle label plus an omitted count fits the card width");
        omitted += 1;
    }
}

fn item_line_len(values: &[String]) -> usize {
    values
        .iter()
        .map(|value| value.chars().count())
        .sum::<usize>()
        + values.len().saturating_sub(1) * 3
}

fn wrap_text(value: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let value = card_text(value);
    if value.is_empty() || max_chars == 0 || max_lines == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut truncated = false;
    for word in value.split_whitespace() {
        let mut pieces = split_word(word, max_chars).into_iter().peekable();
        while let Some(piece) = pieces.next() {
            let separator = usize::from(!current.is_empty());
            if current.chars().count() + separator + piece.chars().count() <= max_chars {
                if separator == 1 {
                    current.push(' ');
                }
                current.push_str(&piece);
            } else {
                lines.push(std::mem::take(&mut current));
                if lines.len() == max_lines {
                    truncated = true;
                    break;
                }
                current.push_str(&piece);
            }
            if pieces.peek().is_some() {
                lines.push(std::mem::take(&mut current));
                if lines.len() == max_lines {
                    truncated = true;
                    break;
                }
            }
        }
        if truncated {
            break;
        }
    }
    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    }
    if truncated && let Some(last) = lines.last_mut() {
        truncate_with_ellipsis(last, max_chars);
    }
    lines
}

fn split_word(word: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    chars
        .chunks(max_chars)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn truncate_with_ellipsis(value: &mut String, max_chars: usize) {
    let keep = max_chars.saturating_sub(3);
    *value = value.chars().take(keep).collect();
    value.push_str("...");
}

fn card_text(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            'a'..='z' => character.to_uppercase().collect::<Vec<_>>(),
            'A'..='Z'
            | '0'..='9'
            | ' '
            | '!'
            | '"'
            | '#'
            | '$'
            | '%'
            | '&'
            | '\''
            | '('
            | ')'
            | '*'
            | '+'
            | ','
            | '-'
            | '.'
            | '/'
            | ':'
            | ';'
            | '<'
            | '='
            | '>'
            | '?'
            | '@'
            | '['
            | '\\'
            | ']'
            | '^'
            | '_'
            | '`'
            | '{'
            | '|'
            | '}'
            | '~' => {
                vec![character]
            }
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => {
                vec!['A']
            }
            'ç' | 'Ç' => vec!['C'],
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => vec!['E'],
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => vec!['I'],
            'ñ' | 'Ñ' => vec!['N'],
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => vec!['O'],
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => vec!['U'],
            'ý' | 'ÿ' | 'Ý' => vec!['Y'],
            '’' | '‘' => vec!['\''],
            '–' | '—' | '−' => vec!['-'],
            '·' | '•' => vec!['/'],
            '×' => vec!['X'],
            character if character.is_whitespace() => vec![' '],
            _ => vec!['?'],
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn draw_text_lines(
    pixmap: &mut Pixmap,
    lines: &[String],
    x: i32,
    y: i32,
    scale: u32,
    line_height: i32,
    color: (u8, u8, u8, u8),
) {
    for (index, line) in lines.iter().enumerate() {
        draw_text(
            pixmap,
            line,
            x,
            y + index as i32 * line_height,
            scale,
            color,
        );
    }
}

fn draw_text(
    pixmap: &mut Pixmap,
    value: &str,
    x: i32,
    y: i32,
    scale: u32,
    color: (u8, u8, u8, u8),
) {
    draw_text_sized(pixmap, value, x as f32, y as f32, scale as f32, color);
}

fn draw_text_sized(
    pixmap: &mut Pixmap,
    value: &str,
    x: f32,
    y: f32,
    scale: f32,
    color: (u8, u8, u8, u8),
) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.0, color.1, color.2, color.3);
    paint.anti_alias = scale.fract() != 0.0;
    let advance = scale * 7.0;

    for (index, character) in card_text(value).chars().enumerate() {
        let glyph = BASIC_FONTS
            .get(character)
            .or_else(|| LATIN_FONTS.get(character))
            .or_else(|| BASIC_FONTS.get('?'))
            .expect("the fallback glyph exists");
        let origin_x = x + index as f32 * advance;
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0_u32..8 {
                if *bits & (1_u8 << column) == 0 {
                    continue;
                }
                let rect = Rect::from_xywh(
                    origin_x + column as f32 * scale,
                    y + row as f32 * scale,
                    scale,
                    scale,
                )
                .expect("positive glyph rectangle");
                pixmap.fill_rect(rect, &paint, Transform::identity(), None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interests::lifting::archive::api::Set;

    fn workout() -> Workout {
        Workout {
            id: "fitness:2026-09-07T15:06:51".into(),
            path: "2026-09-07T11-06-51-04-00".into(),
            title: "Laser Beans & Friends".into(),
            raw_title: "Laser Beans & Friends".into(),
            started_at_local: "2026-09-07 11:06:51".into(),
            ended_at_local: "2026-09-07 11:36:39".into(),
            eastern_offset_minutes: -240,
            end_eastern_offset_minutes: -240,
            duration_seconds: 1788,
            duration_suspicious: false,
            notes: None,
            description: None,
            sets: vec![
                set("Incline Bench Press", 1, "WARMUP_SET", None),
                set("Incline Bench Press", 2, "NORMAL_SET", Some(950)),
                set("Barbell Zercher Squat", 3, "NORMAL_SET", Some(900)),
            ],
        }
    }

    fn set(name: &str, ordinal: u32, set_type: &str, effort: Option<u64>) -> Set {
        Set {
            id: format!("s{ordinal}"),
            ordinal,
            exercise_name: name.into(),
            raw_exercise_name: name.into(),
            exercise_note: None,
            superset_id: None,
            weight_milli: Some(100_000),
            weight_unit: "lbs".into(),
            reps: Some(5),
            effort_hundredths: effort,
            failure: false,
            distance_milli: None,
            set_time_seconds: None,
            set_type: set_type.into(),
            records: Vec::new(),
        }
    }

    fn involvement() -> MuscleInvolvement {
        MuscleInvolvement {
            primary: vec!["upper-chest", "quads", "glute-max"],
            secondary: vec![
                "anterior-delts",
                "upper-traps",
                "spinal-erectors",
                "mid-chest",
                "serratus-anterior",
                "biceps",
                "triceps",
                "abs",
                "obliques",
                "hamstrings",
                "adductors",
                "glute-med",
                "gastrocnemius",
            ],
        }
    }

    #[test]
    fn metadata_copy_names_the_full_muscle_breakdown() {
        let description = description(&workout(), &involvement());
        assert!(description.contains("3 sets across 2 exercises in 29m 48s"));
        assert!(description.contains("Primary muscles: upper chest, quads, and glute max."));
        assert!(description.contains(
            "Secondary muscles: anterior delts, upper traps, spinal erectors, mid chest, \
             serratus anterior, biceps, triceps, abs, obliques, hamstrings, adductors, \
             glute med, and gastrocnemius."
        ));
        assert!(
            image_alt(&workout(), &involvement())
                .starts_with("Workout card for Laser Beans & Friends. 3 sets across 2 exercises")
        );
    }

    #[test]
    fn image_path_is_versioned_and_uses_the_canonical_workout_url() {
        assert_eq!(
            image_path("2026-09-07T11-06-51-04-00", 149),
            "/fitness/lift/2026-09-07T11-06-51-04-00/social.png?v=149&r=3"
        );
    }

    #[test]
    fn raster_is_a_large_png_with_the_expected_dimensions() {
        let png = render_png(&workout(), &involvement());
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), WIDTH);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), HEIGHT);
        assert!(png.len() < 5 * 1024 * 1024);
    }

    #[test]
    fn image_badges_share_the_page_geometry_and_working_numbers() {
        let mut workout = workout();
        let mut failure = set("Incline Bench Press", 4, "NORMAL_SET", None);
        failure.failure = true;
        workout.sets.push(failure);
        let involvement = involvement();
        let copy = CardCopy::new(&workout, &involvement);
        let numbers: Vec<_> = copy
            .exercises
            .iter()
            .flat_map(|exercise| exercise.rows.iter().map(|row| row.working_number))
            .collect();
        assert_eq!(numbers, [None, Some(1), Some(1), Some(2)]);
        assert!(copy.facts.contains("3 WORKING SETS"));
        let svg = card_svg(&involvement, &copy);
        assert_eq!(svg.matches("class=\"lift-set-bar\"").count(), 12);
        assert!(svg.contains(&badge::seal_shapes(&workout.sets[3], "#2e3626")));
        assert!(!svg.contains("volume points"));
    }

    #[test]
    fn large_workouts_have_explicit_overflow_counts() {
        let mut workout = workout();
        workout.sets = (1..=20)
            .map(|n| set("Bench", n, "NORMAL_SET", Some(1_000)))
            .collect();
        for (i, name) in ["Row", "Squat", "Curl", "Press", "Carry"]
            .iter()
            .enumerate()
        {
            workout
                .sets
                .push(set(name, 21 + i as u32, "NORMAL_SET", None));
        }
        let involvement = involvement();
        let copy = CardCopy::new(&workout, &involvement);
        assert_eq!(copy.exercises.len(), 4);
        assert_eq!(copy.omitted_exercises, 2);
        assert_eq!(copy.exercises[0].rows.len(), 12);
        assert_eq!(copy.exercises[0].omitted_sets, 8);
        assert_eq!(copy.exercises[1].rows[0].working_number, Some(1));
        assert!(copy.facts.contains("25 WORKING SETS"));
    }

    #[test]
    fn wrapping_and_card_text_are_bounded_and_safe() {
        assert_eq!(card_text("Ben’s café — 3×5 💪"), "BEN'S CAFE - 3X5 ?");
        assert_eq!(
            wrap_items(&["upper chest", "quads", "glute max"], 28, 2),
            ["UPPER CHEST / QUADS", "GLUTE MAX"]
        );
        assert_eq!(
            wrap_items(
                &[
                    "anterior delts",
                    "upper traps",
                    "spinal erectors",
                    "abs",
                    "obliques",
                ],
                28,
                2,
            ),
            ["ANTERIOR DELTS / UPPER TRAPS", "SPINAL ERECTORS / ABS / +1",]
        );
        let lines = wrap_text(
            "An extraordinarily long workout title that cannot fit on one line",
            20,
            2,
        );
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.chars().count() <= 20));
        assert!(lines.last().unwrap().ends_with("..."));
    }
}
