pub(crate) mod api;
pub(crate) mod db;
pub(crate) mod runs;

use benjisponge::data::Data;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{HeaderValue, error::redirect_permanent, header, page, query_params, route},
    view::view,
};

use self::runs::{self as spire_runs, Run, fmt_duration};
use crate::{
    components::{back_link, link_label, page_head, rail_section, shell},
    content::interests::interest,
};

/// The result cell's classes: wins in patina, deaths in body ink, abandons
/// muted — the color does the scanning work in a 200-row table.
fn result_class(run: &Run) -> &'static str {
    if run.win {
        "py-1.5 pr-4 text-patina"
    } else if run.abandoned {
        "py-1.5 pr-4 text-muted"
    } else {
        "py-1.5 pr-4 text-ink2"
    }
}

const PER_PAGE: usize = 50;

#[query_params(error = redirect("?"))]
struct SpireQuery {
    page: Option<String>,
}

/// Which slice of the log a `?page=` value selects: the clamped 1-based
/// page, the page count, and the index range it covers. Out-of-range and
/// garbage requests clamp instead of erroring, like the log page's filters.
fn page_slice(total: usize, requested: usize) -> (usize, usize, std::ops::Range<usize>) {
    let pages = total.div_ceil(PER_PAGE).max(1);
    let page = requested.clamp(1, pages);
    let start = (page - 1) * PER_PAGE;
    let end = (start + PER_PAGE).min(total);
    (page, pages, start..end)
}

/// Pager link target. Page 1 is the bare URL so it shares the canonical
/// page's edge-cache entry; the fragment lands readers back at the table.
fn page_url(page: usize) -> String {
    if page <= 1 {
        "/spire#run-log".to_string()
    } else {
        format!("/spire?page={page}#run-log")
    }
}

/// Deep link to the run-log slice holding `target` — how the log timeline's
/// collapsed win clusters land readers near their runs. Best-effort by
/// design: runs syncing in between renders shift everything a slot.
pub(crate) fn run_page_url(runs: &[Run], target: &Run) -> String {
    let index = runs
        .iter()
        .position(|run| run.game == target.game && run.id == target.id)
        .unwrap_or(0);
    page_url(index / PER_PAGE + 1)
}

