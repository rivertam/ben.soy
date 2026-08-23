//! Inline citation popovers.

use topcoat::{
    Result,
    view::{View, component, view},
};

/// An inline, dismissible popover for citations and other short asides.
/// `id` must be unique on the page and a valid CSS custom-ident fragment.
/// `heading` can override the panel's visible kicker without changing the
/// trigger text or accessible dialog label.
/// `rail_side` can art-direct a wide-screen note to the left or right edge;
/// leave it empty for the automatic solver.
/// `rail_slot` can name a page-owned `[data-inline-popover-slot]` marker for
/// a free-floating composition; collision avoidance still has the final say.
/// Keep the child phrasing content; use block-styled spans for multiple paragraphs.
#[component]
pub async fn inline_popover(
    id: &str,
    label: &str,
    #[default("")] heading: &str,
    #[default("")] rail_side: &str,
    #[default("")] rail_slot: &str,
    child: View,
) -> Result {
    let anchor_name = format!("anchor-name: --inline-popover-{};", id);
    let position_anchor = format!("position-anchor: --inline-popover-{};", id);
    let href = format!("#{}", id);
    let heading = if heading.is_empty() { label } else { heading };
    let rail_side = matches!(rail_side, "left" | "right").then_some(rail_side);
    let rail_slot = (!rail_slot.is_empty()).then_some(rail_slot);
    view! {
        <a
            href=(href.as_str())
            role="button"
            class="inline-popover-trigger oxlink"
            data-inline-popover-trigger=(id)
            data-inline-popover-rail-side=(rail_side)
            data-inline-popover-rail-slot=(rail_slot)
            aria-controls=(id)
            aria-expanded="false"
            aria-haspopup="dialog"
            style=(anchor_name.as_str())
        >(label)</a>
        <span
            id=(id)
            class="inline-popover-panel"
            data-inline-popover-panel=""
            popover="auto"
            role="dialog"
            aria-label=(label)
            style=(position_anchor.as_str())
        >
            <button
                type="button"
                class="inline-popover-close"
                popovertarget=(id)
                popovertargetaction="hide"
                data-inline-popover-close=""
                aria-label="Close popover"
            >"×"</button>
            <span class="inline-popover-kicker" data-inline-popover-kicker="">(heading)</span>
            (child)
        </span>
    }
}

#[cfg(test)]
mod tests {
    use topcoat::context::Cx;

    use super::*;

    #[tokio::test]
    async fn trigger_is_a_fragmenting_inline_invoker() {
        let cx = Cx::default();
        let __cx = &cx;
        let result: Result = view! {
            inline_popover(
                id: "test-source",
                label: "A long citation label",
                <span>"Citation detail"</span>
            )
        };
        let html = result.unwrap().render(__cx);

        assert!(html.contains("<a href=\"#test-source\" role=\"button\""));
        assert!(html.contains("data-inline-popover-trigger=\"test-source\""));
        assert!(html.contains("aria-controls=\"test-source\""));
        assert!(html.contains("id=\"test-source\" class=\"inline-popover-panel\""));
        assert!(html.contains("data-inline-popover-panel=\"\" popover=\"auto\""));
        assert!(html.contains("data-inline-popover-close=\"\""));
        assert!(!html.contains("popovertarget=\"test-source\">A long citation label"));
    }

    #[tokio::test]
    async fn heading_override_does_not_rename_the_trigger_or_dialog() {
        let cx = Cx::default();
        let __cx = &cx;
        let result: Result = view! {
            inline_popover(
                id: "test-aside",
                label: "Clickable question?",
                heading: "A pet peeve",
                <span>"Aside detail"</span>
            )
        };
        let html = result.unwrap().render(__cx);

        assert!(html.contains(">Clickable question?</a>"));
        assert!(html.contains("role=\"dialog\" aria-label=\"Clickable question?\""));
        assert!(html.contains(
            "class=\"inline-popover-kicker\" data-inline-popover-kicker=\"\">A pet peeve</span>"
        ));
    }

    #[tokio::test]
    async fn rail_side_is_an_optional_art_direction_hint() {
        let cx = Cx::default();
        let __cx = &cx;
        let result: Result = view! {
            inline_popover(
                id: "left-note",
                label: "Left note",
                rail_side: "left",
                <span>"Aside detail"</span>
            )
        };
        let html = result.unwrap().render(__cx);

        assert!(html.contains("data-inline-popover-rail-side=\"left\""));
    }

    #[tokio::test]
    async fn rail_slot_is_an_optional_collision_aware_target() {
        let cx = Cx::default();
        let __cx = &cx;
        let result: Result = view! {
            inline_popover(
                id: "placed-note",
                label: "Placed note",
                rail_slot: "hero-lower-right",
                <span>"Aside detail"</span>
            )
        };
        let html = result.unwrap().render(__cx);

        assert!(html.contains("data-inline-popover-rail-slot=\"hero-lower-right\""));
    }

    #[test]
    fn shared_overlay_driver_opens_inline_invokers() {
        const DRIVER: &str = include_str!("browser/modals.js");

        assert!(DRIVER.contains("[data-inline-popover-trigger]"));
        assert!(DRIVER.contains("showPopover({ source: popoverTrigger })"));
        assert!(DRIVER.contains("data-inline-popover-rail-active"));
        assert!(DRIVER.contains("[data-dont-obstruct]"));
        assert!(DRIVER.contains("layoutRail"));
        assert!(DRIVER.contains("REVEAL_SCROLL_Y = 100"));
        assert!(DRIVER.contains("Math.min(REVEAL_SCROLL_Y, maxScroll)"));
        assert!(DRIVER.contains("readerConsented = true"));
        assert!(DRIVER.contains("const bandEdges ="));
        assert!(DRIVER.contains("selectedEntries.has(hovered)"));
        assert!(DRIVER.contains("aria-expanded"));
    }
}
