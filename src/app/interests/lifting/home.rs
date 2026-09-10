//! `/fitness` — the unified fitness landing page.

use super::*;

const LOG_LAUNCHER_JS: Asset = asset!("./log-launcher.js");

#[page("/fitness")]
async fn lifting(cx: &Cx) -> Result {
    let raw = match parse_query_params::<Vec<(String, String)>>(cx) {
        Ok(raw) => raw,
        Err(_) => return Err(redirect(FITNESS_PATH).into()),
    };
    let Some(filters) = Filters::normalize(raw) else {
        return Err(redirect(FITNESS_PATH).into());
    };
    let canonical = filters.query();
    if uri(cx).query().is_some_and(|query| query != canonical) {
        return Err(redirect(filters.url(false)).into());
    }

    let meta = interest("fitness");
    let can_log = viewer(cx).is_some_and(|current| is_admin(&current.email));
    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            page: meta.title,
            active: "",
            runtime: true,
            fitness_pwa: true,
            log::fitness_content(filters: &filters)
            if can_log {
                log_dialogs()
            }
            back_link(href: "/", label: "~")
            <script
                type="module"
                src=(crate::app::interests::running::PWA_JS)
                data-fitness-draft-resume=(can_log)
            ></script>
        )
    }
}

#[route(GET "/lifting")]
async fn legacy_lifting(cx: &Cx) -> Result {
    let target = if uri(cx).query().is_some() {
        with_raw_query(cx, LOG_PATH)
    } else {
        FITNESS_PATH.to_string()
    };
    Err(redirect_permanent(&target).into())
}

#[route(GET "/interests/lifting")]
async fn legacy_interest_lifting() -> Result {
    Err(redirect_permanent(FITNESS_PATH).into())
}

/// The same fitness archive in the phone pane, with its own default filters.
/// Home's log query belongs to the main timeline and must not filter this pane.
#[component]
pub(crate) async fn fitness_home_content() -> Result {
    let filters = Filters::default();
    view! { log::fitness_content(filters: &filters) }
}

