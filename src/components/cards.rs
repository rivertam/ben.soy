//! Cards: index entries and YouTube thumbnails.

use topcoat::{
    Result,
    view::{component, view},
};

use super::links::{link_label, split_link_label};

/// One entry on an index page (thoughts, interests): stamp in the margin,
/// oxlinked display title, one-line teaser.
#[component]
pub async fn index_card(stamp: &str, href: String, title: &str, teaser: &str) -> Result {
    view! {
        <article class="rail-row" data-rail-href=(href.as_str())>
            <p class="rail-stamp">(stamp)</p>
            <div class="min-w-0">
                <h2 class="font-display text-2xl leading-snug font-semibold">
                    <a class="oxlink" href=(href.as_str())>(title)</a>
                </h2>
                <p class="mt-1.5 max-w-prose text-ink2">(teaser)</p>
            </div>
        </article>
    }
}

/// A YouTube thumbnail card. Both the watch URL and the thumbnail derive
/// from the one video id, so they can't drift apart.
#[component]
pub async fn video_card(youtube_id: &str, label: &str) -> Result {
    let href = format!("https://www.youtube.com/watch?v={youtube_id}");
    let thumb = format!("https://img.youtube.com/vi/{youtube_id}/mqdefault.jpg");
    let (alt, _) = split_link_label(label);
    view! {
        <a class="video-card" href=(href.as_str())>
            <img src=(thumb.as_str()) alt=(alt) loading="lazy">
            <span class="video-card-label font-meta text-sm text-ink2">
                link_label(label: label)
            </span>
        </a>
    }
}
