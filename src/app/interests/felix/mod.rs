use topcoat::{
    Result,
    asset::{Asset, asset},
    context::Cx,
    router::{error::not_found, error::redirect_permanent, page, path_param, route},
    view::{component, view},
};

use crate::{
    components::{back_link, ext_link, inline_popover, rail_section, shell},
    content::interests::interest,
};

struct Photo {
    slug: &'static str,
    src: Asset,
    alt: &'static str,
    stamp: &'static str,
    caption: &'static str,
    width: u16,
    height: u16,
}

#[path_param]
struct PhotoSlug(str);

const STUDIO_PORTRAIT: Asset = asset!("./photos/2023-portrait.webp");
const FELIX_HOME_JS: Asset = asset!("./felix-home.js");

const PHOTOS: &[Photo] = &[
    Photo {
        slug: "2022-attention",
        src: asset!("./photos/2022-attention.webp"),
        alt: "Felix sitting attentively on a hardwood floor and looking up.",
        stamp: "nov 2022",
        caption: "",
        width: 1200,
        height: 1600,
    },
    Photo {
        slug: "felix-close-up",
        src: asset!("./photos/felix-close-up.webp"),
        alt: "Close-up side view of Felix's nose as a hand reaches toward it.",
        stamp: "snoot",
        caption: "",
        width: 810,
        height: 540,
    },
    Photo {
        slug: "2022-walk-ready",
        src: asset!("./photos/2022-walk-ready.webp"),
        alt: "Felix indoors wearing a gray coat, scarf, and four small boots.",
        stamp: "dec 2022",
        caption: "Dr. Daniel got me a mug with this picture on it",
        width: 1200,
        height: 1600,
    },
    Photo {
        slug: "2023-overlook",
        src: asset!("./photos/2023-overlook.webp"),
        alt: "Felix in action mode",
        stamp: "aug 2023",
        caption: "POV: you're about to throw a ball",
        width: 1200,
        height: 1600,
    },
    Photo {
        slug: "2023-play-bow",
        src: asset!("./photos/2023-play-bow.webp"),
        alt: "Felix with Ruthie",
        stamp: "2023",
        caption: "Felix with his aunt Ruthie in North Carolina",
        width: 1600,
        height: 1200,
    },
    Photo {
        slug: "2023-sprint",
        src: asset!("./photos/2023-sprint.webp"),
        alt: "Felix running toward the camera through an autumn park.",
        stamp: "oct 2023",
        caption: "",
        width: 800,
        height: 1200,
    },
    Photo {
        slug: "2024-big-smile",
        src: asset!("./photos/2024-big-smile.webp"),
        alt: "Felix sitting on a dark tiled floor with a wide open-mouthed smile.",
        stamp: "may 2024",
        caption: "",
        width: 1200,
        height: 1600,
    },
    Photo {
        slug: "graceful",
        src: asset!("./photos/graceful.webp"),
        alt: "The epitome of grace. I'm sorry, you have to see it to believe it.",
        stamp: "oct 2023",
        caption: "The epitome of grace.",
        width: 1067,
        height: 1600,
    },
    Photo {
        slug: "phone-background",
        src: asset!("./photos/phone-background.webp"),
        alt: "Felix standing alertly on a tree stump in an autumn forest.",
        stamp: "oct 2023",
        caption: "",
        width: 158,
        height: 320,
    },
    Photo {
        slug: "2025-rug",
        src: asset!("./photos/2025-rug.webp"),
        alt: "Felix sitting on a rug, looking up at the camera with his tongue out.",
        stamp: "mar 2025",
        caption: "",
        width: 1083,
        height: 1600,
    },
    Photo {
        slug: "2025-chair",
        src: asset!("./photos/2025-chair.webp"),
        alt: "Felix sitting happily in a large dark armchair.",
        stamp: "apr 2025",
        caption: "",
        width: 1200,
        height: 1600,
    },
    Photo {
        slug: "2025-waterfront",
        src: asset!("./photos/2025-waterfront.webp"),
        alt: "Felix sitting on a picnic table beside a waterfront city skyline.",
        stamp: "apr 2025",
        caption: "LIC Waterfront",
        width: 1200,
        height: 1600,
    },
];

#[page("/felix")]
async fn felix() -> Result {
    view! { felix_page(initial_photo: "") }
}

#[page("/felix/{photo_slug}")]
async fn felix_photo(cx: &Cx) -> Result {
    let photo_slug = path_param::<PhotoSlug>(cx);
    if !PHOTOS.iter().any(|photo| photo.slug == photo_slug) {
        return Err(not_found().into());
    }
    view! { felix_page(initial_photo: photo_slug) }
}

