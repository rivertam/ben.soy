//! The margin rail: stamped rows, grouped rows, page heads, and the closing
//! back-link row. Every `.rail-row` participates in site-wide j/k navigation
//! automatically. A row can opt into Enter by setting `enter_href`, or its
//! child markup can mark any link/button with `data-rail-enter`.

use topcoat::{
    Result,
    view::{View, component, view},
};

/// A page's opening rail row: a mono stamp in the margin, a display-face title,
/// and an optional one-line lede (pass `""` to omit). `enter_href` optionally
/// makes Enter navigate from this row without changing its visible markup.
#[component]
pub async fn page_head(
    #[default("")] enter_href: &str,
    stamp: &str,
    title: &str,
    lede: &str,
) -> Result {
    let enter_href = (!enter_href.is_empty()).then_some(enter_href);
    view! {
        <header
            class="rail-row mt-16"
            data-rail-href=(enter_href)
        >
            <p class="rail-stamp rail-stamp-label">(stamp)</p>
            <div class="min-w-0">
                <h1 class="font-display text-4xl font-bold tracking-tight">(title)</h1>
                if !lede.is_empty() {
                    <p class="mt-3 max-w-prose text-ink2">(lede)</p>
                }
            </div>
        </header>
    }
}

/// One rail row: a stamped label in the margin column, the body in the
/// content column. `stamp: ""` renders the empty spacer cell instead (prose
/// continuation rows). `class` is optional extra classes on the row — it
/// defaults to `"mt-10"`, so pass e.g. `class: "mt-6"` to tighten the top
/// margin or `class: ""` inside an already-spaced parent. Child markup follows
/// the named properties, e.g. `rail_section(stamp: "links", <p>"…"</p>)`.
/// `enter_href` optionally makes Enter navigate from this row; for a button
/// or other child action, put `data-rail-enter` on that child instead.
#[component]
pub async fn rail_section(
    #[default("mt-10")] class: &str,
    #[default("")] enter_href: &str,
    stamp: &str,
    child: View,
) -> Result {
    let row_class = if class.is_empty() {
        "rail-row".to_string()
    } else {
        format!("rail-row {class}")
    };
    let enter_href = (!enter_href.is_empty()).then_some(enter_href);
    view! {
        <div
            class=(row_class.as_str())
            data-rail-href=(enter_href)
        >
            if stamp.is_empty() {
                <div></div>
            } else {
                <p class="rail-stamp rail-stamp-label">(stamp)</p>
            }
            <div class="min-w-0">(child)</div>
        </div>
    }
}

/// A rail row whose body is running prose: paragraphs at reading measure in
/// the secondary ink. `class` and `enter_href` work as on [`rail_section`].
#[component]
pub async fn rail_prose(
    #[default("mt-10")] class: &str,
    #[default("")] enter_href: &str,
    stamp: &str,
    child: View,
) -> Result {
    let prose = view! { <div class="max-w-prose space-y-4 text-ink2">(child)</div> }?;
    view! { rail_section(class: class, enter_href: enter_href, stamp: stamp, (prose)) }
}

/// Visually and semantically groups related rail rows with one bracket in the
/// gutter. `label` names the relationship for assistive technology without
/// adding another visible line of metadata. Add `rail-group-compact` through
/// `class` when the group lives inside a card rather than on the full rail.
#[component]
pub async fn rail_group(#[default("")] class: &str, label: &str, child: View) -> Result {
    let group_class = if class.is_empty() {
        "rail-group".to_string()
    } else {
        format!("rail-group {class}")
    };
    view! {
        <div class=(group_class.as_str()) role="group" aria-label=(label)>
            (child)
        </div>
    }
}

/// A page's closing rail row: a quiet link back up to the section index.
#[component]
pub async fn back_link(href: &str, label: &str) -> Result {
    let label = label.strip_prefix("← ").unwrap_or(label);
    view! {
        <div class="rail-row mt-14" data-rail-href=(href)>
            <div></div>
            <p class="min-w-0 font-meta text-sm">
                <a class="quiet-link" href=(href)>
                    <span class="link-arrow link-arrow-before" aria-hidden="true">"<-"</span>
                    (label)
                </a>
            </p>
        </div>
    }
}
