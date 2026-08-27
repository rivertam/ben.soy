//! The running timeline — curated logbook entries, synced Slay the Spire
//! wins, published lifts, and fitness runs, filterable and searchable. It
//! renders as the log pane of `/` (`home.rs` owns the page and its cache
//! header); `/log`, its address for a while in 2026, permanently redirects
//! home with any filter query intact.

use benjisponge::data::{Data, running_models::RunningActivity};

use super::feed::workout_volume_points;
use super::interests::lifting::archive::snapshot::PublishedWorkout;
use super::interests::lifting::archive::store::FitnessStore;
use super::interests::running;
use super::interests::spire::run_page_url;
use super::interests::spire::runs::{self as spire_runs, Run};
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{error::redirect_permanent, query_params, request::uri, route},
    view::{component, view},
};

use crate::{
    app::login::viewer,
    components::link_label,
    content::{
        access::is_admin,
        interests::{INTERESTS, Interest},
        logbook::{Entry, FILTER_TAGS, Kind, LOG, serial},
    },
    util::urlencode,
};

/// The interests the hero cycles through.
fn hero_words() -> Vec<&'static Interest> {
    INTERESTS.iter().collect()
}

/// The hero's longest rendered line, in ems: "I like {title}." at Fira
/// Mono's fixed 0.6em glyph advance. The hero never wraps, so logbook.css
/// divides this out of the column width to cap the font at exactly-fits
/// (`--log-hero-fit`); phones would otherwise scroll sideways whenever the
/// rotation lands on a long title. Proportional theme faces (Comic Sans,
/// Zilla Slab) average narrower than mono, so this bound holds everywhere.
fn hero_line_ems(words: &[&'static Interest]) -> f64 {
    words
        .iter()
        .map(|i| {
            format!("I like {}.", i.title.to_lowercase())
                .chars()
                .count()
        })
        .max()
        .unwrap_or(0) as f64
        * 0.6
}

#[query_params(error = redirect("?"))]
struct LogQuery {
    kind: Option<String>,
    tag: Option<String>,
    q: Option<String>,
    /// Kept as a string and parsed leniently — `?page=banana` should fall
    /// back to page one like an unknown kind does, not error-redirect.
    page: Option<String>,
}

/// A filter-row chip: a link that sets, swaps, or clears one query param.
struct Chip {
    label: String,
    href: String,
    active: bool,
}

/// One timeline item: a curated logbook entry, a Slay the Spire victory
/// (wins only — deaths stay on `/spire`), a manually published lift, or a
/// A running activity, imported from Garmin or entered manually.
enum Item<'a> {
    Log {
        serial: String,
        entry: &'static Entry,
    },
    Win(&'a Run),
    Lift(&'a PublishedWorkout),
    FitnessRun(&'a RunningActivity),
}

impl<'a> Item<'a> {
    fn date(&self) -> &str {
        match self {
            Item::Log { entry, .. } => entry.date(),
            Item::Win(run) => &run.date,
            Item::Lift(published) => &published.date,
            Item::FitnessRun(activity) => running::activity_date(activity),
        }
    }

    /// Sort rank on equal dates: the curated entry leads the day's dynamic items.
    fn rank(&self) -> u8 {
        match self {
            Item::Log { .. } => 0,
            Item::Win(_) | Item::Lift(_) | Item::FitnessRun(_) => 1,
        }
    }

    /// Tie-break among same-date dynamic items; logbook dates are day-granular.
    fn start_time(&self) -> i64 {
        match self {
            Item::Log { .. } => 0,
            Item::Win(run) => run.start_time,
            Item::Lift(published) => published.start_time,
            Item::FitnessRun(activity) => running::start_time_seconds(activity),
        }
    }

    /// The foldable kind, if any: streaks of one dynamic kind collapse on the
    /// default view; curated entries never do.
    fn dyn_kind(&self) -> Option<DynKind> {
        match self {
            Item::Log { .. } => None,
            Item::Win(_) => Some(DynKind::Win),
            Item::Lift(_) => Some(DynKind::Lift),
            Item::FitnessRun(_) => Some(DynKind::FitnessRun),
        }
    }

    /// Everything a search term can match, lowercased: the item's visible
    /// copy plus its tag vocabulary (wins and lifts mirror the tags the chip
    /// filters give them), prefixed with the ISO date so year searches work.
    fn haystack(&self) -> String {
        let copy = match self {
            Item::Log { entry, .. } => match entry {
                Entry::Essay {
                    title,
                    teaser,
                    tags,
                    ..
                } => format!("{title} {teaser} {}", tags.join(" ")),
                Entry::Note {
                    body, source, tags, ..
                } => format!("note {body} {source} {}", tags.join(" ")),
                Entry::Update {
                    stamp,
                    label,
                    body,
                    link_label: update_link_label,
                    tags,
                    ..
                } => {
                    let cover_copy = entry
                        .drum_cover()
                        .map(|cover| format!(" {} {}", cover.title, cover.artist))
                        .unwrap_or_default();
                    format!(
                        "[{stamp}] {label} {body} {update_link_label}{cover_copy} {}",
                        tags.join(" ")
                    )
                }
            },
            Item::Win(run) => format!(
                "[spire] win games {} {} a{}",
                run.game_label(),
                run.character,
                run.ascension
            ),
            Item::Lift(published) => {
                let mut copy = format!("[fitness] lift {}", published.workout.title);
                for set in &published.workout.sets {
                    copy.push(' ');
                    copy.push_str(&set.exercise_name);
                }
                copy
            }
            Item::FitnessRun(activity) => format!(
                "[fitness] run {} {} {} {} {}",
                activity.title,
                activity.activity_type,
                running::distance_label(activity),
                running::duration_label(activity),
                running::pace_label(activity)
            ),
        };
        format!("{} {copy}", self.date()).to_lowercase()
    }

    /// True when every search term appears somewhere in the haystack.
    fn matches(&self, terms: &[String]) -> bool {
        if terms.is_empty() {
            return true;
        }
        let haystack = self.haystack();
        terms.iter().all(|term| haystack.contains(term.as_str()))
    }
}

/// The dynamic item kinds that can pile up between curated entries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DynKind {
    Win,
    Lift,
    FitnessRun,
}

/// A collapsed streak tail: `[stamp] label body link →`, dated by its newest
/// hidden item so the year badges stay honest. Precomputed strings, like
/// every other row.
struct Fold {
    date: String,
    stamp: &'static str,
    label: String,
    body: String,
    href: String,
    link_label: &'static str,
}

/// What a timeline row holds: one item, or the folded tail of a streak.
enum RowItem<'a> {
    One(Item<'a>),
    Fold(Fold),
}

impl RowItem<'_> {
    fn date(&self) -> &str {
        match self {
            RowItem::One(item) => item.date(),
            RowItem::Fold(fold) => &fold.date,
        }
    }
}

/// One visible timeline row, precomputed so the markup stays declarative.
struct Row<'a> {
    /// Set when the year changes between consecutive visible entries.
    year_mark: Option<String>,
    item: RowItem<'a>,
}

