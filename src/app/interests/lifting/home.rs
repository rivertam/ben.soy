//! `/lifting` — the lifting archive landing page.

use super::*;

#[page("/lifting")]
async fn lifting(cx: &Cx) -> Result {
    // Filtered archive links remain useful; the archive itself now lives at
    // `/lifting/log`.
    if let Some(query) = uri(cx).query() {
        return Err(redirect(&format!("{LOG_PATH}?{query}")).into());
    }

    let meta = interest("lifting");
    let can_upload = viewer(cx).is_some_and(|current| is_admin(&current.email));
    let (calendar, latest, focus, interruptions) =
        fitness::load_home(app_context::<FitnessStore>(cx)).await;
    if let Err(error) = &calendar {
        eprintln!("fitness calendar fetch failed: {error}");
    }
    if let Err(error) = &latest {
        eprintln!("fitness latest workout fetch failed: {error}");
    }
    if let Err(error) = &focus {
        eprintln!("fitness training focus failed: {error}");
    }
    if let Err(error) = &interruptions {
        eprintln!("fitness interruptions fetch failed: {error}");
    }

    let calendar_days = calendar.ok().map(|calendar| calendar.days);
    let interruption_rows = interruptions.unwrap_or_default();
    let open_interruptions = interruptions::open_rows(&interruption_rows);
    let focus_summary = focus
        .as_ref()
        .ok()
        .filter(|summary| !summary.muscles.is_empty());
    let latest_error = latest.as_ref().err();
    let latest_workout = latest
        .as_ref()
        .ok()
        .and_then(|detail| detail.workout.as_ref());
    let next_lift_url = latest
        .as_ref()
        .ok()
        .and_then(|detail| detail.older_workout_path.as_deref())
        .map(workout_url);

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        shell(
            title: meta.title,
            active: "",
            runtime: true,
            <header class="rail-row mt-16">
                <p class="rail-stamp rail-stamp-label">(meta.slug)</p>
                <div class="flex min-w-0 items-start justify-between gap-4">
                    <h1 class="font-display text-4xl font-bold tracking-tight">
                        (meta.title)
                    </h1>
                    if can_upload {
                        <div class="flex flex-none flex-wrap items-start justify-end gap-2">
                            interruptions::create_dialog()
                            workout_upload_dialog()
                        </div>
                    }
                </div>
            </header>
            <div
                class=(class!(
                    "relative",
                    "min-[90rem]:min-h-[40rem]" if focus_summary.is_some(),
                ))
            >
                if let Some(focus) = focus_summary {
                    <details class="group mt-8 min-[90rem]:hidden">
                        <summary
                            class="flex min-h-11 w-full cursor-pointer list-none items-center \
                                 justify-between gap-4 rounded-[0.2rem] border border-oxide px-4 \
                                 py-2.5 font-meta text-xs text-oxide \
                                 after:text-base after:leading-none after:content-['+'] \
                                 group-open:after:content-['−'] \
                                 group-open:bg-oxide group-open:text-card \
                                 hover:bg-oxide hover:text-card \
                                 focus-visible:outline-solid focus-visible:outline-2 \
                                 focus-visible:outline-oxide focus-visible:outline-offset-2 \
                                 [&::-webkit-details-marker]:hidden"
                        >
                            <span class="group-open:hidden">
                                "show muscle load + next focus"
                            </span>
                            <span class="hidden group-open:inline">
                                "hide muscle load + next focus"
                            </span>
                        </summary>
                        <div class="mt-3 rounded-[0.2rem] border border-hairline bg-card p-4">
                            training_focus::panel(
                                focus: focus,
                                heading_id: "training-focus-mobile"
                            )
                        </div>
                    </details>
                    <aside
                        class="hidden border-t border-hairline pt-4 \
                             min-[90rem]:absolute min-[90rem]:left-full min-[90rem]:top-10 \
                             min-[90rem]:ml-8 min-[90rem]:block min-[90rem]:w-[14.5rem]"
                        aria-label="Muscle load and next focus"
                    >
                        training_focus::panel(
                            focus: focus,
                            heading_id: "training-focus-desktop"
                        )
                    </aside>
                }

                rail_section(
                    class: "mt-10",
                    stamp: "volume",
                    <header id="volume">
                        if let Some(days) = calendar_days {
                            heatmap::calendar_heatmap(
                                days: days,
                                interruptions: interruption_rows.clone()
                            )
                        } else {
                            <section class="p-4 bg-card border border-hairline">
                                <p class=(EMPTY_COPY)>
                                    "Daily volume is unavailable right now."
                                </p>
                            </section>
                        }
                    </header>
                )

                if !open_interruptions.is_empty() {
                    rail_section(
                        class: "mt-12",
                        stamp: "notes",
                        interruptions::open_panel(
                            rows: open_interruptions.as_slice(),
                            can_edit: can_upload
                        )
                    )
                }

                rail_section(
                    class: "mt-12",
                    stamp: "sets",
                    <header class="flex items-end justify-between gap-4" id="set-log">
                        <div>
                            <h2 class="font-display text-2xl font-semibold">
                                "Most recent lift"
                            </h2>
                        </div>
                        <a class=(class!(LIFT_LINK, "flex-none")) href=(LOG_PATH)>
                            "search full log →"
                        </a>
                        if let Some(href) = &next_lift_url {
                            <a class=(LIFT_LINK) href=(href.as_str())>
                                "see next lift →"
                            </a>
                        }
                    </header>
                )

                <section class=(LIST) aria-label="Most recent workout">
                    if latest_error.is_some() {
                        <div class=(EMPTY_ERROR_CARD)>
                            <p class=(EMPTY_TITLE)>
                                "The latest lift did not load."
                            </p>
                            <p class=(EMPTY_COPY)>
                                "Try the workout archive again in a moment."
                            </p>
                            <a class=(EMPTY_RESET) href="/lifting#set-log">
                                "retry"
                            </a>
                        </div>
                    } else if let Some(workout) = latest_workout {
                        workout_sheet(workout: workout, permalink: true)
                    } else {
                        <div class=(EMPTY_CARD)>
                            <p class=(EMPTY_TITLE)>"No lifts yet."</p>
                            <p class=(EMPTY_COPY)>
                                "The workout archive will appear here after its first import."
                            </p>
                        </div>
                    }
                </section>
            </div>
            back_link(href: "/", label: "~")
        )
    }
}
