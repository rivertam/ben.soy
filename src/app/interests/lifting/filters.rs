//! Native GET-query normalization, filter labels, and shareable lifting URLs.

use crate::util::urlencode;

const DEFAULT_PER_PAGE: &str = "10";
pub(super) const LOG_PATH: &str = "/lifting/log";

pub(super) const MOVEMENTS: &[(&str, &str)] = &[
    ("squat-type", "squat-type"),
    ("hinge", "hinge"),
    ("horizontal-push", "horizontal push"),
    ("vertical-push", "vertical push"),
    ("horizontal-pull", "horizontal pull"),
    ("vertical-pull", "vertical pull"),
    ("core", "core"),
    ("carry", "carry"),
    ("cardio", "cardio"),
    ("olympic-lift", "Olympic lift"),
];

pub(super) const MOVEMENT_DETAILS: &[(&str, &str)] = &[
    ("dip", "dip"),
    ("fly", "fly"),
    ("elbow-flexion", "elbow flexion"),
    ("elbow-extension", "elbow extension"),
    ("shoulder-abduction", "shoulder abduction"),
    ("shoulder-flexion", "shoulder flexion"),
    ("shoulder-extension", "shoulder extension"),
    ("rear-delt", "rear delt"),
    ("shrug", "shrug"),
    ("knee-flexion", "knee flexion"),
    ("knee-extension", "knee extension"),
    ("hip-abduction", "hip abduction"),
    ("hip-adduction", "hip adduction"),
    ("calf-raise", "calf raise"),
    ("grip-wrist", "grip / wrist"),
    ("throw", "throw"),
];

pub(super) const MUSCLES: &[(&str, &str)] = &[
    ("quads", "quads"),
    ("glutes", "glutes"),
    ("hamstrings", "hamstrings"),
    ("calves", "calves"),
    ("chest", "chest"),
    ("back", "back"),
    ("shoulders", "shoulders"),
    ("biceps", "biceps"),
    ("triceps", "triceps"),
    ("forearms", "forearms"),
    ("traps", "traps"),
    ("adductors", "adductors"),
    ("core", "core"),
];

pub(super) const EQUIPMENT: &[(&str, &str)] = &[
    ("barbell", "barbell"),
    ("dumbbell", "dumbbell"),
    ("cable", "cable"),
    ("machine", "machine"),
    ("smith-machine", "smith machine"),
    ("bodyweight", "bodyweight"),
    ("landmine", "landmine"),
    ("rings", "rings"),
    ("sandbag", "sandbag"),
    ("medicine-ball", "medicine ball"),
];

pub(super) const SET_TYPES: &[(&str, &str)] = &[
    ("NORMAL_SET", "working"),
    ("WARMUP_SET", "warm-up"),
    ("FAILURE_SET", "failure"),
    ("PARTIAL_REPS_SET", "partial reps"),
    ("NEGATIVE_REPS_SET", "negative reps"),
    ("DROP_SET", "drop set"),
];

