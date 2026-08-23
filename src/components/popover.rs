//! Inline citation popovers.

use topcoat::{
    Result,
    view::{View, component, view},
};

/// An inline, dismissible popover for citations and other short asides.
/// `id` must be unique on the page and a valid CSS custom-ident fragment.
/// Keep the child phrasing content; use block-styled spans for multiple paragraphs.
#[component]
pub async fn inline_popover(id: &str, label: &str, child: View) -> Result {
    let anchor_name = format!("anchor-name: --inline-popover-{};", id);
    let position_anchor = format!("position-anchor: --inline-popover-{};", id);
    let href = format!("#{}", id);
    view! {
        <a
            href=(href.as_str())
            role="button"
            class="inline-popover-trigger oxlink"
            data-inline-popover-trigger=(id)
            aria-controls=(id)
            aria-expanded="false"
            aria-haspopup="dialog"
            style=(anchor_name.as_str())
        >(label)</a>
        <span
            id=(id)
            class="inline-popover-panel"
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
                aria-label="Close popover"
            >"×"</button>
            <span class="inline-popover-kicker">(label)</span>
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
        assert!(!html.contains("popovertarget=\"test-source\">A long citation label"));
    }

    #[test]
    fn shared_overlay_driver_opens_inline_invokers() {
        const DRIVER: &str = include_str!("browser/modals.js");

        assert!(DRIVER.contains("[data-inline-popover-trigger]"));
        assert!(DRIVER.contains("showPopover({ source: popoverTrigger })"));
        assert!(DRIVER.contains("aria-expanded"));
    }
}