impl<'a> Row<'a> {
    fn log(&self) -> Option<(&str, &'static Entry)> {
        match &self.item {
            RowItem::One(Item::Log { serial, entry }) => Some((serial.as_str(), entry)),
            _ => None,
        }
    }

    fn win(&self) -> Option<&'a Run> {
        match self.item {
            RowItem::One(Item::Win(run)) => Some(run),
            _ => None,
        }
    }

    fn lift(&self) -> Option<&'a PublishedWorkout> {
        match self.item {
            RowItem::One(Item::Lift(published)) => Some(published),
            _ => None,
        }
    }

    fn fitness_run(&self) -> Option<&'a RunningActivity> {
        match self.item {
            RowItem::One(Item::FitnessRun(activity)) => Some(activity),
            _ => None,
        }
    }

    fn fold(&self) -> Option<&Fold> {
        match &self.item {
            RowItem::Fold(fold) => Some(fold),
            _ => None,
        }
    }
}

/// The timeline's URL for a filter state, dropping absent params and page
/// one. The timeline is the home page, so everything hangs off `/`.
fn log_url(kind: Option<&str>, tag: Option<&str>, query: Option<&str>, page: usize) -> String {
    let mut params = Vec::new();
    if let Some(kind) = kind {
        params.push(format!("kind={kind}"));
    }
    if let Some(tag) = tag {
        params.push(format!("tag={}", urlencode(tag)));
    }
    if let Some(query) = query {
        params.push(format!("q={}", urlencode(query)));
    }
    if page > 1 {
        params.push(format!("page={page}"));
    }
    if params.is_empty() {
        "/".to_string()
    } else {
        format!("/?{}", params.join("&"))
    }
}

/// Timeline items per page.
const PAGE_SIZE: usize = 20;

/// The requested 1-based page; junk (`?page=banana`, `?page=0`) reads as
/// page one, matching the kind filter's silent fallback.
fn parse_page(raw: Option<&str>) -> usize {
    raw.and_then(|page| page.parse::<usize>().ok())
        .map_or(1, |page| page.max(1))
}

/// Clamp a requested page against the filtered item count: the page actually
/// shown, the page count, and the visible index range.
fn page_slice(total: usize, requested: usize) -> (usize, usize, std::ops::Range<usize>) {
    let pages = total.div_ceil(PAGE_SIZE).max(1);
    let page = requested.clamp(1, pages);
    let start = (page - 1) * PAGE_SIZE;
    (page, pages, start..(start + PAGE_SIZE).min(total))
}

/// Lowercased whitespace-split search terms; an item must contain every one.
fn search_terms(query: Option<&str>) -> Vec<String> {
    query
        .map(|q| q.split_whitespace().map(str::to_lowercase).collect())
        .unwrap_or_default()
}

/// A streak of this many same-kind dynamic items collapses…
const FOLD_THRESHOLD: usize = 4;
/// …down to its newest few plus one fold row (the tail is always ≥ 2, so a
/// fold never costs more rows than it saves).
const FOLD_KEEP: usize = 2;

/// Collapse streaks of one dynamic kind into lead items plus a [`Fold`].
/// Only the default view folds (`folding: false` passes everything through)
/// — a filter or search means the reader asked for the whole pile, and
/// `?tag=spire` would otherwise collapse to almost nothing. `all_runs` is
/// the full run log, for aiming the fold link at the right `/spire` page.
fn fold_items<'a>(items: Vec<Item<'a>>, folding: bool, all_runs: &[Run]) -> Vec<RowItem<'a>> {
    let mut display: Vec<RowItem<'a>> = Vec::new();
    let mut queue = items.into_iter().peekable();
    while let Some(item) = queue.next() {
        let kind = item.dyn_kind();
        if !folding || kind.is_none() {
            display.push(RowItem::One(item));
            continue;
        }
        let mut streak = vec![item];
        while let Some(next) = queue.next_if(|next| next.dyn_kind() == kind) {
            streak.push(next);
        }
        if streak.len() < FOLD_THRESHOLD {
            display.extend(streak.into_iter().map(RowItem::One));
            continue;
        }
        let mut streak = streak.into_iter();
        for lead in streak.by_ref().take(FOLD_KEEP) {
            display.push(RowItem::One(lead));
        }
        let folded: Vec<Item> = streak.collect();
        display.push(RowItem::Fold(make_fold(&folded, all_runs)));
    }
    display
}