const ADVANCED_KEYS: &[&str] = &[
    "from",
    "to",
    "time_of_day",
    "weekday",
    "muscle",
    "equipment",
    "set_type",
    "min_load",
    "max_load",
    "min_reps",
    "max_reps",
    "max_effort",
    "has_record",
    "has_superset",
    "has_notes",
    "incomplete",
    "duration",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Filters {
    pairs: Vec<(String, String)>,
}

impl Filters {
    /// Match native `FormData` normalization: trim values and drop blanks.
    /// Canonical page/default-size params are omitted only when singular, so
    /// duplicated non-empty params still reach the Worker's validator.
    ///
    /// Control characters are rejected before rendering. Topcoat correctly
    /// escapes markup, but HTML escaping does not normalize raw control bytes.
    pub(super) fn normalize(raw: Vec<(String, String)>) -> Option<Self> {
        if raw.iter().any(|(key, value)| {
            key.chars().any(char::is_control) || value.chars().any(char::is_control)
        }) {
            return None;
        }

        let mut pairs: Vec<(String, String)> = raw
            .into_iter()
            .filter_map(|(key, value)| {
                let value = value.trim();
                (!value.is_empty()).then(|| (key, value.to_string()))
            })
            .collect();
        let page_count = pairs.iter().filter(|(key, _)| key == "page").count();
        let per_page_count = pairs.iter().filter(|(key, _)| key == "per_page").count();
        pairs.retain(|(key, value)| {
            !(key == "page" && page_count == 1 && value == "1")
                && !(key == "per_page" && per_page_count == 1 && value == DEFAULT_PER_PAGE)
        });
        Some(Self { pairs })
    }

    pub(super) fn value(&self, key: &str) -> &str {
        self.pairs
            .iter()
            .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
            .unwrap_or("")
    }

    pub(super) fn contains(&self, key: &str, value: &str) -> bool {
        self.pairs
            .iter()
            .any(|(candidate, candidate_value)| candidate == key && candidate_value == value)
    }

    pub(super) fn per_page(&self) -> &str {
        let value = self.value("per_page");
        if value.is_empty() {
            DEFAULT_PER_PAGE
        } else {
            value
        }
    }

    pub(super) fn api_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = self.pairs.clone();
        if !pairs.iter().any(|(key, _)| key == "per_page") {
            pairs.push(("per_page".to_string(), DEFAULT_PER_PAGE.to_string()));
        }
        pairs
    }

    pub(super) fn query(&self) -> String {
        self.pairs
            .iter()
            .map(|(key, value)| format!("{}={}", urlencode(key), urlencode(value)))
            .collect::<Vec<_>>()
            .join("&")
    }

    pub(super) fn url(&self, fragment: bool) -> String {
        let query = self.query();
        let mut url = if query.is_empty() {
            LOG_PATH.to_string()
        } else {
            format!("{LOG_PATH}?{query}")
        };
        if fragment {
            url.push_str("#set-log");
        }
        url
    }

    pub(super) fn advanced(&self) -> bool {
        self.pairs.iter().any(|(key, value)| {
            ADVANCED_KEYS.contains(&key.as_str())
                || (key == "movement" && MOVEMENT_DETAILS.iter().any(|(detail, _)| detail == value))
        })
    }

    pub(super) fn active(&self) -> Vec<ActiveFilter> {
        self.pairs
            .iter()
            .enumerate()
            .filter(|(_, (key, _))| key != "page" && key != "per_page")
            .map(|(index, (key, value))| {
                let mut remaining = self
                    .pairs
                    .iter()
                    .enumerate()
                    .filter(|(candidate, (candidate_key, _))| {
                        *candidate != index && candidate_key != "page"
                    })
                    .map(|(_, pair)| pair.clone())
                    .collect::<Vec<_>>();
                // Removing a filter always resets pagination; normalization
                // also keeps the default page size out of the resulting URL.
                remaining.retain(|(candidate_key, candidate_value)| {
                    candidate_key != "page"
                        && !(candidate_key == "per_page" && candidate_value == DEFAULT_PER_PAGE)
                });
                let remaining = Self { pairs: remaining };
                let label = active_filter_label(key, value);
                ActiveFilter {
                    aria_label: format!("Remove {label} filter"),
                    label,
                    href: remaining.url(true),
                }
            })
            .collect()
    }

    /// The active filters a heatmap day link should carry: everything
    /// except `from`/`to` (the day supplies those) and `page` (a narrower
    /// result set restarts at page one).
    pub(super) fn day_link_query(&self) -> String {
        let pairs = self
            .pairs
            .iter()
            .filter(|(key, _)| key != "from" && key != "to" && key != "page")
            .cloned()
            .collect();
        Self { pairs }.query()
    }

    pub(super) fn page_url(&self, page: usize) -> String {
        let mut pairs = self
            .pairs
            .iter()
            .filter(|(key, _)| key != "page")
            .cloned()
            .collect::<Vec<_>>();
        if page > 1 {
            pairs.push(("page".to_string(), page.to_string()));
        }
        Self { pairs }.url(true)
    }
}

pub(super) struct ActiveFilter {
    pub(super) label: String,
    pub(super) aria_label: String,
    pub(super) href: String,
}

fn active_filter_label(key: &str, value: &str) -> String {
    match key {
        "has_record" => bool_label(value, "personal records"),
        "has_superset" => bool_label(value, "supersets"),
        "has_notes" => bool_label(value, "with notes"),
        "incomplete" => bool_label(value, "incomplete rows"),
        "duration" => match value {
            "suspicious" => "suspect timers".to_string(),
            "normal" => "normal timers".to_string(),
            _ => format!("timer: {value}"),
        },
        _ => {
            let key_label = match key {
                "q" => "search",
                "exercise" => "exercise",
                "movement" => "movement",
                "muscle" => "muscle",
                "equipment" => "equipment",
                "set_type" => "set kind",
                "from" => "from",
                "to" => "through",
                "time_of_day" => "time",
                "weekday" => "day",
                "min_load" => "load \u{2265}",
                "max_load" => "load \u{2264}",
                "min_reps" => "reps \u{2265}",
                "max_reps" => "reps \u{2264}",
                "max_effort" => "RPE \u{2264}",
                _ => key,
            };
            format!("{key_label}: {}", filter_value_label(key, value))
        }
    }
}

