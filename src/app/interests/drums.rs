use topcoat::{
    Result,
    router::{error::redirect_permanent, page, route},
    view::view,
};

use crate::{
    components::{back_link, ext_link, inline_popover, page_head, rail_section, shell, video_card},
    content::interests::interest,
};

#[page("/drums")]
async fn drums() -> Result {
    let meta = interest("drums");
    view! {
        shell(
            title: meta.title,
            active: "interests",
            page_head(stamp: meta.slug, title: meta.title, lede: meta.teaser)
            rail_section(
                class: "mt-4",
                stamp: "footage",
                <div class="flex flex-wrap gap-5">
                    video_card(
                        youtube_id: "VaKI7J2M2Ms",
                        label: "Taylor Swift cover →"
                    )
                    video_card(
                        youtube_id: "8lrjsP1KWrY",
                        label: "Manchester Orchestra cover →"
                    )
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2006",
                <div>
                    "Took my first drum lesson at "
                    inline_popover(
                        id: "island-lake-cite",
                        label: "Island Lake",
                        <span class="inline-popover-preview">
                            "Summer camp in Pennsylvania. Other skills I learned include light mountain \
                         biking, the music from Newsies and Bat Boy, and how to play \
                         Warcraft III: Frozen Throne"
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://www.islandlake.com/",
                            label: "islandlake.com →"
                        )
                    )
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2007",
                <div>
                    "Ignored a drum instructor who patiently tried to teach me "
                    inline_popover(
                        id: "rudiments-cite",
                        label: "rudiments",
                        <span class="inline-popover-preview">
                            "Standard sticking patterns — the vocabulary drum teachers start with."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://pas.org/rudiments/",
                            label: "Percussive Arts Society →"
                        )
                    )
                    " for several months"
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2007-2010",
                <div>
                    "Played "
                    inline_popover(
                        id: "rock-band-cite",
                        label: "Rock Band",
                        <span class="inline-popover-preview">
                            "Rhythm game series with plastic instrument controllers."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://www.youtube.com/watch?v=PFoizasD3Hc",
                            label: "What it looked like →"
                        )
                    )
                    " almost every day"
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2010",
                <div>
                    "Parents got me a "
                    inline_popover(
                        id: "dtxplorer-cite",
                        label: "Yamaha DTXplorer",
                        <span class="inline-popover-preview">
                            "Yamaha's entry-level electronic kit and one of the reasons I actually don't \
                         recommend getting a cheap electric kit for kids if they haven't decided whether \
                         they like playing the drums yet (they will learn to dislike playing the drums \
                         because many practice sessions start with a debugging session)."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://usa.yamaha.com/products/musical_instruments/drums/el_drums/dtx/index.html",
                            label: "Yamaha DTX →"
                        )
                    )
                    " which I practiced with sometimes"
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2011",
                <div>"Parents got me a used acoustic kit which I still have!"</div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2016",
                <div>"Started playing with some folks in Sunnyside"</div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2016",
                <div>
                    "Bought myself an "
                    inline_popover(
                        id: "dm10x-cite",
                        label: "Alesis DM10X",
                        <span class="inline-popover-preview">
                            "A discontinued six-piece electronic drum kit with large pads and the DM10 module."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://www.alesis.com/products/view/dm10-x-kit.html",
                            label: "Alesis →"
                        )
                    )
                </div>
            )
            rail_section(
                class: "mt-4",
                stamp: "2017",
                <div>"Sold my Alesis DM10X to Guitar Center"</div>
            )

            rail_section(
                class: "mt-4",
                stamp: "2018-2019",
                <div>
                    "Got weirdly into custom electronic drums and converted my acoustic
                 kit to electronic using "
                    inline_popover(
                        id: "ufo-drums-cite",
                        label: "UFO Drums heads and sensors",
                        <span class="inline-popover-preview">
                            "Mesh heads and trigger assemblies for converting acoustic drums to electronic."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://www.ufodrums.com/",
                            label: "ufodrums.com →"
                        )
                    )
                    ", "
                    inline_popover(
                        id: "jobeky-cite",
                        label: "Jobeky cymbals",
                        <span class="inline-popover-preview">
                            "Custom electronic drums, cymbals, and conversion hardware."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://jobekydrums.co.uk/",
                            label: "jobekydrums.co.uk →"
                        )
                    )
                    ", and a hodgepodge of modules including a "
                    inline_popover(
                        id: "megadrum-cite",
                        label: "MegaDrum module",
                        <span class="inline-popover-preview">
                            "A DIY-friendly MIDI drum trigger module."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://www.megadrum.uk/",
                            label: "megadrum.uk →"
                        )
                    )
                    ". Started playing with some embedded software to analyze "
                    inline_popover(
                        id: "piezo-sensors-cite",
                        label: "piezo sensors",
                        <span class="inline-popover-preview">
                            "Vibration-sensitive elements commonly used to detect drum hits and send trigger signals."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://www.daddario.com/blogs/percussion/everything-you-need-to-know-about-drum-triggers-sensors",
                            label: "how drum triggers work →"
                        )
                    )
                    ". Also became fascinated by "
                    inline_popover(
                        id: "sunhouse-cite",
                        label: "Sunhouse",
                        <span class="inline-popover-preview">
                            "A music-technology company making tools for expressive electronic drumming."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://sunhou.se/about",
                            label: "sunhou.se →"
                        )
                    )
                    ", the company behind "
                    inline_popover(
                        id: "sensory-percussion-cite",
                        label: "Sensory Percussion",
                        <span class="inline-popover-preview">
                            "A system that turns an acoustic drum into a controller for digital music."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://sunhou.se/",
                            label: "Sensory Percussion →"
                        )
                    )
                    ".
                 Nothing really has come out of any of this yet."
                </div>
            )

            rail_section(
                class: "mt-4",
                stamp: "2020",
                <div>"Bought myself an EFNote 5 (excellent decision)."</div>
            )

            rail_section(
                class: "mt-4",
                stamp: "2021",
                <div>
                    "Met a few folks I still play with (Justin and Sanket, mostly)."
                </div>
            )

            rail_section(
                class: "mt-4",
                stamp: "What I play",
                <ul>
                    <li>
                        inline_popover(
                            id: "efnote-cite",
                            label: "EFNote 5",
                            <span class="inline-popover-preview">
                                "Electronic drum kit that people look at and will be like \"what are you \
                             talking about, that's an acoustic kit\""
                            </span>
                            ext_link(
                                class: "quiet-link",
                                href: "https://www.efnotepro.com/products/efnote5",
                                label: "efnotepro.com →"
                            )
                        )
                    </li>
                    <li>
                        inline_popover(
                            id: "strike-multipad-cite",
                            label: "Alesis Strike MultiPad",
                            <span class="inline-popover-preview">
                                "Sample pad with built-in sounds"
                            </span>
                            ext_link(
                                class: "quiet-link",
                                href: "https://www.alesis.com/strike-multipad.html",
                                label: "alesis.com →"
                            )
                        )
                    </li>
                    <li>
                        inline_popover(
                            id: "club-jam-cite",
                            label: "Tama Club-Jam Pancake kit",
                            <span class="inline-popover-preview">
                                "Compact acoustic kit — 18\" bass, small toms, still my main acoustic set."
                            </span>
                            ext_link(
                                class: "quiet-link",
                                href: "https://www.tama.com/usa/products/drums/acoustic/club-jam/",
                                label: "tama.com →"
                            )
                        )
                    </li>
                </ul>
            )
            back_link(href: "/interests", label: "all interests")
        )
    }
}

#[route(GET "/interests/drums")]
async fn legacy_drums() -> Result {
    Err(redirect_permanent("/drums").into())
}
