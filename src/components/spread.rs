//! The spread: components for pages that leave the centered column — a
//! full-bleed band, an off-rail identity masthead, a stamped sidebar prose
//! column, and a rubber-stamp mark. Extracted for the dispatch-desk planes
//! page; felix's hero uses the same full-bleed trick inline and can migrate.

use topcoat::{
    Result,
    view::{View, component, view},
};

/// A band that escapes the shell's centered `max-w-4xl` column to span the
/// viewport (`.full-bleed` in `styles/site.css`). `class` adds the band's own
/// surface styling, e.g. `"desk-band"`.
#[component]
pub async fn full_bleed(#[default("")] class: &str, child: View) -> Result {
    let band = if class.is_empty() {
        "full-bleed".to_string()
    } else {
        format!("full-bleed {class}")
    };
    view! { <div class=(band.as_str())>(child)</div> }
}

/// The identity masthead for an immersive page (no site header): a quiet way
/// home, a mono date stamp, and the page's `<h1>` — a log entry's anatomy,
/// set free of the rail grid. `class` lands on the `<header>` for layout
/// (grid areas etc.).
#[component]
pub async fn doc_head(
    #[default("")] class: &str,
    stamp: &str,
    title: &str,
    #[default("/")] home_href: &str,
    #[default("Ben")] home_label: &str,
) -> Result {
    let head = if class.is_empty() {
        "doc-head".to_string()
    } else {
        format!("doc-head {class}")
    };
    view! {
        <header class=(head.as_str()) data-rail-item="">
            <p class="doc-head-home" data-rail-item="" data-rail-href=(home_href)>
                <a class="quiet-link" href=(home_href)>
                    <span class="link-arrow link-arrow-before" aria-hidden="true">"<-"</span>
                    " "
                    (home_label)
                </a>
            </p>
            <p class="doc-head-stamp">(stamp)</p>
            <h1 class="doc-head-title">(title)</h1>
        </header>
    }
}

/// A stamped prose column for panes too narrow for the rail grid: the
/// sidebar voice of [`rail_prose`](crate::components::rail_prose). The stamp
/// keeps the house label grammar (lowercase mono, wide tracking).
#[component]
pub async fn margin_notes(#[default("")] class: &str, stamp: &str, child: View) -> Result {
    let aside = if class.is_empty() {
        "margin-notes".to_string()
    } else {
        format!("margin-notes {class}")
    };
    view! {
        <aside class=(aside.as_str()) aria-label=(stamp)>
            <p class="margin-notes-stamp">(stamp)</p>
            <div class="margin-notes-body max-w-prose space-y-4 text-ink2">(child)</div>
        </aside>
    }
}

/// An inline rubber-stamp mark: mono caps in a canted oxide border. For
/// totals, verdicts, and other pressed-into-the-paper moments.
#[component]
pub async fn stamp_seal(#[default("")] class: &str, text: String) -> Result {
    let seal = if class.is_empty() {
        "stamp-seal".to_string()
    } else {
        format!("stamp-seal {class}")
    };
    view! { <span class=(seal.as_str())>(text.as_str())</span> }
}