fn bool_label(value: &str, label: &str) -> String {
    if value == "false" {
        format!("not {label}")
    } else {
        label.to_string()
    }
}

fn filter_value_label<'a>(key: &str, value: &'a str) -> &'a str {
    match key {
        "movement" => lookup(MOVEMENTS, value)
            .or_else(|| lookup(MOVEMENT_DETAILS, value))
            .unwrap_or(value),
        "muscle" => lookup(MUSCLES, value).unwrap_or(value),
        "equipment" => lookup(EQUIPMENT, value).unwrap_or(value),
        "set_type" => lookup(SET_TYPES, value).unwrap_or(value),
        "time_of_day" => match value {
            "morning" => "morning",
            "afternoon" => "afternoon",
            "evening" => "evening",
            "night" => "night",
            _ => value,
        },
        "weekday" => match value {
            "mon" => "Monday",
            "tue" => "Tuesday",
            "wed" => "Wednesday",
            "thu" => "Thursday",
            "fri" => "Friday",
            "sat" => "Saturday",
            "sun" => "Sunday",
            _ => value,
        },
        _ => value,
    }
}

pub(super) fn lookup<'a>(options: &'a [(&str, &str)], value: &str) -> Option<&'a str> {
    options
        .iter()
        .find_map(|(candidate, label)| (*candidate == value).then_some(*label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_form_queries_drop_blanks_and_keep_repeated_facets() {
        let filters = Filters::normalize(vec![
            ("q".into(), "  ".into()),
            ("exercise".into(), " Squat ".into()),
            ("movement".into(), "squat-type".into()),
            ("movement".into(), "hinge".into()),
            ("per_page".into(), "10".into()),
            ("page".into(), "1".into()),
        ])
        .expect("safe query");
        assert_eq!(
            filters.pairs,
            vec![
                ("exercise".into(), "Squat".into()),
                ("movement".into(), "squat-type".into()),
                ("movement".into(), "hinge".into()),
            ]
        );
        assert_eq!(
            filters.query(),
            "exercise=Squat&movement=squat-type&movement=hinge"
        );
        assert!(
            filters
                .api_pairs()
                .contains(&("per_page".into(), "10".into()))
        );
    }

    #[test]
    fn duplicate_singular_params_reach_the_api_validator() {
        let filters = Filters::normalize(vec![
            ("page".into(), "1".into()),
            ("page".into(), "2".into()),
            ("per_page".into(), "10".into()),
            ("per_page".into(), "20".into()),
        ])
        .expect("safe query");
        assert_eq!(filters.pairs.len(), 4);
    }

    #[test]
    fn pager_links_preserve_filters_and_reset_page_one() {
        let filters = Filters::normalize(vec![
            ("movement".into(), "squat-type".into()),
            ("page".into(), "4".into()),
        ])
        .expect("safe query");
        assert_eq!(
            filters.page_url(1),
            "/lifting/log?movement=squat-type#set-log"
        );
        assert_eq!(
            filters.page_url(3),
            "/lifting/log?movement=squat-type&page=3#set-log"
        );
    }

    #[test]
    fn day_link_query_drops_dates_and_pagination_only() {
        let filters = Filters::normalize(vec![
            ("movement".into(), "squat-type".into()),
            ("from".into(), "2026-01-01".into()),
            ("to".into(), "2026-02-01".into()),
            ("per_page".into(), "40".into()),
            ("page".into(), "3".into()),
        ])
        .expect("safe query");
        assert_eq!(filters.day_link_query(), "movement=squat-type&per_page=40");
        assert_eq!(
            Filters::normalize(vec![("from".into(), "2026-01-01".into())])
                .expect("safe query")
                .day_link_query(),
            ""
        );
    }

    #[test]
    fn control_characters_never_reach_html_or_the_api() {
        assert!(Filters::normalize(vec![("q".into(), "nul\0byte".into())]).is_none());
        assert!(Filters::normalize(vec![("q\u{7f}".into(), "safe".into())]).is_none());
        assert!(Filters::normalize(vec![("q".into(), "line\nbreak".into())]).is_none());
    }
}
