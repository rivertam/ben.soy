//! Compact tag chrome + two-step add-filter picker for `/fitness/log`.

use super::{
    AUTO_FILTER_JS, LOG_PATH, META_LABEL, PAGE_CURRENT, PAGE_DISABLED, PAGE_GAP, PAGE_LINK,
    RESULT_COUNT,
    filters::{ActiveFilter, EQUIPMENT, Filters, MOVEMENT_DETAILS, MOVEMENTS, MUSCLES, SET_TYPES},
    results::Pager,
};
use topcoat::{
    Result,
    view::{class, component, view},
};

const TAG: &str = "inline-flex items-center gap-[0.25rem] max-w-full px-[0.55rem] \
     py-[0.3rem] font-meta text-[0.67rem] leading-[1.2] text-oxide \
     bg-oxide/6 border border-oxide/30 rounded \
     hover:border-oxide hover:underline hover:underline-offset-[0.2em] \
     focus-visible:border-oxide focus-visible:underline \
     focus-visible:underline-offset-[0.2em]";

const ADD_BTN: &str = "inline-flex items-center min-h-[1.75rem] px-[0.55rem] py-[0.3rem] \
     font-meta text-[0.67rem] leading-[1.2] text-ink2 border border-dashed \
     border-hairline rounded cursor-pointer hover:text-oxide hover:border-oxide \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";

const CAT_BTN: &str = "block w-full px-3 py-2 text-left font-meta text-[0.75rem] \
     text-ink2 rounded-[0.15rem] cursor-pointer hover:text-oxide hover:bg-oxide/6 \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-[-2px]";

const PANEL: &str = "inline-popover-panel w-[min(18rem,calc(100vw-2rem))]";

const PICK_LINK: &str = "inline-flex items-center px-[0.65rem] py-[0.35rem] font-meta \
     text-[0.7rem] leading-none text-ink2 border border-hairline rounded-full \
     hover:text-oxide hover:border-oxide focus-visible:text-oxide \
     focus-visible:border-oxide";

const COMPACT_CONTROL: &str = "w-full min-w-0 h-9 px-2.5 py-1.5 text-ink bg-page \
     border border-hairline rounded-[0.2rem] font-body text-sm leading-[1.2] outline-none \
     placeholder:text-muted focus-visible:outline-solid focus-visible:outline-2 \
     focus-visible:outline-oxide focus-visible:outline-offset-2";

/// Categories shown in the add-filter picker (numeric ranges + page size omitted).
struct Category {
    id: &'static str,
    label: &'static str,
    key: &'static str,
    kind: CategoryKind,
}

