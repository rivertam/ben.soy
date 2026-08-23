use topcoat::{
    Result,
    asset::{Asset, asset},
    router::{error::redirect_permanent, page, route},
    view::view,
};

use crate::{
    components::{back_link, ext_link, inline_popover, page_head, rail_section, shell, video_card},
    content::interests::interest,
};

const GLOVE80: Asset = asset!("./glove80.webp");
const ERGODOX_EZ: Asset = asset!("./ergodox-ez.webp");
const KINESIS_ADVANTAGE2: Asset = asset!("./kinesis-advantage2.webp");

#[page("/keyboards")]
async fn keyboards() -> Result {
    let meta = interest("keyboards");
    view! {
        shell(
            page: meta.title,
            active: "",
            page_head(stamp: meta.slug, title: meta.title, lede: meta.teaser)
            rail_section(
                class: "mt-4",
                stamp: "2024",
                <div>
                    "Switched to a "
                    inline_popover(
                        id: "glove80-cite",
                        label: "Glove80",
                        <img
                            class="inline-popover-photo"
                            src=(GLOVE80)
                            alt="MoErgo Glove80 split keyboard with contoured key wells and white low-profile keycaps."
                            width="900"
                            height="675"
                            loading="lazy"
                            decoding="async"
                        >
                        ext_link(
                            class: "quiet-link",
                            href: "https://www.moergo.com/collections/glove80-keyboards",
                            label: "MoErgo Glove80 →"
                        )
                    )
                    "."
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2021",
                <div>
                    "Custom "
                    inline_popover(
                        id: "dactyl-manuform-cite",
                        label: "Dactyl Manuform",
                        <span class="inline-popover-preview">
                            "A split, sculpted keyboard whose Manuform thumb clusters curve away from the user. "
                            "My video below is honestly one of the best showcases of what one is."
                        </span>
                    )
                    " from "
                    inline_popover(
                        id: "oh-keycaps-cite",
                        label: "Oh, Keycaps!",
                        <span class="inline-popover-preview">
                            "The vendor that builds and sells Dactyl and Dactyl Manuform keyboards."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://ohkeycaps.com/collections/in-stock-dactyls-manuforms",
                            label: "Oh, Keycaps! →"
                        )
                    )
                    " — "
                    inline_popover(
                        id: "marble-case-cite",
                        label: "marble case",
                        <span class="inline-popover-preview">
                            "A case finish that lets the board's LEDs shine through more visibly than darker colors."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://ohkeycaps.com/pages/dactyl-and-dactyl-manuform-faq",
                            label: "case FAQ →"
                        )
                    )
                    ", lubed "
                    inline_popover(
                        id: "zilents-cite",
                        label: "67g Zilents",
                        <span class="inline-popover-preview">
                            "Silent tactile switches; the 67 g variant is rated by Zeal at bottom-out force."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://zealpc.net/products/zilent",
                            label: "Zeal Zilent V2 →"
                        )
                    )
                    ", and "
                    inline_popover(
                        id: "sa-keycaps-cite",
                        label: "SA keycaps",
                        <span class="inline-popover-preview">
                            "SA-profile ABS keycaps; Oh, Keycaps!' Dactyl Manuform listing uses SA unless noted otherwise."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://ohkeycaps.com/products/5x6-dactyl-manuform-keycaps",
                            label: "Dactyl Manuform keycaps →"
                        )
                    )
                    "."
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "footage",
                <div class="flex flex-wrap gap-5">
                    video_card(youtube_id: "yZl30vWuERs", label: "the keyboard →")
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2017",
                <div>
                    "Tried an "
                    inline_popover(
                        id: "ergodox-ez-cite",
                        label: "ErgoDox EZ",
                        <img
                            class="inline-popover-photo"
                            src=(ERGODOX_EZ)
                            alt="ErgoDox EZ Glow split keyboard, tented, with cyan backlighting."
                            width="900"
                            height="563"
                            loading="lazy"
                            decoding="async"
                        >
                        <p>"
                          The lack of keywells ruins it for me. It was just such a pain to use
                          this keyboard. At least with a normal brick keyboard, I'm used to
                          the typical pains. Idk, I did it for 6 months and the kinesis felt
                          soooo good to get back to.
                        "</p>
                        ext_link(
                            class: "quiet-link",
                            href: "https://ergodox-ez.com/",
                            label: "ErgoDox EZ →"
                        )
                    )
                    ", then went back to the "
                    inline_popover(
                        id: "kinesis-return-cite",
                        label: "Kinesis Advantage II",
                        <img
                            class="inline-popover-photo"
                            src=(KINESIS_ADVANTAGE2)
                            alt="Kinesis Advantage2 contoured keyboard with blue home-row keycaps."
                            width="900"
                            height="674"
                            loading="lazy"
                            decoding="async"
                        >
                        ext_link(
                            class: "quiet-link",
                            href: "https://kinesis-ergo.com/shop/advantage2/",
                            label: "Kinesis Advantage2 →"
                        )
                    )
                    " until the Dactyl."
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2015",
                <div>
                    "Started with a "
                    inline_popover(
                        id: "kinesis-first-cite",
                        label: "Kinesis Advantage II",
                        <img
                            class="inline-popover-photo"
                            src=(KINESIS_ADVANTAGE2)
                            alt="Kinesis Advantage2 contoured keyboard with blue home-row keycaps."
                            width="900"
                            height="674"
                            loading="lazy"
                            decoding="async"
                        >
                        <p>"
                          Some fairly potent neurochemicals seem to release in my brain
                          when I look at this picture. Behold the splendor.
                        "</p>
                        ext_link(
                            class: "quiet-link",
                            href: "https://kinesis-ergo.com/shop/advantage2/",
                            label: "Kinesis Advantage2 →"
                        )
                    )
                    "."
                </div>
            )
            rail_section(
                class: "mt-6",
                stamp: "stats",
                <div>
                    inline_popover(
                        id: "typeracer-cite",
                        label: "TypeRacer speed",
                        <span class="inline-popover-preview">
                            "A multiplayer typing competition; this profile records my race statistics."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://data.typeracer.com/pit/profile?user=rivertam",
                            label: "my TypeRacer profile →"
                        )
                    )
                    ": 121 lately, 165 peak."
                </div>
            )
            back_link(href: "/", label: "~")
        )
    }
}

#[route(GET "/keys")]
async fn legacy_keys() -> Result {
    Err(redirect_permanent("/keyboards").into())
}

#[route(GET "/interests/keys")]
async fn legacy_interest_keys() -> Result {
    Err(redirect_permanent("/keyboards").into())
}

#[route(GET "/interests/keyboards")]
async fn legacy_interest_keyboards() -> Result {
    Err(redirect_permanent("/keyboards").into())
}
