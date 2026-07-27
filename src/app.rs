mod analytics;
mod feed;
mod interests;
mod llms;
pub(crate) mod login;
mod motorcycles;
mod not_found;
mod response_layer;
mod resume;
mod thoughts;
mod workout_upload;

use benjisponge::data::Data;

use self::feed::workout_description;
use self::interests::lifting::archive::snapshot::PublishedWorkout;
use self::interests::lifting::archive::store::FitnessStore;
use self::interests::spire::runs::{self as spire_runs, Run, fmt_duration};
use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    context::{Cx, app_context},
    cookie::{Key, RouterBuilderCookieExt},
    router::{HeaderValue, Router, RouterBuilderDiscoverExt, header, page, query_params},
    session::{Config, RouterBuilderSessionExt, cookie::CookieTokenStore},
    view::view,
};

use crate::{
    components::{ext_link, inline_popover, link_label, shell},
    content::{
        interests::INTERESTS,
        logbook::{Entry, FILTER_TAGS, Kind, LOG, serial},
    },
    util::urlencode,
};

pub fn router() -> Router {
    let data = Data::from_env();
    Router::builder()
        .assets(AssetBundle::load().unwrap())
        .discover()
        .cookies()
        .sessions(
            Config::builder()
                .token_store(CookieTokenStore::new().name("analytics-visitor"))
                .lifetime(std::time::Duration::from_hours(24 * 400))
                .build(),
        )
        .app_context(data.clone())
        .app_context(FitnessStore::new(data))
        .app_context(analytics::guard::AnalyticsGuard::default())
        .app_context(cookie_key())
        .build()
}

/// The key behind `private_cookies` (the login module's viewer cookie).
/// `COOKIE_KEY` is any secret string of 32+ bytes; without it, a fresh key
/// per boot means viewer sessions silently reset on every restart — fine to
/// ignore until sign-in matters, wrong to ship once it does.
fn cookie_key() -> Key {
    match std::env::var("COOKIE_KEY") {
        Ok(master) if master.len() >= 32 => Key::derive_from(master.as_bytes()),
        Ok(_) => panic!("COOKIE_KEY must be at least 32 bytes"),
        Err(_) => {
            eprintln!(
                "{}",
                serde_json::json!({
                    "message": "COOKIE_KEY unset; viewer sessions will not survive restarts",
                })
            );
            Key::generate()
        }
    }
}

#[query_params(error = redirect("?"))]
struct HomeQuery {
    kind: Option<String>,
    tag: Option<String>,
}

/// A filter-row chip: a link that sets, swaps, or clears one query param.
struct Chip {
    label: String,
    href: String,
    active: bool,
}

/// One timeline item: a curated logbook entry, a Slay the Spire 2 victory
/// (wins only — deaths stay on `/spire`), or a manually published lift.
enum Item<'a> {
    Log {
        serial: String,
        entry: &'static Entry,
    },
    Win(&'a Run),
    Lift(&'a PublishedWorkout),
}

impl<'a> Item<'a> {
    fn date(&self) -> &str {
        match self {
            Item::Log { entry, .. } => entry.date(),
            Item::Win(run) => &run.date,
            Item::Lift(published) => &published.date,
        }
    }

    /// Sort rank on equal dates: the curated entry leads the day's dynamic items.
    fn rank(&self) -> u8 {
        match self {
            Item::Log { .. } => 0,
            Item::Win(_) | Item::Lift(_) => 1,
        }
    }

    /// Tie-break among same-date dynamic items; logbook dates are day-granular.
    fn start_time(&self) -> i64 {
        match self {
            Item::Log { .. } => 0,
            Item::Win(run) => run.start_time,
            Item::Lift(published) => published.start_time,
        }
    }
}

/// One visible timeline row, precomputed so the markup stays declarative.
struct Row<'a> {
    /// Set when the year changes between consecutive visible entries.
    year_mark: Option<String>,
    item: Item<'a>,
}

impl<'a> Row<'a> {
    fn log(&self) -> Option<(&str, &'static Entry)> {
        match &self.item {
            Item::Log { serial, entry } => Some((serial.as_str(), entry)),
            Item::Win(_) | Item::Lift(_) => None,
        }
    }

