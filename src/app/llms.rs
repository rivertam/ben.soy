//! LLMs disclosure — how this site uses LLMs. Em dashes in the copy link here
//! (including on this page) via the response layer in `emdash_layer`.

use topcoat::{Result, router::page, view::view};

use crate::components::{ext_link, inline_popover, page_head, rail_prose, shell};

#[page("/llms")]
async fn llms() -> Result {
    view! {
        shell(
            page: "LLMs",
            active: "",
            page_head(
                stamp: "llms",
                title: "LLMs",
                lede: "How I use LLMs on this site — a short disclosure."
            )
            rail_prose(
                stamp: "Thoughts on LLMs",
                <p>
                    "
                LLMs are a really, really cool technology. I use them every day, often a lot. They have a lot of issues.
                LLMs are pretty bad for the environment. I tend to think their issues are over-hyped, but they also
                can't be denied. LLMs have almost certainly stolen a lot of content. Some of it is philosophically
                debatable, and some of it isn't (unless you wanna get "
                    <em>"real"</em>
                    " skeptical with it).
            "
                </p>
                <p>
                    "
                Nevertheless, LLMs are a really, really cool technology. They speed up the generation of code and
                can do simple maintenance tasks that relate to text very well. They're also excellent at doing
                deep (open internet only) research very quickly. They are not incredibly reliable for both
                alignment problem reasons and general quality issues. Their work needs to be checked in a similar
                way as a human's would: if it's a really high risk task and you have plenty of time, you're obviously
                going to want to check it over, but if it's a lower risk task or you simply can't achieve what
                you want to do without delegating, you have to delegate. Sometimes that means failure, as in the
                case of delegating to a person.
            "
                </p>
            )

            rail_prose(
                stamp: "Actual content",
                <p>
                    "
                My goal on this site is usually to communicate ideas. I tend to think that LLMs are doing a
                pretty bad job at that. The world will be a much different place when LLMs can write the way
                that I'm writing even here, in my opinion.
            "
                </p>
                <p>
                    "
                My goal on this site is also to be correct. I use LLMs to check my facts. I use LLMs to back up
                my facts. A lot of the facts on this site are formatted by LLMs. Why? Well, they like to write
                things like \"CO₂e\" properly (@clod pls fix). In many cases, they're giving me a research report
                on something that I've already done research on, and they're returning it exactly as I'd want
                it to be presented. But whether I write something with LLMs or not is basically aligned with my
                philosophy on them for this kind of task: I check over their work, and I don't do any sort of
                gratuitous unnecessary rewriting if I like the way it comes out.
            "
                </p>

                <p>
                    "
                There may be some copy that's left on this site which I didn't mean to leave as a placeholder
                but there it is. I'm still getting the basic shape of the site's ontology down, so some of this
                is the LLM trying to pretend to be me, but hopefully by around 2026-07-30 that's no longer the
                case.
            "
                </p>

                <p>
                    "
                I've also done stuff like \"Go on my YouTube account and find videos to add here\" or
                \"organize these photos and put them on Felix's page\". 
            "
                </p>
            )

            rail_prose(
                stamp: "Vibe Coding",
                <p>
                    "
                Unlike almost every web framework I've used before, this is the first time I've found
                 a web framework that I both really like and wouldn't know how to write manually before
                 I made a website successfully with it ("
                    inline_popover(
                        id: "topcoat-cite",
                        label: "topcoat",
                        <span class="inline-popover-preview">
                            "A Rust server-side web framework from the tokio project. This site runs \
                         on 0.1.3."
                        </span>
                        ext_link(
                            class: "quiet-link",
                            href: "https://github.com/tokio-rs/topcoat",
                            label: "github.com/tokio-rs/topcoat →"
                        )
                    )
                    "). I read a lot of the code, and I edit the content
                 in neovim, but almost all of the actual code edits are vibe coded, and the whole scaffolding and
                 deployment setup was 100% vibe coded.
            "
                </p>
                <p>
                    "
                I have been alternating between Anthropic's Fable 5, OpenAI's Luna, Terra, and Sol 5.6,
                and Cursor models (Grok 4.5 and Composer 2.5). I really like how the code's been coming out.
            "
                </p>
            )
        )
    }
}
