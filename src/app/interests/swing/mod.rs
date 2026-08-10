use topcoat::{
    Result,
    asset::{Asset, asset},
    context::Cx,
    router::{error::not_found, error::redirect_permanent, page, path_param, route},
    view::{component, view},
};

use crate::{
    components::{back_link, ext_link, page_head, rail_prose, rail_section, shell},
    content::interests::interest,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum MediaKind {
    Photo,
    Video,
}

impl MediaKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Photo => "Photo",
            Self::Video => "Video",
        }
    }
}

struct Media {
    slug: &'static str,
    src: Asset,
    preview: Asset,
    kind: MediaKind,
    alt: &'static str,
    caption: &'static str,
    width: u16,
    height: u16,
}

const SWING_GALLERY_JS: Asset = asset!("./swing-gallery.js");

const MEDIA: &[Media] = &[
    Media {
        slug: "guggenheim-olivia",
        src: asset!("./guggenheim-olivia.webp"),
        preview: asset!("./guggenheim-olivia.webp"),
        kind: MediaKind::Photo,
        alt: "Ben swing dancing with Olivia Meyer in the Guggenheim's rotunda.",
        caption: "At the Guggenheim with Olivia",
        width: 1238,
        height: 1651,
    },
    Media {
        slug: "mermaid-parade",
        src: asset!("./mermaid-parade.webp"),
        preview: asset!("./mermaid-parade.webp"),
        kind: MediaKind::Photo,
        alt: "Ben dancing with a woman in a green costume at the Mermaid Parade.",
        caption: "Mermaid Parade with Caryn",
        width: 1000,
        height: 667,
    },
    Media {
        slug: "with-eileen-photo",
        src: asset!("./with-eileen.webp"),
        preview: asset!("./with-eileen.webp"),
        kind: MediaKind::Photo,
        alt: "Ben and Eileen swing dancing on the Intrepid.",
        caption: "Dancing with Eileen on the Intrepid",
        width: 1134,
        height: 1294,
    },
    Media {
        slug: "with-laurel",
        src: asset!("./with-laurel.mp4"),
        preview: asset!("./with-laurel-poster.webp"),
        kind: MediaKind::Video,
        alt: "Ben swing dancing with Laurel DiSera on a crowded floor beneath an aircraft.",
        caption: "with Laurel",
        width: 848,
        height: 480,
    },
    Media {
        slug: "with-eileen-faster",
        src: asset!("./with-eileen-faster.mp4"),
        preview: asset!("./with-eileen-faster-poster.webp"),
        kind: MediaKind::Video,
        alt: "Ben and Eileen swing dancing in front of a live band.",
        caption: "Competing with Eileen",
        width: 1280,
        height: 720,
    },
    Media {
        slug: "with-eileen",
        src: asset!("./with-eileen.mp4"),
        preview: asset!("./with-eileen-video-poster.webp"),
        kind: MediaKind::Video,
        alt: "Ben and Eileen swing dancing at a social dance.",
        caption: "Competing with Eileen",
        width: 1280,
        height: 720,
    },
    Media {
        slug: "with-amanda-frim-fram",
        src: asset!("./with-amanda-frim-fram.mp4"),
        preview: asset!("./with-amanda-frim-fram-poster.webp"),
        kind: MediaKind::Video,
        alt: "Ben swing dancing with Amanda at the Frim Fram.",
        caption: "Dancing with Amanda at Frim Fram",
        width: 320,
        height: 568,
    },
];

#[path_param]
struct MediaSlug(str);

#[page("/swing")]
async fn swing() -> Result {
    view! { swing_page(initial_media: "") }
}

#[page("/swing/{media_slug}")]
async fn swing_media(cx: &Cx) -> Result {
    let media_slug = path_param::<MediaSlug>(cx);
    if !MEDIA.iter().any(|media| media.slug == media_slug) {
        return Err(not_found().into());
    }
    view! { swing_page(initial_media: media_slug) }
}