/// The fold row for a streak's hidden tail (newest first, length ≥ 2).
fn make_fold(folded: &[Item], all_runs: &[Run]) -> Fold {
    let count = folded.len();
    let oldest = folded[folded.len() - 1].date().to_string();
    match &folded[0] {
        Item::Win(run) => Fold {
            date: run.date.clone(),
            stamp: "[spire]",
            label: format!("{count} more wins ·"),
            body: format!("back to {oldest}"),
            href: run_page_url(all_runs, run),
            link_label: "run log →",
        },
        Item::Lift(published) => Fold {
            date: published.date.clone(),
            stamp: "[fitness]",
            label: format!("{count} more lifts ·"),
            // Date-range filters, unlike page numbers, keep pointing at
            // these workouts as newer ones arrive.
            href: format!("/fitness/log?from={oldest}&to={}", published.date),
            body: format!("back to {oldest}"),
            link_label: "workouts →",
        },
        Item::FitnessRun(activity) => Fold {
            date: running::activity_date(activity).to_string(),
            stamp: "[fitness]",
            label: format!("{count} more runs ·"),
            body: format!("back to {oldest}"),
            href: format!(
                "/fitness/log?from={oldest}&to={}",
                running::activity_date(activity)
            ),
            link_label: "activity log →",
        },
        Item::Log { .. } => unreachable!("curated entries never fold"),
    }
}

/// `/log` carried the timeline for a stretch of 2026; its filter and pager
/// URLs travel, so forward them home with the query intact. (The query
/// string is never percent-decoded, so it is safe inside a Location header.)
#[route(GET "/log")]
async fn legacy_log(cx: &Cx) -> Result {
    let target = match uri(cx).query() {
        Some(query) => format!("/?{query}"),
        None => "/".to_string(),
    };
    Err(redirect_permanent(&target).into())
}