enum CategoryKind {
    /// After (`from`) + until (`to`) date fields in one panel.
    DateRange,
    Exercise,
    Chips(&'static [(&'static str, &'static str)]),
    /// Chip sections under one category (e.g. compounds / isolations).
    GroupedChips(&'static [(&'static str, &'static [(&'static str, &'static str)])]),
    Options(&'static [(&'static str, &'static str)]),
    /// Selecting the category applies `value` immediately.
    Flag(&'static str),
}

const TIME_OF_DAY: &[(&str, &str)] = &[
    ("morning", "morning · 5–11"),
    ("afternoon", "afternoon · 12–4"),
    ("evening", "evening · 5–8"),
    ("night", "night · 9–4"),
];

const WEEKDAYS: &[(&str, &str)] = &[
    ("mon", "Monday"),
    ("tue", "Tuesday"),
    ("wed", "Wednesday"),
    ("thu", "Thursday"),
    ("fri", "Friday"),
    ("sat", "Saturday"),
    ("sun", "Sunday"),
];

const MOVEMENT_GROUPS: &[(&str, &[(&str, &str)])] =
    &[("compounds", MOVEMENTS), ("isolations", MOVEMENT_DETAILS)];

const CATEGORIES: &[Category] = &[
    Category {
        id: "muscle",
        label: "muscle group",
        key: "muscle",
        kind: CategoryKind::Chips(MUSCLES),
    },
    Category {
        id: "movement",
        label: "movement",
        key: "movement",
        kind: CategoryKind::GroupedChips(MOVEMENT_GROUPS),
    },
    Category {
        id: "exercise",
        label: "exercise",
        key: "exercise",
        kind: CategoryKind::Exercise,
    },
    Category {
        id: "equipment",
        label: "equipment",
        key: "equipment",
        kind: CategoryKind::Chips(EQUIPMENT),
    },
    Category {
        id: "set-kind",
        label: "set kind",
        key: "set_type",
        kind: CategoryKind::Chips(SET_TYPES),
    },
    Category {
        id: "date",
        label: "date",
        key: "from",
        kind: CategoryKind::DateRange,
    },
    Category {
        id: "time",
        label: "time of day",
        key: "time_of_day",
        kind: CategoryKind::Options(TIME_OF_DAY),
    },
    Category {
        id: "weekday",
        label: "weekday",
        key: "weekday",
        kind: CategoryKind::Options(WEEKDAYS),
    },
    Category {
        id: "records",
        label: "personal records",
        key: "has_record",
        kind: CategoryKind::Flag("true"),
    },
    Category {
        id: "supersets",
        label: "supersets",
        key: "has_superset",
        kind: CategoryKind::Flag("true"),
    },
    Category {
        id: "notes",
        label: "with notes",
        key: "has_notes",
        kind: CategoryKind::Flag("true"),
    },
    Category {
        id: "incomplete",
        label: "incomplete rows",
        key: "incomplete",
        kind: CategoryKind::Flag("true"),
    },
    Category {
        id: "timers",
        label: "suspect timers",
        key: "duration",
        kind: CategoryKind::Flag("suspicious"),
    },
];

fn category_available(filters: &Filters, category: &Category) -> bool {
    match category.kind {
        CategoryKind::Flag(value) => !filters.contains(category.key, value),
        CategoryKind::Chips(options) | CategoryKind::Options(options) => options
            .iter()
            .any(|(value, _)| !filters.contains(category.key, value)),
        CategoryKind::GroupedChips(groups) => groups.iter().any(|(_, options)| {
            options
                .iter()
                .any(|(value, _)| !filters.contains(category.key, value))
        }),
        _ => true,
    }
}

fn chip_options<'a>(
    filters: &Filters,
    key: &str,
    options: &'a [(&'a str, &'a str)],
) -> Vec<(&'a str, &'a str, String)> {
    options
        .iter()
        .copied()
        .filter(|(value, _)| !filters.contains(key, value))
        .map(|(value, label)| (value, label, filters.adding(key, value).url(true)))
        .collect()
}

#[component]
pub(super) async fn filter_chrome(
    filters: &Filters,
    active: &[ActiveFilter],
    exercise_options: &[(String, String)],
) -> Result {
    let cat_id = "lifting-filter-categories";
    let q = filters.value("q");
    let search_carry = filters.form_carry("q");
    view! {
        <div
            class="space-y-3"
            data-lifting-filters=""
            aria-labelledby="fitness-filter-title"
        >
            <div class="flex items-baseline justify-between gap-3">
                <div class="min-w-0">
                    <p id="fitness-filter-title" class=(META_LABEL)>"filters"</p>
                </div>
                if !active.is_empty() {
                    <a
                        class="flex-none py-1 font-meta text-[0.67rem] text-oxide \
                             underline decoration-oxide/35 underline-offset-[0.25em]"
                        href=(LOG_PATH)
                    >
                        "clear"
                    </a>
                }
            </div>

            <form class="space-y-2" action="/fitness/log#set-log" method="get">
                for (name, value) in search_carry.iter() {
                    <input type="hidden" name=(name.as_str()) value=(value.as_str())>
                }
                <label class="block" for="fitness-filter-q">
                    <span class="sr-only">"search"</span>
                    <input
                        class=(COMPACT_CONTROL)
                        id="fitness-filter-q"
                        name="q"
                        type="search"
                        value=(q)
                        placeholder="exercise, workout, or note"
                        autocomplete="off"
                    >
                </label>
                <button
                    type="submit"
                    class="font-meta text-[0.67rem] text-oxide underline \
                         decoration-oxide/35 underline-offset-[0.2em] cursor-pointer"
                >
                    "search"
                </button>
            </form>

            <div
                class="flex flex-wrap items-center gap-[0.35rem]"
                aria-label="Active filters"
            >
                if active.is_empty() {
                    <span class="font-meta text-[0.67rem] text-muted">
                        "All sets"
                    </span>
                }
                for filter in active.iter() {
                    <a
                        class=(TAG)
                        href=(filter.href.as_str())
                        aria-label=(filter.aria_label.as_str())
                    >
                        <span class="min-w-0 truncate">(filter.label.as_str())</span>
                        <span aria-hidden="true">"×"</span>
                    </a>
                }
                <button
                    type="button"
                    class=(ADD_BTN)
                    hidden=""
                    data-lifting-add=""
                    popovertarget=(cat_id)
                    style="anchor-name: --lifting-filter-add;"
                >
                    "+ filter"
                </button>
            </div>

            <div
                id=(cat_id)
                class=(PANEL)
                popover="auto"
                style="position-anchor: --lifting-filter-add;"
            >
                <p class=(class!(META_LABEL, "mb-2"))>"filter by"</p>
                <div class="flex flex-col gap-0.5">
                    for category in CATEGORIES.iter().filter(|c| category_available(filters, c)) {
                        category_entry(filters: filters, category: category, cat_id: cat_id)
                    }
                </div>
            </div>

            for category in CATEGORIES.iter().filter(|c| {
                category_available(filters, c) && !matches!(c.kind, CategoryKind::Flag(_))
            }) {
                value_popover(
                    filters: filters,
                    category: category,
                    exercise_options: exercise_options
                )
            }

            <details class="group" data-lifting-filters-fallback="">
                <summary
                    class=(class!(
                        ADD_BTN,
                        "list-none [&::-webkit-details-marker]:hidden"
                    ))
                >
                    "+ filter"
                </summary>
                <div class="mt-3 space-y-2 border-t border-hairline pt-3">
                    for category in CATEGORIES.iter().filter(|c| category_available(filters, c)) {
                        fallback_category(
                            filters: filters,
                            category: category,
                            exercise_options: exercise_options
                        )
                    }
                </div>
            </details>

            <script type="module" src=(AUTO_FILTER_JS)></script>
        </div>
    }
}

#[component]
async fn category_entry(filters: &Filters, category: &Category, cat_id: &str) -> Result {
    let value_id = format!("lifting-filter-value-{}", category.id);
    match category.kind {
        CategoryKind::Flag(value) => {
            let href = filters.adding(category.key, value).url(true);
            view! {
                <a class=(CAT_BTN) href=(href.as_str())>
                    (category.label)
                </a>
            }
        }
        _ => {
            view! {
                <button
                    type="button"
                    class=(CAT_BTN)
                    popovertarget=(value_id.as_str())
                    data-lifting-category=(category.id)
                    data-lifting-close=(cat_id)
                >
                    (category.label)
                </button>
            }
        }
    }
}

#[component]
async fn value_popover(
    filters: &Filters,
    category: &Category,
    exercise_options: &[(String, String)],
) -> Result {
    let value_id = format!("lifting-filter-value-{}", category.id);
    view! {
        <div
            id=(value_id.as_str())
            class=(PANEL)
            popover="auto"
            style="position-anchor: --lifting-filter-add;"
            data-lifting-value=(category.id)
        >
            <div class="mb-2 flex items-center justify-between gap-2">
                <p class=(META_LABEL)>(category.label)</p>
                <button
                    type="button"
                    class="font-meta text-[0.67rem] text-muted hover:text-oxide \
                         cursor-pointer"
                    popovertarget=(value_id.as_str())
                    popovertargetaction="hide"
                    aria-label="Close"
                >
                    "×"
                </button>
            </div>
            value_body(
                filters: filters,
                category: category,
                exercise_options: exercise_options,
                enhanced: true
            )
        </div>
    }
}

#[component]
async fn fallback_category(
    filters: &Filters,
    category: &Category,
    exercise_options: &[(String, String)],
) -> Result {
    if let CategoryKind::Flag(value) = category.kind {
        let href = filters.adding(category.key, value).url(true);
        return view! {
            <a
                class="block py-1.5 font-meta text-[0.72rem] text-oxide \
                     underline decoration-oxide/35 underline-offset-[0.2em]"
                href=(href.as_str())
            >
                (category.label)
            </a>
        };
    }
    view! {
        <details class="group/fb">
            <summary
                class="py-1.5 list-none font-meta text-[0.72rem] text-ink2 \
                     cursor-pointer [&::-webkit-details-marker]:hidden \
                     before:content-['+'] before:inline-block before:w-4 \
                     before:text-oxide group-open/fb:before:content-['−']"
            >
                (category.label)
            </summary>
            <div class="pb-2 pl-4">
                value_body(
                    filters: filters,
                    category: category,
                    exercise_options: exercise_options,
                    enhanced: false
                )
            </div>
        </details>
    }
}

#[component]
async fn value_body(
    filters: &Filters,
    category: &Category,
    exercise_options: &[(String, String)],
    enhanced: bool,
) -> Result {
    match category.kind {
        CategoryKind::Flag(_) => view! { <span></span> },
        CategoryKind::Chips(options) => {
            let picks = chip_options(filters, category.key, options);
            view! {
                <div class="flex flex-wrap gap-[0.35rem]">
                    for (_, label, href) in picks.iter() {
                        <a class=(PICK_LINK) href=(href.as_str())>
                            (*label)
                        </a>
                    }
                </div>
            }
        }
        CategoryKind::GroupedChips(groups) => {
            let sections = groups
                .iter()
                .map(|(heading, options)| (*heading, chip_options(filters, category.key, options)))
                .filter(|(_, picks)| !picks.is_empty())
                .collect::<Vec<_>>();
            view! {
                <div class="space-y-3">
                    for (heading, picks) in sections.iter() {
                        <div class="space-y-1.5">
                            <p class=(META_LABEL)>(*heading)</p>
                            <div class="flex flex-wrap gap-[0.35rem]">
                                for (_, label, href) in picks.iter() {
                                    <a class=(PICK_LINK) href=(href.as_str())>
                                        (*label)
                                    </a>
                                }
                            </div>
                        </div>
                    }
                </div>
            }
        }
        CategoryKind::Options(options) => {
            let picks = chip_options(filters, category.key, options);
            view! {
                <div class="flex flex-col gap-0.5">
                    for (_, label, href) in picks.iter() {
                        <a class=(CAT_BTN) href=(href.as_str())>
                            (*label)
                        </a>
                    }
                </div>
            }
        }
        CategoryKind::DateRange => {
            let suffix = if enhanced { "" } else { "-fallback" };
            let from_id = format!("fitness-filter-from{suffix}");
            let to_id = format!("fitness-filter-to{suffix}");
            let carry = filters.form_carry_except(&["from", "to"]);
            let from = filters.value("from");
            let to = filters.value("to");
            view! {
                <form class="space-y-3" action="/fitness/log#set-log" method="get">
                    for (name, value) in carry.iter() {
                        <input type="hidden" name=(name.as_str()) value=(value.as_str())>
                    }
                    <label class="block space-y-1" for=(from_id.as_str())>
                        <span class=(META_LABEL)>"after"</span>
                        <input
                            class=(COMPACT_CONTROL)
                            id=(from_id.as_str())
                            name="from"
                            type="date"
                            value=(from)
                            autocomplete="off"
                        >
                    </label>
                    <label class="block space-y-1" for=(to_id.as_str())>
                        <span class=(META_LABEL)>"until"</span>
                        <input
                            class=(COMPACT_CONTROL)
                            id=(to_id.as_str())
                            name="to"
                            type="date"
                            value=(to)
                            autocomplete="off"
                        >
                    </label>
                    <button
                        type="submit"
                        class="font-meta text-[0.67rem] text-oxide underline \
                             decoration-oxide/35 underline-offset-[0.2em] cursor-pointer"
                    >
                        "apply"
                    </button>
                </form>
            }
        }
        CategoryKind::Exercise => {
            let input_id = if enhanced {
                "fitness-filter-exercise"
            } else {
                "fitness-filter-exercise-fallback"
            };
            let carry = filters.form_carry("exercise");
            view! {
                <form class="space-y-2" action="/fitness/log#set-log" method="get">
                    for (key, value) in carry.iter() {
                        <input type="hidden" name=(key.as_str()) value=(value.as_str())>
                    }
                    <label class="block" for=(input_id)>
                        <span class="sr-only">"exercise"</span>
                        <select
                            class=(class!(COMPACT_CONTROL, "pr-8"))
                            id=(input_id)
                            name="exercise"
                            required=""
                        >
                            <option value="" disabled="" selected="">
                                "choose…"
                            </option>
                            for option in exercise_options.iter() {
                                <option value=(option.0.as_str())>
                                    (option.1.as_str())
                                </option>
                            }
                        </select>
                    </label>
                    <button
                        type="submit"
                        class="font-meta text-[0.67rem] text-oxide underline \
                             decoration-oxide/35 underline-offset-[0.2em] cursor-pointer"
                    >
                        "add"
                    </button>
                </form>
            }
        }
    }
}

#[component]
async fn per_page_choice(href: &str, label: &str, current: bool) -> Result {
    if current {
        view! {
            <span class="text-ink" aria-current="true">(label)</span>
        }
    } else {
        view! {
            <a
                class="text-ink2 underline decoration-transparent underline-offset-[0.2em] \
                     hover:text-oxide hover:decoration-oxide/40 \
                     focus-visible:text-oxide focus-visible:decoration-oxide/40"
                href=(href)
            >
                (label)
            </a>
        }
    }
}

#[component]
pub(super) async fn log_pager(
    filters: &Filters,
    pager: Option<&Pager>,
    result_summary: &str,
) -> Result {
    let per_page = filters.per_page();
    let url_10 = filters.per_page_url("10");
    let url_20 = filters.per_page_url("20");
    let url_40 = filters.per_page_url("40");
    view! {
        <div class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
            <div class="min-w-0">
                <h2 class="font-display text-2xl font-semibold">"Activity log"</h2>
                <p class=(RESULT_COUNT)>(result_summary)</p>
            </div>
            <div
                class="flex flex-wrap items-center gap-x-3 gap-y-2"
                aria-label="Pagination"
            >
                <p
                    class="flex items-center gap-1.5 font-meta text-[0.67rem] text-muted"
                    aria-label="Activities per page"
                >
                    <span>"per page"</span>
                    per_page_choice(
                        href: url_10.as_str(),
                        label: "10",
                        current: per_page == "10"
                    )
                    <span aria-hidden="true">"·"</span>
                    per_page_choice(
                        href: url_20.as_str(),
                        label: "20",
                        current: per_page == "20"
                    )
                    <span aria-hidden="true">"·"</span>
                    per_page_choice(
                        href: url_40.as_str(),
                        label: "40",
                        current: per_page == "40"
                    )
                </p>
                <p class=(class!(META_LABEL, "flex-none"))>"newest first"</p>
            </div>
        </div>
        if let Some(pager) = pager {
            <nav
                class="flex flex-wrap items-center gap-[0.35rem] mt-3 font-meta text-[0.72rem]"
                aria-label="Workout log pages"
            >
                if let Some(href) = &pager.newer {
                    <a class=(PAGE_LINK) href=(href.as_str())>
                        "← newer"
                    </a>
                } else {
                    <span class=(PAGE_DISABLED) aria-disabled="true">
                        "← newer"
                    </span>
                }
                for part in pager.parts.iter() {
                    if let Some(number) = part {
                        if *number == pager.current {
                            <span class=(PAGE_CURRENT) aria-current="page">
                                (number.to_string())
                            </span>
                        } else {
                            <a class=(PAGE_LINK) href=(filters.page_url(*number))>
                                (number.to_string())
                            </a>
                        }
                    } else {
                        <span class=(PAGE_GAP)>"…"</span>
                    }
                }
                if let Some(href) = &pager.older {
                    <a class=(PAGE_LINK) href=(href.as_str())>
                        "older →"
                    </a>
                } else {
                    <span class=(PAGE_DISABLED) aria-disabled="true">
                        "older →"
                    </span>
                }
            </nav>
        }
    }
}