    fn win(&self) -> Option<&'a Run> {
        match self.item {
            Item::Win(run) => Some(run),
            Item::Log { .. } | Item::Lift(_) => None,
        }
    }

    fn lift(&self) -> Option<&'a PublishedWorkout> {
        match self.item {
            Item::Lift(published) => Some(published),
            Item::Log { .. } | Item::Win(_) => None,
        }
    }
}

/// The homepage URL for a filter state, dropping absent params.
fn home_url(kind: Option<&str>, tag: Option<&str>) -> String {
    let mut params = Vec::new();
    if let Some(kind) = kind {
        params.push(format!("kind={kind}"));
    }
    if let Some(tag) = tag {
        params.push(format!("tag={}", urlencode(tag)));
    }
    if params.is_empty() {
        "/".to_string()
    } else {
        format!("/?{}", params.join("&"))
    }
}

#[page("/")]
async fn home(cx: &Cx) -> Result {
    let q = query_params::<HomeQuery>(cx)?;
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

    let kind_chips: Vec<Chip> = [
        ("all", None),
        ("essays", Some("essay")),
        ("notes", Some("note")),
        ("updates", Some("update")),
    ]
    .into_iter()
    .map(|(label, value)| Chip {
        label: label.to_string(),
        href: home_url(value, tag),
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
                    home_url(kind_param, None)
                } else {
                    home_url(kind_param, Some(t))
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
            href: home_url(kind_param, None),
            active: true,
        });
    }

    // Curated entries, synced Spire wins, and published lifts interleave
    // into one timeline. Wins behave like updates tagged "spire" or "games";
    // lifts behave like updates tagged "fitness" (CSV history stays
    // archive-only, matching `/feed.xml`).
    let spire = spire_runs::load(app_context::<Data>(cx)).await;
    let workouts = match app_context::<FitnessStore>(cx).snapshot().await {
        Ok(snapshot) => snapshot.published_workouts(),
        Err(error) => {
            eprintln!("fitness homepage snapshot failed: {error}");
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
    }
    items.sort_by(|a, b| {
        b.date()
            .cmp(a.date())
            .then_with(|| a.rank().cmp(&b.rank()))
            .then_with(|| b.start_time().cmp(&a.start_time()))
    });

    let mut rows: Vec<Row> = Vec::new();
    let mut last_year: Option<String> = None;
    for item in items {
        let year = item.date()[0..4].to_string();
        let year_mark = match &last_year {
            Some(prev) if *prev != year => Some(year.clone()),
            _ => None,
        };
        last_year = Some(year);
        rows.push(Row { year_mark, item });
    }

    view! {
        // Fresh runs and lifts appear within a minute; CDN honors s-maxage
        // (see docs/railway-deploy.md).
        ((header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=0, s-maxage=60")))
        shell(title: "", active: "log",
        // Hero: "I like {interest}", cycling the interest registry. Pure CSS
        // — see .log-hero-* in logbook.css. Each word links to its page; only
        // the currently visible one is hoverable (visibility + pause-on-hover).
        <section class="mt-16">
            <h1 class="log-hero font-display text-[2.75rem] leading-none font-bold tracking-tight sm:text-[4rem]">
                "I like "
                <span class="log-hero-words">
                    for interest in INTERESTS
                        .iter()
                        .filter(|i| !matches!(i.slug, "simulation" | "puzzles"))
                    {
                        <a
                            class="log-hero-word text-oxide"
                            href=(format!("/{}", interest.slug))
                        >(format!("{}.", interest.title.to_lowercase()))</a>
                    }
                </span>
            </h1>
            <p class="mt-4 max-w-prose text-[17px] leading-relaxed text-ink2">
                "Software developer in New York"
            </p>
            <p class="mt-5 font-meta text-[13px] text-muted">
                "now — building "
                inline_popover(
                    id: "digichem-cite",
                    label: "DigiChem",
                    <span class="inline-popover-preview">
                        "A startup that was trying to do digital manufacturing of specialty \
                         chemicals and is currently sourcing ideas."
                    </span>
                    ext_link(
                        class: "quiet-link",
                        href: "https://digichem.com",
                        label: "digichem.com →"
                    )
                )"
                · I contain " <a class="underline" href="./interests">"multitudes"</a>
                <span class="log-caret text-oxide">"▍"</span>
            </p>
        </section>

        // Filter row: kind chips, tag chips, and the feed. Server-side —
        // every chip is a link that rewrites the query string.
        <div class="mt-11 flex flex-wrap items-baseline gap-4 border-t border-hairline pt-4 font-meta text-[13px]">
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
            <a class="ml-auto text-muted hover:text-oxide" href="/feed.xml">
                link_label(label: "rss ↗")
            </a>
        </div>

        // The timeline: one vertical hairline, a marker per entry, a year
        // badge wherever the visible entries change year.
        <section class="log-timeline">
            for row in rows.iter() {
                if let Some(year) = &row.year_mark {
                    <div class="log-row">
                        <span class="log-year">(year.as_str())</span>
                    </div>
                }
                if let Some((serial, entry)) = row.log() {
                    if let Entry::Essay { title, teaser, slug, tags, .. } = entry {
                        <article class="log-row">
                            <span class="log-mark log-mark-essay"></span>
                            <div class="log-rail">
                                <p class="log-date">(entry.date())</p>
                                <p class="log-serial">(serial)</p>
                            </div>
                            <div class="log-card">
                                <span class="log-stamp">"essay"</span>
                                <h2 class="log-card-title font-display font-bold">
                                    <a class="oxlink" href=(format!("/thoughts/{slug}"))>(title)</a>
                                </h2>
                                <p class="mt-2.5 max-w-prose leading-relaxed text-ink2">(teaser)</p>
                                <div class="mt-4 flex flex-wrap items-baseline gap-3 font-meta text-xs">
                                    for t in tags.iter() {
                                        <a class="log-tag" href=(home_url(kind_param, Some(t)))>(format!("#{t}"))</a>
                                    }
                                    <a
                                        class="ml-auto text-ink2 no-underline hover:text-oxide"
                                        href=(format!("/thoughts/{slug}"))
                                    >link_label(label: "read →")</a>
                                </div>
                            </div>
                        </article>
                    }
                    if let Entry::Note { body, source, slug, .. } = entry {
                        <article class="log-row">
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
                                    <a class="log-permalink" href=(format!("/thoughts/{slug}"))>"permalink"</a>
                                </p>
                            </div>
                        </article>
                    }
                    if let Entry::Update { stamp, label, body, href, link_label: update_link_label, .. } = entry {
                        <article class="log-row items-baseline">
                            <span class="log-mark log-mark-update"></span>
                            <p class="log-date">(entry.date())</p>
                            <p class="log-update min-w-0">
                                <span class="log-update-stamp">(format!("[{stamp}]"))</span>
                                " "
                                <span class="text-patina">(format!("{label} ·"))</span>
                                " "
                                (body)
                                " "
                                <a class="log-update-link" href=(href)>
                                    link_label(label: update_link_label)
                                </a>
                            </p>
                        </article>
                    }
                }
                if let Some(run) = row.win() {
                    <article class="log-row items-baseline">
                        <span class="log-mark log-mark-update"></span>
                        <p class="log-date">(run.date.as_str())</p>
                        <p class="log-update min-w-0">
                            <span class="log-update-stamp">"[spire]"</span>
                            " "
                            <span class="text-patina">"win ·"</span>
                            " "
                            (format!(
                                "{}, Ascension {} — {} floors in {}.",
                                run.character,
                                run.ascension,
                                run.floors,
                                fmt_duration(run.run_time)
                            ))
                            " "
                            <a class="log-update-link" href="/spire">
                                link_label(label: "run log →")
                            </a>
                        </p>
                    </article>
                }
                if let Some(published) = row.lift() {
                    <article class="log-row items-baseline">
                        <span class="log-mark log-mark-update"></span>
                        <p class="log-date">(published.date.as_str())</p>
                        <p class="log-update min-w-0">
                            <span class="log-update-stamp">"[fitness]"</span>
                            " "
                            <span class="text-patina">"lift ·"</span>
                            " "
                            (format!(
                                "{} — {}",
                                published.workout.title,
                                workout_description(&published.workout)
                            ))
                            " "
                            <a
                                class="log-update-link"
                                href=(format!("/lifting/{}", published.workout.path))
                            >
                                link_label(label: "workout →")
                            </a>
                        </p>
                    </article>
                }
            }
        </section>
    ) }
}