/// The timeline itself: hero, filter row, entries, pager. Rendered by
/// `home.rs` as the log pane — first of the phone deck's five, the whole
/// page on desktop. It reads its filter state from the request query, so the
/// chips work wherever it renders.
#[component]
pub(crate) async fn timeline(cx: &Cx) -> Result {
    let can_log = viewer(cx).is_some_and(|current| is_admin(&current.email));
    let q = query_params::<LogQuery>(cx)?;
    // An unknown kind silently falls back to the full log; a tag filters
    // whatever it names (arbitrary tags just match fewer entries).
    let kind = match q.kind.as_deref() {
        Some("essay") => Some(Kind::Essay),
        Some("note") => Some(Kind::Note),
        Some("update") => Some(Kind::Update),
        _ => None,
    };
    let kind_param = kind.map(|k| match k {
        Kind::Essay => "essay",
        Kind::Note => "note",
        Kind::Update => "update",
    });
    let tag = q.tag.as_deref().filter(|t| !t.is_empty());
    let query = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let terms = search_terms(query);

    // Chips and pagination links carry the search along; anything that
    // changes the filter state resets to page one.
    let kind_chips: Vec<Chip> = [
        ("all", None),
        ("essays", Some("essay")),
        ("notes", Some("note")),
        ("updates", Some("update")),
    ]
    .into_iter()
    .map(|(label, value)| Chip {
        label: label.to_string(),
        href: log_url(value, tag, query, 1),
        active: kind_param == value,
    })
    .collect();

    // The six fixed tags; an active tag from outside the list joins the row
    // so it can be toggled off.
    let mut tag_chips: Vec<Chip> = FILTER_TAGS
        .iter()
        .map(|t| {
            let active = tag == Some(t);
            Chip {
                label: if active {
                    format!("#{t} ×")
                } else {
                    format!("#{t}")
                },
                href: if active {
                    log_url(kind_param, None, query, 1)
                } else {
                    log_url(kind_param, Some(t), query, 1)
                },
                active,
            }
        })
        .collect();
    if let Some(t) = tag
        && !FILTER_TAGS.contains(&t)
    {
        tag_chips.push(Chip {
            label: format!("#{t} ×"),
            href: log_url(kind_param, None, query, 1),
            active: true,
        });
    }

    // Curated entries, synced Spire wins, published lifts, and fitness runs
    // interleave into one timeline. Wins behave like updates tagged "spire"
    // or "games"; both fitness kinds behave like updates tagged "fitness"
    // (unpublished CSV history stays archive-only, matching `/feed.xml`).
    let spire = spire_runs::load(app_context::<Data>(cx)).await;
    let run_log = running::load(app_context::<Data>(cx)).await;
    let workouts = match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => snapshot.published_workouts(),
        Err(error) => {
            eprintln!("fitness timeline snapshot failed: {error}");
            Vec::new()
        }
    };
    let mut items: Vec<Item> = Vec::new();
    for (index, entry) in LOG.iter().enumerate() {
        if kind.is_some_and(|k| entry.kind() != k)
            || tag.is_some_and(|t| !entry.tags().contains(&t))
        {
            continue;
        }
        items.push(Item::Log {
            serial: serial(index),
            entry,
        });
    }
    if !kind.is_some_and(|k| k != Kind::Update)
        && !tag.is_some_and(|t| t != "spire" && t != "games")
    {
        items.extend(spire.runs.iter().filter(|r| r.win).map(Item::Win));
    }
    if !kind.is_some_and(|k| k != Kind::Update) && !tag.is_some_and(|t| t != "fitness") {
        items.extend(workouts.iter().map(Item::Lift));
        items.extend(run_log.activities.iter().map(Item::FitnessRun));
    }
    items.retain(|item| item.matches(&terms));
    items.sort_by(|a, b| {
        b.date()
            .cmp(a.date())
            .then_with(|| a.rank().cmp(&b.rank()))
            .then_with(|| b.start_time().cmp(&a.start_time()))
    });

    let folding = kind.is_none() && tag.is_none() && query.is_none();
    let display = fold_items(items, folding, &spire.runs);

    let total = display.len();
    let (page, total_pages, visible) = page_slice(total, parse_page(q.page.as_deref()));

    let mut rows: Vec<Row> = Vec::new();
    // Pages after the first open with a year badge — the running year context
    // from the rows above them is gone. (`Some("")` differs from every year.)
    let mut last_year: Option<String> = (page > 1).then(String::new);
    for item in display.into_iter().skip(visible.start).take(visible.len()) {
        let year = item.date()[0..4].to_string();
        let year_mark = match &last_year {
            Some(prev) if *prev != year => Some(year.clone()),
            _ => None,
        };
        last_year = Some(year);
        rows.push(Row { year_mark, item });
    }

    let words = hero_words();
    let hero_fit = format!("--log-hero-fit: {:.1}", hero_line_ems(&words));

    view! {
        // Hero: "I like {interest}", cycling the interest registry. Pure CSS
        // — see .log-hero-* in logbook.css, including the font-size, which
        // divides --log-hero-fit (set here from the registry) out of the
        // text column's container width so the longest rotation always fits.
        // Each word links to its page; only the currently visible one is
        // hoverable (visibility + pause-on-hover).
        <header class="rail-row mt-16">
            <p class="rail-stamp rail-stamp-label">"log"</p>
            <div class="flex min-w-0 items-start justify-between gap-4">
                <div class="@container min-w-0 flex-1" style=(hero_fit.as_str())>
                    <h1 class="log-hero font-display leading-none font-bold tracking-tight">
                        "I like "
                        <span class="log-hero-words">
                            for interest in words.iter() {
                                <a
                                    class="log-hero-word text-oxide"
                                    href=(format!("/{}", interest.slug))
                                >(format!("{}.", interest.title.to_lowercase()))</a>
                            }
                        </span>
                    </h1>
                    <p class="mt-4 max-w-prose text-[17px] leading-relaxed text-ink2">
                        "Software developer in New York"
                        // The phone deck's one hint; desktop has the tmux windows.
                        <span class="text-muted sm:hidden">" · swipe for more →"</span>
                    </p>
                </div>
                if can_log {
                    crate::app::interests::lifting::home::log_launcher()
                }
            </div>
        </header>

        // Filter row: kind chips, tag chips, search, and the feed. Server-side
        // — every chip is a link that rewrites the query string, and the
        // search box is a plain GET form that does the same (hidden inputs
        // carry the active filters along). Phones drop the row for the
        // minimal pane look; the timeline itself is the mobile page.
        <div class="mt-11 hidden flex-wrap items-baseline gap-4 border-t border-hairline pt-4 font-meta text-[13px] sm:flex">
            for chip in kind_chips.iter() {
                <a
                    class=(if chip.active { "log-chip log-chip-active" } else { "log-chip" })
                    href=(chip.href.as_str())
                >(chip.label.as_str())</a>
            }
            <span class="text-hairline">"|"</span>
            for chip in tag_chips.iter() {
                <a
                    class=(if chip.active { "log-tag log-tag-active" } else { "log-tag" })
                    href=(chip.href.as_str())
                >(chip.label.as_str())</a>
            }
            <form class="ml-auto" action="/" method="get">
                if let Some(kind_value) = kind_param {
                    <input type="hidden" name="kind" value=(kind_value)>
                }
                if let Some(tag_value) = tag {
                    <input type="hidden" name="tag" value=(tag_value)>
                }
                <input
                    type="search"
                    name="q"
                    value=(query.unwrap_or(""))
                    placeholder="search"
                    aria-label="Search the log"
                    class="w-28 border-b border-hairline bg-transparent pb-0.5 text-ink outline-none placeholder:text-muted focus:border-oxide"
                >
            </form>
            <a class="text-muted hover:text-oxide" href="/feed.xml">
                link_label(label: "rss ↗")
            </a>
        </div>

        // Search results line: the match count doubles as the empty-state
        // explanation, and clearing drops only `q` (filters stay put).
        if let Some(query_text) = query {
            <p class="mt-4 font-meta text-[13px] text-ink2">
                (format!(
                    "{total} {} for “{query_text}”",
                    if total == 1 { "entry" } else { "entries" }
                ))
                " · "
                <a class="log-tag" href=(log_url(kind_param, tag, None, 1))>"clear ×"</a>
            </p>
        }

        // The timeline: one vertical hairline, a marker per entry, a year
        // badge wherever the visible entries change year. Streaks of wins,
        // lifts, or runs arrive pre-collapsed into fold rows (see `fold_items`).
        <section class="log-timeline">
            if rows.is_empty() {
                <p class="font-meta text-[13px] text-muted">
                    "Nothing in the log matches."
                </p>
            }
            for row in rows.iter() {
                if let Some(year) = &row.year_mark {
                    <div class="log-row">
                        <span class="log-year">(year.as_str())</span>
                    </div>
                }
                if let Some((serial, entry)) = row.log() {
                    if let Entry::Essay {
                        title,
                        teaser,
                        slug,
                        photo,
                        read_label,
                        tags,
                        ..
                    } = entry {
                        <article class="log-row" data-rail-item="">
                            <span class="log-mark log-mark-essay"></span>
                            <div class="log-rail">
                                <p class="log-date">(entry.date())</p>
                                <p class="log-serial">(serial)</p>
                            </div>
                            <div class="log-card">
                                <h2 class="log-card-title font-display font-bold">
                                    <a
                                        class="oxlink"
                                        href=(format!("/thoughts/{slug}"))
                                        data-rail-enter=""
                                    >(title)</a>
                                </h2>
                                <p class="mt-2.5 max-w-prose leading-relaxed text-ink2">(teaser)</p>
                                if let Some(photo) = photo {
                                    <a
                                        class="mt-5 block overflow-hidden rounded-[2px] border border-hairline"
                                        href=(format!("/thoughts/{slug}"))
                                    >
                                        <img
                                            src=(photo.src)
                                            alt=(photo.alt)
                                            width=(photo.width)
                                            height=(photo.height)
                                            loading="lazy"
                                            decoding="async"
                                            class="aspect-[4/3] w-full object-cover object-center"
                                        >
                                    </a>
                                }
                                <div class="mt-4 flex flex-wrap items-baseline gap-3 font-meta text-xs">
                                    for t in tags.iter() {
                                        <a class="log-tag" href=(log_url(kind_param, Some(t), query, 1))>(format!("#{t}"))</a>
                                    }
                                    <a
                                        class="ml-auto text-ink2 no-underline hover:text-oxide"
                                        href=(format!("/thoughts/{slug}"))
                                    >link_label(label: read_label)</a>
                                </div>
                            </div>
                        </article>
                    }
                    if let Entry::Note { body, source, slug, .. } = entry {
                        <article class="log-row" data-rail-item="">
                            <span class="log-mark log-mark-note"></span>
                            <div class="log-rail">
                                <p class="log-date">(entry.date())</p>
                                <p class="log-serial">(serial)</p>
                            </div>
                            <div class="log-note min-w-0">
                                <p class="log-note-body font-display">(body)</p>
                                <p class="mt-2.5 font-meta text-xs text-muted">
                                    "note · "
                                    (source)
                                    " · "
                                    <a
                                        class="log-permalink"
                                        href=(format!("/thoughts/{slug}"))
                                        data-rail-enter=""
                                    >"permalink"</a>
                                </p>
                            </div>
                        </article>
                    }
                    if let Entry::Update { stamp, label, body, href, link_label: update_link_label, .. } = entry {
                        if let Some(cover) = entry.drum_cover() {
                            <article class="log-row" data-rail-item="">
                                <span class="log-mark log-mark-update"></span>
                                <p class="log-date">(entry.date())</p>
                                <a class="log-cover-link" href=(href) data-rail-enter="">
                                    <span class="log-cover-media">
                                        <img
                                            src=(format!(
                                                "https://img.youtube.com/vi/{}/mqdefault.jpg",
                                                cover.youtube_id
                                            ))
                                            alt=""
                                            loading="lazy"
                                            decoding="async"
                                        >
                                        <span class="log-cover-play" aria-hidden="true">
                                            "▶"
                                        </span>
                                    </span>
                                    <span class="log-cover-copy">
                                        <span class="log-cover-kicker">
                                            <span class="log-update-stamp">(format!("[{stamp}]"))</span>
                                            " "
                                            <span class="text-patina">(format!("{label} ·"))</span>
                                            " "
                                            (body)
                                        </span>
                                        <strong class="log-cover-title">(cover.title)</strong>
                                        <span class="log-cover-artist">(cover.artist)</span>
                                        <span class="log-cover-watch">
                                            link_label(label: "watch on YouTube ↗")
                                        </span>
                                    </span>
                                </a>
                            </article>
                        } else {
                            <article class="log-row items-baseline" data-rail-item="">
                                <span class="log-mark log-mark-update"></span>
                                <p class="log-date">(entry.date())</p>
                                <p class="log-update min-w-0">
                                    <span class="log-update-stamp">(format!("[{stamp}]"))</span>
                                    " "
                                    <span class="text-patina">(format!("{label} ·"))</span>
                                    " "
                                    (body)
                                    " "
                                    <a class="log-update-link" href=(href) data-rail-enter="">
                                        link_label(label: update_link_label)
                                    </a>
                                </p>
                            </article>
                        }
                    }
                }
                if let Some(run) = row.win() {
                    <article class="log-row items-baseline" data-rail-item="">
                        <span class="log-mark log-mark-update"></span>
                        <p class="log-date">(run.date.as_str())</p>
                        <p class="log-update min-w-0">
                            <span class="log-update-stamp">"[spire]"</span>
                            " "
                            <span class="text-patina">"win ·"</span>
                            " "
                            (format!(
                                "{} {}, A{}",
                                run.game_label(),
                                run.character,
                                run.ascension
                            ))
                            " "
                            <a class="log-update-link" href="/spire" data-rail-enter="">
                                link_label(label: "run log →")
                            </a>
                        </p>
                    </article>
                }
                if let Some(published) = row.lift() {
                    <article class="log-row items-baseline" data-rail-item="">
                        <span class="log-mark log-mark-update"></span>
                        <p class="log-date">(published.date.as_str())</p>
                        <p class="log-update min-w-0">
                            <span class="log-update-stamp">"[fitness]"</span>
                            " "
                            <span class="text-patina">"lift ·"</span>
                            " "
                            (format!(
                                "{}, {} volume points",
                                published.workout.title,
                                workout_volume_points(&published.workout)
                            ))
                            " "
                            <a
                                class="log-update-link"
                                href=(format!("/fitness/lift/{}", published.workout.path))
                                data-rail-enter=""
                            >
                                link_label(label: "workout →")
                            </a>
                        </p>
                    </article>
                }
                if let Some(activity) = row.fitness_run() {
                    <article class="log-row items-baseline" data-rail-item="">
                        <span class="log-mark log-mark-update"></span>
                        <p class="log-date">(running::activity_date(activity))</p>
                        <p class="log-update min-w-0">
                            <span class="log-update-stamp">"[fitness]"</span>
                            " "
                            <span class="text-patina">"run ·"</span>
                            " "
                            (format!(
                                "{}, {} in {} at {}",
                                activity.title,
                                running::distance_label(activity),
                                running::duration_label(activity),
                                running::pace_label(activity)
                            ))
                            " "
                            <a
                                class="log-update-link"
                                href=(running::public_url(activity))
                                data-rail-enter=""
                            >
                                link_label(label: "run →")
                            </a>
                        </p>
                    </article>
                }
                if let Some(fold) = row.fold() {
                    <article class="log-row items-baseline" data-rail-item="">
                        <span class="log-mark log-mark-update"></span>
                        <p class="log-date">(fold.date.as_str())</p>
                        <p class="log-update min-w-0">
                            <span class="log-update-stamp">(fold.stamp)</span>
                            " "
                            <span class="text-patina">(fold.label.as_str())</span>
                            " "
                            (fold.body.as_str())
                            " "
                            <a
                                class="log-update-link"
                                href=(fold.href.as_str())
                                data-rail-enter=""
                            >
                                link_label(label: fold.link_label)
                            </a>
                        </p>
                    </article>
                }
            }
        </section>

        // Pager: newest-first, so "older" walks toward higher page numbers.
        // Dead ends render as hairline text to keep the row's shape stable.
        if total_pages > 1 {
            <nav
                class="mt-12 flex items-baseline justify-between border-t border-hairline pt-4 font-meta text-[13px]"
                aria-label="Timeline pages"
            >
                if page > 1 {
                    <a class="log-chip" href=(log_url(kind_param, tag, query, page - 1))>
                        "← newer"
                    </a>
                } else {
                    <span class="text-hairline">"← newer"</span>
                }
                <span class="text-muted">(format!("page {page} of {total_pages}"))</span>
                if page < total_pages {
                    <a class="log-chip" href=(log_url(kind_param, tag, query, page + 1))>
                        "older →"
                    </a>
                } else {
                    <span class="text-hairline">"older →"</span>
                }
            </nav>
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::interests::lifting::archive::api::{Set, Workout};

    static ESSAY: Entry = Entry::Essay {
        date: "2026-07-12",
        title: "How bad are planes?",
        teaser: "A carbon receipt for one seat.",
        slug: "how-bad-are-planes",
        photo: None,
        read_label: "read →",
        tags: &["climate"],
    };

    static UPDATE: Entry = Entry::Update {
        date: "2026-06-28",
        stamp: "pr",
        label: "keyboards",
        body: "TypeRacer now has me at 117wpm.",
        href: "/keyboards",
        link_label: "keyboards →",
        tags: &["keyboards"],
    };

    fn terms(query: &str) -> Vec<String> {
        search_terms(Some(query))
    }

    fn win(id: &str, date: &str) -> Run {
        Run {
            id: id.to_string(),
            game: "sts2".to_string(),
            date: date.to_string(),
            start_time: id.parse().unwrap_or(0),
            character: "Necrobinder".to_string(),
            win: true,
            abandoned: false,
            ascension: 12,
            floors: 34,
            killed_by: None,
            kill_kind: None,
            run_time: 7534,
            game_mode: "standard".to_string(),
        }
    }

    fn lift(date: &str, title: &str, exercise: &str) -> PublishedWorkout {
        PublishedWorkout {
            workout: Workout {
                id: format!("fitness:{date}T14:38:00"),
                path: format!("{date}T10-38-00-04-00"),
                title: title.to_string(),
                raw_title: title.to_string(),
                started_at_local: format!("{date} 10:38:00"),
                ended_at_local: format!("{date} 10:50:00"),
                eastern_offset_minutes: -240,
                end_eastern_offset_minutes: -240,
                duration_seconds: 720,
                duration_suspicious: false,
                notes: None,
                description: None,
                sets: vec![Set {
                    id: format!("fitness:{date}T14:38:00:0001"),
                    ordinal: 1,
                    exercise_name: exercise.to_string(),
                    raw_exercise_name: exercise.to_string(),
                    exercise_note: None,
                    superset_id: None,
                    weight_milli: Some(95_000),
                    weight_unit: "lbs".to_string(),
                    reps: Some(8),
                    effort_hundredths: Some(1_000),
                    distance_milli: None,
                    set_time_seconds: None,
                    set_type: "NORMAL_SET".to_string(),
                    records: Vec::new(),
                }],
            },
            date: date.to_string(),
            start_time: 1_784_903_880,
        }
    }

    fn fitness_run(id: &str, date: &str, title: &str) -> RunningActivity {
        RunningActivity {
            id: id.to_string(),
            source: "garmin-connect".to_string(),
            source_activity_id: id.to_string(),
            source_url: None,
            title: title.to_string(),
            activity_type: "running".to_string(),
            started_at_utc: format!("{date} 14:00:00"),
            started_at_local: format!("{date} 10:00:00"),
            eastern_offset_minutes: -240,
            duration_milliseconds: 2_600_000,
            moving_duration_milliseconds: Some(2_550_000),
            distance_millimeters: 6_437_376,
            ascent_millimeters: Some(91_440),
            imported_at: 1,
        }
    }

    /// One char per display row: curated Log, Spire win, lift, fitness run,
    /// or Fold.
    fn kinds(display: &[RowItem]) -> String {
        display
            .iter()
            .map(|row| match row {
                RowItem::One(Item::Log { .. }) => 'L',
                RowItem::One(Item::Win(_)) => 'w',
                RowItem::One(Item::Lift(_)) => 'l',
                RowItem::One(Item::FitnessRun(_)) => 'r',
                RowItem::Fold(_) => 'F',
            })
            .collect()
    }

    fn first_fold<'x>(display: &'x [RowItem]) -> &'x Fold {
        display
            .iter()
            .find_map(|row| match row {
                RowItem::Fold(fold) => Some(fold),
                _ => None,
            })
            .expect("a fold row")
    }

    /// The hero's fit variable is set here and consumed in logbook.css;
    /// renaming either side alone would quietly restore the sideways
    /// scrolling on phones.
    #[test]
    fn the_hero_fit_variable_reaches_its_stylesheet() {
        const LOGBOOK_CSS: &str = include_str!("../../styles/logbook.css");
        assert!(LOGBOOK_CSS.contains("var(--log-hero-fit"));
        let words = hero_words();
        assert!(!words.is_empty());
        // "I like slay the spire." is today's widest line; the derivation
        // must track the registry, not a constant.
        assert_eq!(format!("{:.1}", hero_line_ems(&words)), "13.2");
    }

    #[test]
    fn the_hero_rotation_has_one_css_slot_per_interest() {
        const LOGBOOK_CSS: &str = include_str!("../../styles/logbook.css");
        let word_count = hero_words().len();
        let cycle_seconds = word_count as f64 * 2.6;
        assert!(LOGBOOK_CSS.contains(&format!(
            "animation: log-word {cycle_seconds:.1}s ease infinite"
        )));
        assert!(LOGBOOK_CSS.contains(&format!(".log-hero-word:nth-child({word_count})")));
    }

    #[test]
    fn log_urls_drop_absent_params_and_page_one() {
        assert_eq!(log_url(None, None, None, 1), "/");
        assert_eq!(log_url(Some("note"), None, None, 1), "/?kind=note");
        assert_eq!(log_url(None, None, None, 2), "/?page=2");
        assert_eq!(
            log_url(Some("essay"), Some("rust"), Some("how bad"), 3),
            "/?kind=essay&tag=rust&q=how%20bad&page=3"
        );
    }

    #[test]
    fn junk_page_params_read_as_page_one() {
        assert_eq!(parse_page(None), 1);
        assert_eq!(parse_page(Some("banana")), 1);
        assert_eq!(parse_page(Some("0")), 1);
        assert_eq!(parse_page(Some("-2")), 1);
        assert_eq!(parse_page(Some("7")), 7);
    }

    #[test]
    fn page_slices_clamp_to_the_item_count() {
        assert_eq!(page_slice(0, 1), (1, 1, 0..0));
        assert_eq!(page_slice(0, 9), (1, 1, 0..0));
        assert_eq!(page_slice(PAGE_SIZE, 1), (1, 1, 0..PAGE_SIZE));
        let total = 2 * PAGE_SIZE + 5;
        assert_eq!(page_slice(total, 2), (2, 3, PAGE_SIZE..2 * PAGE_SIZE));
        assert_eq!(page_slice(total, 3), (3, 3, 2 * PAGE_SIZE..total));
        assert_eq!(page_slice(total, 99), (3, 3, 2 * PAGE_SIZE..total));
    }

    #[test]
    fn search_terms_lowercase_and_split_on_whitespace() {
        assert!(search_terms(None).is_empty());
        assert_eq!(search_terms(Some("How  Bad")), ["how", "bad"]);
    }

    #[test]
    fn essays_match_visible_copy_tags_and_date() {
        let item = Item::Log {
            serial: "№ 0042".to_string(),
            entry: &ESSAY,
        };
        assert!(item.matches(&[]));
        assert!(item.matches(&terms("PLANES")));
        assert!(item.matches(&terms("carbon receipt")));
        assert!(item.matches(&terms("climate")));
        assert!(item.matches(&terms("2026")));
        assert!(!item.matches(&terms("planes drums")));
    }

    #[test]
    fn updates_match_stamp_label_and_body() {
        let item = Item::Log {
            serial: "№ 0041".to_string(),
            entry: &UPDATE,
        };
        assert!(item.matches(&terms("117wpm")));
        assert!(item.matches(&terms("pr keyboards")));
        assert!(!item.matches(&terms("planes")));
    }

    #[test]
    fn drum_covers_match_song_and_artist_searches() {
        let entry = LOG
            .iter()
            .find(|entry| {
                entry
                    .drum_cover()
                    .is_some_and(|cover| cover.youtube_id == "HyPCqzi74nE")
            })
            .expect("new drum cover in log");
        let item = Item::Log {
            serial: "№ 0042".to_string(),
            entry,
        };
        assert!(item.matches(&terms("dancefloor")));
        assert!(item.matches(&terms("arctic monkeys")));
    }

    #[test]
    fn dynamic_items_match_their_stamp_vocabulary() {
        let run = win("1784587453", "2026-07-20");
        let item = Item::Win(&run);
        assert!(item.matches(&terms("spire win")));
        assert!(item.matches(&terms("necrobinder")));
        assert!(item.matches(&terms("a12")));
        assert!(item.matches(&terms("games")));
        assert!(!item.matches(&terms("silent")));

        let published = lift(
            "2026-07-24",
            "Quickest Arms in the West",
            "Incline Bench Press",
        );
        let item = Item::Lift(&published);
        assert!(item.matches(&terms("fitness lift")));
        assert!(item.matches(&terms("quickest arms")));
        assert!(item.matches(&terms("incline bench")));
        assert!(!item.matches(&terms("squat")));

        let activity = fitness_run("24065766206", "2026-07-25", "Morning Run");
        let item = Item::FitnessRun(&activity);
        assert!(item.matches(&terms("fitness run")));
        assert!(item.matches(&terms("morning running")));
        assert!(item.matches(&terms("4.00 mi")));
        assert!(!item.matches(&terms("lift")));
    }

    #[test]
    fn streaks_of_four_or_more_fold_after_two_leads() {
        let runs: Vec<Run> = (0..5)
            .map(|i| win(&format!("{}", 100 - i), &format!("2026-07-{:02}", 20 - i)))
            .collect();
        let items: Vec<Item> = runs.iter().map(Item::Win).collect();
        let display = fold_items(items, true, &runs);
        assert_eq!(kinds(&display), "wwF");
        let fold = first_fold(&display);
        assert_eq!(fold.stamp, "[spire]");
        assert_eq!(fold.label, "3 more wins ·");
        assert_eq!(fold.date, "2026-07-18");
        assert_eq!(fold.body, "back to 2026-07-16");
        assert_eq!(fold.href, "/spire#run-log");
        assert_eq!(fold.link_label, "run log →");
    }

    #[test]
    fn short_streaks_and_filtered_views_stay_unfolded() {
        let runs: Vec<Run> = (0..5)
            .map(|i| win(&format!("{}", 100 - i), &format!("2026-07-{:02}", 20 - i)))
            .collect();
        let three: Vec<Item> = runs[..3].iter().map(Item::Win).collect();
        assert_eq!(kinds(&fold_items(three, true, &runs)), "www");
        let five: Vec<Item> = runs.iter().map(Item::Win).collect();
        assert_eq!(kinds(&fold_items(five, false, &runs)), "wwwww");
    }

    #[test]
    fn curated_entries_break_streaks() {
        let runs: Vec<Run> = (0..6)
            .map(|i| win(&format!("{}", 100 - i), &format!("2026-07-{:02}", 26 - i)))
            .collect();
        let mut items: Vec<Item> = runs[..2].iter().map(Item::Win).collect();
        items.push(Item::Log {
            serial: "№ 0042".to_string(),
            entry: &ESSAY,
        });
        items.extend(runs[2..].iter().map(Item::Win));
        assert_eq!(kinds(&fold_items(items, true, &runs)), "wwLwwF");
    }

    #[test]
    fn lift_folds_link_the_archive_date_range() {
        let runs: Vec<Run> = (0..2)
            .map(|i| win(&format!("{}", 50 - i), &format!("2026-07-{:02}", 28 - i)))
            .collect();
        let lifts: Vec<PublishedWorkout> = (0..4)
            .map(|i| lift(&format!("2026-07-{:02}", 26 - i), "Push Day", "Bench Press"))
            .collect();
        let mut items: Vec<Item> = runs.iter().map(Item::Win).collect();
        items.extend(lifts.iter().map(Item::Lift));
        let display = fold_items(items, true, &runs);
        assert_eq!(kinds(&display), "wwllF");
        let fold = first_fold(&display);
        assert_eq!(fold.stamp, "[fitness]");
        assert_eq!(fold.label, "2 more lifts ·");
        assert_eq!(fold.date, "2026-07-24");
        assert_eq!(fold.body, "back to 2026-07-23");
        assert_eq!(fold.href, "/fitness/log?from=2026-07-23&to=2026-07-24");
        assert_eq!(fold.link_label, "workouts →");
    }

    #[test]
    fn fitness_runs_form_a_distinct_streak_and_fold_to_the_activity_log() {
        let lifts: Vec<PublishedWorkout> = (0..2)
            .map(|i| lift(&format!("2026-07-{:02}", 28 - i), "Push Day", "Bench Press"))
            .collect();
        let activities: Vec<RunningActivity> = (0..4)
            .map(|i| {
                fitness_run(
                    &format!("run-{i}"),
                    &format!("2026-07-{:02}", 26 - i),
                    "Morning Run",
                )
            })
            .collect();
        let mut items: Vec<Item> = lifts.iter().map(Item::Lift).collect();
        items.extend(activities.iter().map(Item::FitnessRun));

        let display = fold_items(items, true, &[]);
        assert_eq!(kinds(&display), "llrrF");
        let fold = first_fold(&display);
        assert_eq!(fold.stamp, "[fitness]");
        assert_eq!(fold.label, "2 more runs ·");
        assert_eq!(fold.date, "2026-07-24");
        assert_eq!(fold.body, "back to 2026-07-23");
        assert_eq!(fold.href, "/fitness/log?from=2026-07-23&to=2026-07-24");
        assert_eq!(fold.link_label, "activity log →");
    }

    #[test]
    fn fold_links_target_the_spire_page_holding_the_newest_hidden_run() {
        // 60 runs, newest first; /spire shows 50 per page.
        let runs: Vec<Run> = (0..60)
            .map(|i| win(&format!("{}", 1000 - i), "2026-01-01"))
            .collect();
        assert_eq!(run_page_url(&runs, &runs[10]), "/spire#run-log");
        assert_eq!(run_page_url(&runs, &runs[52]), "/spire?page=2#run-log");

        // A streak whose two leads sit at indices 50-51: the newest hidden
        // run (index 52) is what the fold link must land on.
        let items: Vec<Item> = runs[50..56].iter().map(Item::Win).collect();
        let fold_href = first_fold(&fold_items(items, true, &runs)).href.clone();
        assert_eq!(fold_href, "/spire?page=2#run-log");
    }
}