/// One owner action opens a small training-ledger menu. Every item remains a
/// real anchor: Lift enters the dedicated logger, while the smaller forms are
/// progressively enhanced by the shared native-dialog driver.
#[component]
pub(crate) async fn log_launcher() -> Result {
    view! {
        <details
            class="group relative mt-1 flex-none"
            name="fitness-log-launcher"
            data-fitness-log-launcher=""
        >
            <summary
                class="flex min-h-11 cursor-pointer list-none items-center gap-2 rounded-sm \
                       border border-oxide bg-oxide px-3.5 py-2 font-meta text-xs \
                       font-semibold text-card hover:bg-oxide-hot hover:text-white \
                       focus-visible:outline-solid focus-visible:outline-2 \
                       focus-visible:outline-oxide focus-visible:outline-offset-2 \
                       [&::-webkit-details-marker]:hidden"
            >
                <span>"log"</span>
                <span
                    class="text-sm leading-none transition-transform group-open:rotate-45"
                    aria-hidden="true"
                >"+"</span>
            </summary>
            <nav
                class="absolute right-0 top-[calc(100%+0.45rem)] z-30 w-56 overflow-hidden \
                       rounded-sm border border-hairline bg-page shadow-xl"
                aria-label="Choose what to log"
            >
                <a
                    href="/fitness/entry"
                    class="group/item flex min-h-11 items-center gap-3 border-b border-hairline \
                           px-3 py-2.5 text-ink no-underline hover:bg-oxide/8 \
                           focus-visible:bg-oxide/8 focus-visible:outline-solid \
                           focus-visible:outline-2 focus-visible:outline-oxide \
                           focus-visible:outline-offset-[-2px]"
                >
                    <span
                        class="grid size-7 flex-none place-items-center rounded-full border \
                               border-oxide font-meta text-[0.65rem] font-semibold text-oxide"
                        aria-hidden="true"
                    >"L"</span>
                    <span class="min-w-0">
                        <span class="block font-display text-base font-semibold">"Lift"</span>
                        <span class="block font-meta text-[0.65rem] leading-tight text-muted">
                            "sets + PR routes"
                        </span>
                    </span>
                </a>
                <a
                    href="#fitness-run-dialog"
                    data-modal-open="fitness-run-dialog"
                    class="group/item flex min-h-11 items-center gap-3 border-b border-hairline \
                           px-3 py-2.5 text-ink no-underline hover:bg-patina/8 \
                           focus-visible:bg-patina/8 focus-visible:outline-solid \
                           focus-visible:outline-2 focus-visible:outline-patina \
                           focus-visible:outline-offset-[-2px]"
                >
                    <span
                        class="grid size-7 flex-none place-items-center rounded-full border \
                               border-patina font-meta text-[0.65rem] font-semibold text-patina"
                        aria-hidden="true"
                    >"R"</span>
                    <span class="min-w-0">
                        <span class="block font-display text-base font-semibold">"Run"</span>
                        <span class="block font-meta text-[0.65rem] leading-tight text-muted">
                            "distance + time"
                        </span>
                    </span>
                </a>
                <a
                    href="#fitness-interruption-dialog"
                    data-modal-open="fitness-interruption-dialog"
                    class="group/item flex min-h-11 items-center gap-3 border-b border-hairline \
                           px-3 py-2.5 text-ink no-underline hover:bg-brass/8 \
                           focus-visible:bg-brass/8 \
                           focus-visible:outline-solid focus-visible:outline-2 \
                           focus-visible:outline-brass focus-visible:outline-offset-[-2px]"
                >
                    <span
                        class="grid size-7 flex-none place-items-center rounded-full border \
                               border-brass font-meta text-[0.65rem] font-semibold text-brass"
                        aria-hidden="true"
                    >"—"</span>
                    <span class="min-w-0">
                        <span class="block font-display text-base font-semibold">
                            "Interruption"
                        </span>
                        <span class="block font-meta text-[0.65rem] leading-tight text-muted">
                            "sickness, travel, rest"
                        </span>
                    </span>
                </a>
                <a
                    href="#fitness-lift-dialog"
                    data-modal-open="fitness-lift-dialog"
                    class="group/item flex min-h-11 items-center gap-3 px-3 py-2.5 text-ink \
                           no-underline hover:bg-steel/8 focus-visible:bg-steel/8 \
                           focus-visible:outline-solid focus-visible:outline-2 \
                           focus-visible:outline-steel focus-visible:outline-offset-[-2px]"
                >
                    <span
                        class="grid size-7 flex-none place-items-center rounded-full border \
                               border-steel font-meta text-[0.65rem] font-semibold text-steel"
                        aria-hidden="true"
                    >"I"</span>
                    <span class="min-w-0">
                        <span class="block font-display text-base font-semibold">"Import"</span>
                        <span class="block font-meta text-[0.65rem] leading-tight text-muted">
                            "Lyfta text fallback"
                        </span>
                    </span>
                </a>
            </nav>
        </details>
    }
}

#[component]
pub(crate) async fn log_dialogs() -> Result {
    view! {
        workout_upload_dialog()
        crate::app::interests::running::manual_run_dialog()
        interruptions::create_dialog()

        // Closed dialogs are display:none without the driver. Make the three
        // real forms ordinary in-flow sections when scripting is disabled, so
        // each launcher's fragment link remains useful.
        <noscript>
            <style>
                "#fitness-lift-dialog,#fitness-run-dialog,#fitness-interruption-dialog{display:block;position:static;width:100%;max-height:none;margin:1rem 0 0;overflow:visible;box-shadow:none}#fitness-lift-dialog .modal-panel,#fitness-run-dialog .modal-panel,#fitness-interruption-dialog .modal-panel{max-height:none}#fitness-lift-dialog .modal-label,#fitness-run-dialog .modal-label,#fitness-interruption-dialog .modal-label,#fitness-lift-dialog .modal-close,#fitness-run-dialog .modal-close,#fitness-interruption-dialog .modal-close{display:none}"
            </style>
        </noscript>
        <script type="module" src=(LOG_LAUNCHER_JS)></script>
    }
}
