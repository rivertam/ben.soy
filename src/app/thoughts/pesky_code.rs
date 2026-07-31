use topcoat::{Result, router::page, view::view};

use crate::components::shell;

crate::register_post!(
    note,
    slug: "pesky-code",
    title: "Pesky code",
    date: "2025-08-14",
    teaser: "On what AI frees me up to truly love.",
    source: "originally a LinkedIn post",
    body: "I'm so glad AI can handle all that pesky code for me so I can focus on what I \
           truly love: navigating endless chains of SSO sign-ins followed by dashboards to \
           manage settings and secrets in different environments ❤️",
    tags: &["ai"],
);

#[page("/thoughts/pesky-code")]
async fn pesky_code() -> Result {
    let crate::content::posts::PostKind::Note { body, source } = POST.kind else {
        unreachable!("pesky code is registered as a note");
    };
    view! {
        shell(
            title: POST.title,
            active: "",
            <article class="rail-row mt-16 sm:mt-24">
                <p class="rail-stamp">(POST.date)</p>
                <div class="min-w-0">
                    <h1 class="font-display text-4xl font-bold tracking-tight">
                        (POST.title)
                    </h1>
                    <p class="mt-8 max-w-prose text-xl leading-relaxed">
                        (body)
                    </p>
                    <p class="mt-8 font-meta text-xs text-muted">
                        (source)
                        ", August 2025"
                    </p>
                </div>
            </article>
        )
    }
}