#[component]
async fn swing_page(initial_media: &str) -> Result {
    let meta = interest("swing");
    view! {
        shell(
            title: meta.title,
            active: "",
            page_head(stamp: meta.slug, title: meta.title, lede: meta.teaser)
            rail_prose(
                stamp: "",
                <p>
                    "Technically I started swing dancing at Wash U in St. Louis. I joined the swing dance
                team \"Swing Theory\" to spend more time with a girl."
                </p>
                <p>
                    "
                In 2023, I started swing dancing again. I've been dancing at least once or twice a month
                since then, sometimes as often as 4x a week.
            "
                </p>
                <p>
                    "If you're looking for swing events (in New York), I highly recommend "
                    ext_link(
                        class: "quiet-link",
                        href: "https://thisweekinswingnyc.com",
                        label: "thisweekinswingnyc.com"
                    )
                    " for a full list of pretty much every event in NYC."
                </p>
            )
            rail_section(
                class: "mt-12",
                stamp: "footage",
                <div
                    class="swing-gallery"
                    data-swing-gallery=""
                    data-swing-gallery-initial=(initial_media)
                    aria-label="Photos and videos of Ben swing dancing"
                >
                    for media in MEDIA {
                        <figure class="swing-media">
                            <button
                                class="swing-media-button"
                                type="button"
                                data-swing-gallery-trigger=""
                                data-swing-gallery-slug=(media.slug)
                                data-swing-gallery-src=(media.src)
                                data-swing-gallery-kind=(media.kind.label())
                                data-swing-gallery-alt=(media.alt)
                                data-swing-gallery-caption=(media.caption)
                                aria-label=(format!(
                                    "Open {}: {}", media.kind.label().to_lowercase(), media.alt
                                ))
                            >
                                if media.kind == MediaKind::Photo {
                                    <img
                                        src=(media.preview)
                                        alt=(media.alt)
                                        width=(media.width)
                                        height=(media.height)
                                        loading="lazy"
                                        decoding="async"
                                    >
                                } else {
                                    <img
                                        src=(media.preview)
                                        alt=""
                                        width=(media.width)
                                        height=(media.height)
                                        aria-hidden="true"
                                        loading="lazy"
                                        decoding="async"
                                    >
                                }
                                <span class="swing-media-kind" aria-hidden="true">
                                    (media.kind.label())
                                </span>
                                if media.kind == MediaKind::Video {
                                    <span class="swing-media-play" aria-hidden="true">
                                        "▶"
                                    </span>
                                }
                                <span class="swing-media-expand" aria-hidden="true">
                                    if media.kind == MediaKind::Video {
                                        "Play"
                                    } else {
                                        "Open"
                                    }
                                </span>
                            </button>
                            <figcaption>
                                <span class="swing-media-label">(media.kind.label())</span>
                                if media.slug == "with-amanda-frim-fram" {
                                    <span>
                                        "Dancing with Amanda at "
                                        ext_link(
                                            class: "quiet-link",
                                            href: "https://frimframjam.com/",
                                            label: "Frim Fram ↗"
                                        )
                                    </span>
                                } else {
                                    <span>(media.caption)</span>
                                }
                            </figcaption>
                        </figure>
                    }
                </div>
            )
            <dialog
                class="swing-lightbox"
                data-swing-gallery-dialog=""
                aria-labelledby="swing-lightbox-title"
                aria-describedby="swing-lightbox-help"
            >
                <div class="swing-lightbox-frame">
                    <button
                        class="swing-lightbox-close"
                        type="button"
                        data-swing-gallery-close=""
                        aria-label="Close gallery"
                    >
                        <span aria-hidden="true">"×"</span>
                    </button>
                    <div class="swing-lightbox-stage">
                        <button
                            class="swing-lightbox-nav swing-lightbox-prev"
                            type="button"
                            data-swing-gallery-prev=""
                            aria-label="Previous item"
                        >
                            <span aria-hidden="true">"←"</span>
                        </button>
                        <img
                            class="swing-lightbox-image"
                            data-swing-gallery-image=""
                            alt=""
                        >
                        <video
                            class="swing-lightbox-video"
                            data-swing-gallery-video=""
                            controls=""
                            playsinline=""
                            preload="metadata"
                            aria-label=""
                            hidden=""
                        >

                        </video>
                        <button
                            class="swing-lightbox-nav swing-lightbox-next"
                            type="button"
                            data-swing-gallery-next=""
                            aria-label="Next item"
                        >
                            <span aria-hidden="true">"→"</span>
                        </button>
                    </div>
                    <div class="swing-lightbox-details">
                        <p
                            id="swing-lightbox-title"
                            class="swing-lightbox-position"
                            data-swing-gallery-position=""
                            aria-live="polite"
                        >

                        </p>
                        <p class="swing-lightbox-caption" data-swing-gallery-caption="">

                        </p>
                    </div>
                    <p id="swing-lightbox-help" class="swing-lightbox-help">
                        "Use the left and right arrow keys to browse. Press Escape to close."
                    </p>
                </div>
            </dialog>
            <script type="module" src=(SWING_GALLERY_JS)></script>
            back_link(href: "/", label: "~")
        )
    }
}

#[route(GET "/interests/swing")]
async fn legacy_swing() -> Result {
    Err(redirect_permanent("/swing").into())
}