#[page("/spire")]
async fn spire(cx: &Cx) -> Result {
    let q = query_params::<SpireQuery>(cx)?;
    let requested = q.page.as_deref().and_then(|p| p.parse().ok()).unwrap_or(1);
    let meta = interest("spire");
    let log = spire_runs::load(app_context::<Data>(cx)).await;
    let (page, pages, range) = page_slice(log.runs.len(), requested);
    let visible = &log.runs[range];
    let page_numbers: Vec<usize> = (1..=pages).collect();

    let total = log.runs.len();
    let wins = log.runs.iter().filter(|r| r.win).count();
    let abandoned = log.runs.iter().filter(|r| r.abandoned && !r.win).count();
    let deaths = total - wins - abandoned;
    let best_win = log.runs.iter().filter(|r| r.win).map(|r| r.ascension).max();
    let since = log.runs.last().map(|r| r.date.as_str()).unwrap_or("");
    let summary = match best_win {
        Some(best) => format!(
            "{total} runs since {since} — {wins} wins · {deaths} deaths · {abandoned} \
             abandoned · best win at Ascension {best}"
        ),
        None => format!("{total} runs since {since} — no wins yet"),
    };

    view! {
        // Fresh runs appear within a minute; CDN honors s-maxage (see docs/railway-deploy.md).
        ((header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=0, s-maxage=60")))
        shell(
            page: meta.title,
            active: "",
            page_head(stamp: meta.slug, title: meta.title, lede: meta.teaser)
            rail_section(
                class: "mt-10",
                stamp: "run log",
                <p class="max-w-prose text-ink2">
                    "There's a cron job on my computer that syncs up my spire runs to this site"
                </p>
                if !log.live {
                    <p class="mt-3 font-meta text-sm text-muted">
                        "The run database is unreachable right now — check back shortly."
                    </p>
                }
                if log.live && log.runs.is_empty() {
                    <p class="mt-3 font-meta text-sm text-muted">
                        "No runs synced yet."
                    </p>
                }
                if !log.runs.is_empty() {
                    <p id="run-log" class="mt-3 font-meta text-[13px] text-muted">
                        (summary.as_str())
                    </p>
                    <div class="mt-4 overflow-x-auto">
                        <table class="w-full border-collapse font-meta text-[13px]">
                            <thead>
                                <tr class="text-left text-muted">
                                    <th class="pb-2 pr-4 font-normal">"date"</th>
                                    <th class="pb-2 pr-4 font-normal">"game"</th>
                                    <th class="pb-2 pr-4 font-normal">"character"</th>
                                    <th class="pb-2 pr-4 font-normal">"result"</th>
                                    <th class="pb-2 pr-4 text-right font-normal">"asc"</th>
                                    <th class="pb-2 pr-4 text-right font-normal">"floors"</th>
                                    <th class="pb-2 text-right font-normal">"time"</th>
                                </tr>
                            </thead>
                            <tbody>
                                for run in visible.iter() {
                                    <tr class="border-t border-hairline">
                                        <td class="py-1.5 pr-4 whitespace-nowrap text-muted">
                                            (run.date.as_str())
                                        </td>
                                        <td class="py-1.5 pr-4 whitespace-nowrap text-muted">
                                            (run.game_label())
                                        </td>
                                        <td class="py-1.5 pr-4 whitespace-nowrap">
                                            (run.character.as_str())
                                            if run.game_mode != "standard" {
                                                <span class="text-muted">
                                                    (format!(" · {}", run.game_mode))
                                                </span>
                                            }
                                        </td>
                                        <td class=(result_class(run))>
                                            (run.result_label())
                                            if let Some(kind) = &run.kill_kind {
                                                <span class="text-muted">(format!(" ({kind})"))</span>
                                            }
                                        </td>
                                        <td class="py-1.5 pr-4 text-right tabular-nums">
                                            (format!("{}", run.ascension))
                                        </td>
                                        <td class="py-1.5 pr-4 text-right tabular-nums">
                                            (format!("{}", run.floors))
                                        </td>
                                        <td
                                            class="py-1.5 text-right whitespace-nowrap tabular-nums"
                                        >
                                            (fmt_duration(run.run_time))
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </div>
                    if pages > 1 {
                        <nav
                            class="mt-4 flex flex-wrap items-baseline gap-3 font-meta text-[13px]"
                        >
                            if page > 1 {
                                <a class="quiet-link" href=(page_url(page - 1))>
                                    "← newer"
                                </a>
                            }
                            for n in page_numbers.iter() {
                                <a
                                    class=(if *n == page {
                                        "log-chip log-chip-active"
                                    } else {
                                        "log-chip"
                                    })
                                    href=(page_url(*n))
                                >
                                    (format!("{n}"))
                                </a>
                            }
                            if page < pages {
                                <a class="quiet-link" href=(page_url(page + 1))>
                                    "older →"
                                </a>
                            }
                        </nav>
                    }
                }
            )
            rail_section(
                class: "mt-6",
                stamp: "links",
                <p class="flex flex-wrap gap-x-4 gap-y-1 font-meta text-sm">
                    <a
                        class="oxlink"
                        href="https://reddit.com/r/slaythespire/comments/jkqx35/annotated_run_synopsis_my_second_a20_win_only_a/"
                    >
                        link_label(
                            label: "this one crazy run I got in Slay the Spire 1"
                        )
                    </a>
                </p>
            )
            back_link(href: "/", label: "~")
        )
    }
}

#[route(GET "/interests/spire")]
async fn legacy_spire() -> Result {
    Err(redirect_permanent("/spire").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_slice_covers_the_log_exactly_once() {
        // 199 runs at 50/page → 4 pages, the last one short.
        assert_eq!(page_slice(199, 1), (1, 4, 0..50));
        assert_eq!(page_slice(199, 4), (4, 4, 150..199));
        // A full multiple doesn't grow a phantom page.
        assert_eq!(page_slice(100, 2), (2, 2, 50..100));
    }

    #[test]
    fn out_of_range_pages_clamp() {
        assert_eq!(page_slice(199, 0), (1, 4, 0..50));
        assert_eq!(page_slice(199, 99), (4, 4, 150..199));
        // A single short page absorbs any request; empty logs stay sane.
        assert_eq!(page_slice(12, 7), (1, 1, 0..12));
        assert_eq!(page_slice(0, 3), (1, 1, 0..0));
    }

    #[test]
    fn page_one_is_the_bare_url() {
        assert_eq!(page_url(1), "/spire#run-log");
        assert_eq!(page_url(3), "/spire?page=3#run-log");
    }
}