#[component]
async fn felix_page(initial_photo: &str) -> Result {
    let meta = interest("felix");
    view! {
        shell(
            title: meta.title,
            active: "",
            hide_nav: true,
            <header class="felix-hero" aria-labelledby="felix-title">
                <img
                    class="felix-hero-media"
                    src=(STUDIO_PORTRAIT)
                    alt="Black-and-white studio portrait of Felix against a dark background."
                    width="1000"
                    height="667"
                    decoding="async"
                    fetchpriority="high"
                >
                <div class="felix-hero-copy">
                    <p class="felix-hero-stamp">"felix / oct 2023"</p>
                    <h1 id="felix-title" class="felix-hero-title">
                        inline_popover(
                            id: "felix-name",
                            label: meta.title,
                            <span class="inline-popover-preview">
                                "He's named after Supreme Court Justice Felix Frankfurter.
                             In fact, all of my family's dogs are named after Jewish Supreme Court Justices,
                             including Brandeis, two Cardozos, and a Ruthie."
                            </span>
                            ext_link(
                                class: "quiet-link",
                                href: "https://en.wikipedia.org/wiki/Felix_Frankfurter",
                                label: "Felix Frankfurter on Wikipedia →"
                            )
                        )
                    </h1>
                    <p class="felix-hero-lede">
                        inline_popover(
                            id: "carolina-dog",
                            label: "Carolina Dog",
                            <span class="inline-popover-preview">
                                "A primitive dog \"breed\" native to the southeastern United States."
                            </span>
                            ext_link(
                                class: "quiet-link",
                                href: "https://en.wikipedia.org/wiki/Carolina_Dog",
                                label: "Carolina Dog on Wikipedia →"
                            )
                        )

                        <span class="felix-hero-lede-separator" aria-hidden="true">
                            "·"
                        </span>

                        inline_popover(
                            id: "dingus",
                            label: "Dingus",
                            <span class="inline-popover-preview">
                                "Singular of \"Dingo\""
                            </span>
                            ext_link(
                                class: "quiet-link",
                                href: "https://en.wikipedia.org/wiki/Dingo",
                                label: "What's a Dingo? →"
                            )
                        )
                    </p>
                    <p class="felix-age" aria-label="Felix's estimated age">
                        <span class="felix-age-measure">
                            <span class="felix-age-label">"age"</span>
                            <time
                                class="felix-age-value"
                                datetime="2021-10-31"
                                data-felix-age=""
                                data-birthday="2021-10-31"
                            >
                                "born around Oct. 31, 2021"
                            </time>
                        </span>
                        <span class="felix-age-measure">
                            inline_popover(
                                id: "dog-years",
                                label: "dog years",
                                <span class="inline-popover-preview">
                                    "The first silly vibe-coded website I ever made; the older cousin of this site"
                                </span>
                                ext_link(
                                    class: "quiet-link",
                                    href: "https://www.saamd.com",
                                    label: "saamd.com →"
                                )
                            )
                            <span class="felix-age-value" data-felix-dog-age="">
                                "7 per calendar year"
                            </span>
                        </span>
                    </p>

                    <a class="felix-hero-scroll" href="#felix-notes">
                        "meet Felix "
                        <span aria-hidden="true">"↓"</span>
                    </a>
                </div>
            </header>
            <a class="felix-home" href="/" data-felix-home="true">
                <span class="link-arrow link-arrow-before" aria-hidden="true">
                    "<-"
                </span>
                " Ben"
            </a>
            <script type="module" src=(FELIX_HOME_JS)></script>
            rail_section(
                class: "mt-8",
                stamp: "photos",
                <div
                    class="felix-gallery"
                    data-felix-gallery=""
                    data-felix-gallery-initial=(initial_photo)
                    aria-label="Photos of Felix"
                >
                    for photo in PHOTOS {
                        <figure class="felix-photo">
                            <button
                                class="felix-photo-button"
                                type="button"
                                data-felix-gallery-trigger=""
                                data-felix-gallery-slug=(photo.slug)
                                data-felix-gallery-src=(photo.src)
                                data-felix-gallery-alt=(photo.alt)
                                data-felix-gallery-stamp=(photo.stamp)
                                data-felix-gallery-caption=(photo.caption)
                                aria-label=(format!("Expand photo: {}", photo.alt))
                            >
                                <img
                                    src=(photo.src)
                                    alt=(photo.alt)
                                    width=(photo.width)
                                    height=(photo.height)
                                    loading="lazy"
                                    decoding="async"
                                >
                                <span class="felix-photo-expand" aria-hidden="true">
                                    "Expand"
                                </span>
                            </button>
                            <figcaption>
                                <span class="felix-photo-stamp">(photo.stamp)</span>
                                <span>(photo.caption)</span>
                            </figcaption>
                        </figure>
                    }
                </div>
            )
            <dialog
                class="felix-lightbox"
                data-felix-gallery-dialog=""
                aria-labelledby="felix-lightbox-title"
                aria-describedby="felix-lightbox-help"
            >
                <div class="felix-lightbox-frame">
                    <button
                        class="felix-lightbox-close"
                        type="button"
                        data-felix-gallery-close=""
                        aria-label="Close photo gallery"
                    >
                        <span aria-hidden="true">"×"</span>
                    </button>
                    <div class="felix-lightbox-stage">
                        <button
                            class="felix-lightbox-nav felix-lightbox-prev"
                            type="button"
                            data-felix-gallery-prev=""
                            aria-label="Previous photo"
                        >
                            <span aria-hidden="true">"←"</span>
                        </button>
                        <img
                            class="felix-lightbox-image"
                            data-felix-gallery-image=""
                            alt=""
                        >
                        <button
                            class="felix-lightbox-nav felix-lightbox-next"
                            type="button"
                            data-felix-gallery-next=""
                            aria-label="Next photo"
                        >
                            <span aria-hidden="true">"→"</span>
                        </button>
                    </div>
                    <div class="felix-lightbox-details">
                        <p
                            id="felix-lightbox-title"
                            class="felix-lightbox-position"
                            data-felix-gallery-position=""
                            aria-live="polite"
                        >

                        </p>
                        <p class="felix-lightbox-caption" data-felix-gallery-caption="">

                        </p>
                    </div>
                    <p id="felix-lightbox-help" class="felix-lightbox-help">
                        "Use the left and right arrow keys to browse. Press Escape to close."
                    </p>
                </div>
            </dialog>
            back_link(href: "/", label: "~")
        )
    }
}

#[route(GET "/interests/felix")]
async fn legacy_felix() -> Result {
    Err(redirect_permanent("/felix").into())
}
