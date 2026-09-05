//! Owner-only, mobile-first native workout entry.
//!
//! The mutable workout lives only in the browser until Finish. Publication is
//! one create-only archive transaction through the same manual-workout path as
//! Lyfta imports; no partial sets or draft rows enter SurrealDB.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{SystemTime, UNIX_EPOCH},
};

use benjisponge::data::Data;
use fitness_entry_core::{
    ExerciseGuide, FinalizedWorkout, GuideConfig, GuideMark, LoadPreset, SetType,
};
use jiff::ToSpan;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        Body, HeaderMap, HeaderValue, StatusCode, error::redirect, header, page, request::headers,
        response::Response, route, to_bytes,
    },
    view::{component, view},
};

use crate::{
    app::{login::viewer, not_found::not_found_page},
    components::shell,
    content::access::is_admin,
    util::is_same_origin,
};

use super::{
    archive::{
        db::{self, ManualImportOutcome},
        eastern,
        filters::Filters,
        native_entry::build_native_entry,
        records::{CurrentBest, Kind},
        store::FitnessStore,
    },
    data as fitness,
    format::{format_integer, format_signed_scaled},
    muscle_taxonomy,
};

const ENTRY_PATH: &str = "/fitness/entry";
const PUBLISH_PATH: &str = "/fitness/entry/publish";
const LOGIN_REDIRECT: &str = "/login?next=%2Ffitness%2Fentry";
const BODY_LIMIT_BYTES: usize = 64 * 1024;
const NO_STORE: &str = "no-store";
const ENTRY_JS: Asset = asset!("./entry.js");
const RIR_OPTIONS: [(&str, &str, bool); 11] = [
    ("", "—", false),
    ("", "FAIL", true),
    ("1000", "0", false),
    ("950", "0.5", false),
    ("900", "1", false),
    ("850", "1.5", false),
    ("800", "2", false),
    ("750", "2.5", false),
    ("700", "3", false),
    ("650", "3.5", false),
    ("600", "4", false),
];
const SET_TYPE_OPTIONS: [(SetType, &str); 5] = [
    (SetType::Normal, "work"),
    (SetType::Warmup, "warm"),
    (SetType::Drop, "drop"),
    (SetType::PartialReps, "partial"),
    (SetType::NegativeReps, "negative"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct LoadHistorySet {
    weight_milli: Option<i64>,
    reps: Option<u64>,
    set_type: SetType,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ExerciseSession {
    sets: Vec<LoadHistorySet>,
}

#[derive(Default)]
struct ExerciseHistory {
    set_count: usize,
    workout_ids: HashSet<String>,
    last_date: String,
    recent_sessions: Vec<ExerciseSession>,
}

#[page("/fitness/entry")]
async fn workout_entry(cx: &Cx) -> Result {
    let Some(current) = viewer(cx) else {
        return Err(redirect(LOGIN_REDIRECT).into());
    };
    if !is_admin(&current.email) {
        return view! {
            ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
            not_found_page(requested: ENTRY_PATH)
        };
    }

    let guide = entry_guide(app_context::<FitnessStore>(cx)).await;
    if let Err(error) = &guide {
        eprintln!(
            "{}",
            serde_json::json!({
                "message": "native workout entry guide failed",
                "path": ENTRY_PATH,
                "error": error,
            })
        );
    }

    view! {
        ((header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE)))
        shell(
            page: "Workout entry",
            active: "",
            hide_nav: true,
            runtime: false,
            fitness_pwa: true,
            if let Ok(guide) = &guide {
                entry_surface(guide: guide)
                <script
                    type="module"
                    src=(crate::app::interests::running::PWA_JS)
                ></script>
                <script type="module" src=(ENTRY_JS)></script>
            } else {
                <section class="fitness-entry">
                    <div class="entry-shell">
                        <header class="entry-header">
                            <a class="entry-header__back" href="/fitness" aria-label="Back to Fitness">
                                "←"
                            </a>
                            <div class="entry-header__identity">
                                <p class="entry-header__title">"Workout entry"</p>
                            </div>
                        </header>
                        <div class="entry-scroll">
                            <main class="entry-main">
                                <section class="entry-exercise">
                                    <div class="entry-exercise__header">
                                        <div class="entry-exercise__heading">
                                            <p class="entry-prs__label">"archive unavailable"</p>
                                            <h1 class="entry-exercise__name">"The training ledger did not load."</h1>
                                            <p class="entry-exercise__meta">
                                                "No draft was changed. Return to Fitness and try again."
                                            </p>
                                            <a class="entry-button" href="/fitness">"Back to Fitness"</a>
                                        </div>
                                    </div>
                                </section>
                            </main>
                        </div>
                    </div>
                </section>
            }
        )
    }
}

#[component]
async fn entry_surface(guide: &GuideConfig) -> Result {
    let guide_json = serde_json::to_string(guide).expect("entry guide serializes");
    view! {
        <section
            class="fitness-entry"
            data-fitness-entry=""
            data-entry-protocol=(fitness_entry_core::PROTOCOL_VERSION)
            data-entry-guide=(guide_json)
        >
            <div class="entry-shell">
                <header class="entry-header">
                    <a class="entry-header__back" href="/fitness" aria-label="Back to Fitness">
                        "←"
                    </a>
                    <div class="entry-header__identity">
                        <p class="entry-header__title">"live workout"</p>
                        <time class="entry-timer" data-entry-timer="" datetime="PT0S">"0:00"</time>
                    </div>
                    <button
                        type="button"
                        class="entry-header__finish"
                        data-action="finish"
                        data-entry-finish=""
                        disabled=""
                    >"Finish"</button>
                </header>

                <div class="entry-scroll">
                    <div class="entry-strip">
                        <p
                            class="entry-status"
                            data-entry-status=""
                            data-state="saved"
                            aria-live="polite"
                        >"Local draft"</p>
                        <dl class="entry-coverage" aria-label="Workout coverage">
                            <div class="entry-coverage__item">
                                <dt class="entry-coverage__label">"exercises"</dt>
                                <dd class="entry-coverage__value" data-entry-exercise-count="">"0"</dd>
                            </div>
                            <div class="entry-coverage__item">
                                <dt class="entry-coverage__label">"sets"</dt>
                                <dd class="entry-coverage__value" data-entry-set-count="">"0"</dd>
                            </div>
                            <div class="entry-coverage__item">
                                <dt class="entry-coverage__label">"done"</dt>
                                <dd class="entry-coverage__value" data-entry-completed-count="">"0"</dd>
                            </div>
                            for _ in 0..4 {
                                <div
                                    class="entry-coverage__item"
                                    data-entry-coverage-item=""
                                    hidden=""
                                >
                                    <dt
                                        class="entry-coverage__label"
                                        data-entry-coverage-label=""
                                    ></dt>
                                    <dd
                                        class="entry-coverage__value"
                                        data-entry-coverage-value=""
                                    ></dd>
                                </div>
                            }
                        </dl>
                    </div>

                    <main class="entry-main">
                        <div class="entry-workout">
                            <div data-entry-exercises=""></div>
                        </div>

                        <aside class="entry-side">
                            workout_queue()
                            <section
                                class="entry-fork"
                                data-entry-fork=""
                                aria-label="Next exercise"
                                hidden=""
                            >
                                <div class="entry-fork__lanes">
                                    suggestion_lane(
                                        lane: "deepen",
                                        label: "deepen",
                                        choice: "More of this",
                                        reason: "Same training thread."
                                    )
                                    suggestion_lane(
                                        lane: "expand",
                                        label: "expand",
                                        choice: "Add variety",
                                        reason: "A different region."
                                    )
                                </div>
                            </section>
                            quick_entry(guide: guide)
                        </aside>
                    </main>
                </div>
            </div>

            finish_review()
            exercise_template()
            set_template()
            pending_workout_template()
            failed_workout_template()
            saved_workout_template()
        </section>
    }
}

#[component]
async fn suggestion_lane(lane: &str, label: &str, choice: &str, reason: &str) -> Result {
    view! {
        <article class="entry-fork__lane" data-lane=(lane)>
            <p class="entry-fork__lane-label">(label)</p>
            <h3 class="entry-fork__choice" data-entry-suggestion-choice="">(choice)</h3>
            <p class="entry-fork__reason" data-entry-suggestion-reason="">(reason)</p>
            <p class="entry-exercise__meta" data-entry-suggestion-mark=""></p>
            <button
                type="button"
                class="entry-fork__add"
                data-action="use-suggestion"
                aria-label="Add suggested exercise"
            >"+"</button>
        </article>
    }
}

#[component]
async fn quick_entry(guide: &GuideConfig) -> Result {
    view! {
        <section class="entry-quick" aria-label="Add exercise">
            <div
                class="entry-quick__directions"
                data-entry-directions=""
                role="group"
                aria-label="Workout direction"
            >
                for (direction, label) in [
                    ("push", "Push"),
                    ("pull", "Pull"),
                    ("squat", "Squat"),
                    ("hinge", "Hinge"),
                    ("arms", "Arms"),
                    ("shoulders", "Shoulders"),
                ] {
                    <button
                        type="button"
                        class="entry-direction"
                        data-action="choose-direction"
                        data-direction=(direction)
                        aria-pressed="false"
                    >(label)</button>
                }
            </div>

            <div class="entry-starters" data-entry-starters="" hidden="">
                for _ in 0..2 {
                    <button
                        type="button"
                        class="entry-starter"
                        data-action="use-starter"
                        data-entry-starter=""
                        hidden=""
                    >
                        <span class="entry-starter__lane" data-entry-starter-lane=""></span>
                        <strong class="entry-starter__name" data-entry-starter-name=""></strong>
                        <span class="entry-starter__mark" data-entry-starter-mark=""></span>
                    </button>
                }
            </div>

            <label class="entry-quick__search">
                <span class="sr-only">"Search exercises"</span>
                <input
                    type="search"
                    inputmode="search"
                    autocomplete="off"
                    placeholder="Search exercises"
                    aria-controls="entry-search-results"
                    aria-expanded="false"
                    data-entry-picker-search=""
                >
            </label>

            <p class="sr-only" data-entry-search-feedback="" aria-live="polite"></p>
            <p
                class="entry-quick__empty"
                data-entry-quick-empty=""
                aria-live="polite"
                hidden=""
            ></p>
            <div
                id="entry-search-results"
                class="entry-quick__results"
                data-entry-search-results=""
                hidden=""
            >
                for exercise in guide.exercises.iter() {
                    <button
                        type="button"
                        class="entry-picker-option"
                        data-action="add-exercise"
                        data-exercise-catalog=""
                        data-name=(exercise.name.as_str())
                        hidden=""
                    >
                        <span class="entry-picker-option__name">(exercise.name.as_str())</span>
                        <span class="entry-picker-option__reason">(exercise.picker_meta.as_str())</span>
                        if !exercise.picker_mark.is_empty() {
                            <span class="entry-picker-option__action">(exercise.picker_mark.as_str())</span>
                        }
                    </button>
                }
            </div>
        </section>
    }
}

#[component]
async fn finish_review() -> Result {
    view! {
        <dialog class="entry-sheet" data-entry-review="" aria-labelledby="entry-review-heading">
            <div class="entry-sheet__panel">
                <header class="entry-sheet__header">
                    <div>
                        <h2 id="entry-review-heading" class="entry-sheet__title">"Publish workout"</h2>
                    </div>
                    <button
                        type="button"
                        class="entry-sheet__close"
                        data-action="close-review"
                        aria-label="Close workout review"
                    >"×"</button>
                </header>
                <div class="entry-sheet__body">
                    <label class="entry-review-field">
                        <span class="entry-prs__label">"workout title"</span>
                        <input
                            type="text"
                            maxlength="240"
                            required=""
                            value="Workout"
                            data-entry-review-title=""
                        >
                    </label>
                    <label class="entry-review-field">
                        <span class="entry-prs__label">"notes · optional"</span>
                        <textarea
                            rows="4"
                            maxlength="10000"
                            data-entry-review-notes=""
                        ></textarea>
                    </label>
                    <p class="entry-exercise__meta" data-entry-review-omitted="" hidden=""></p>
                    <p class="entry-exercise__meta" data-entry-review-status="" aria-live="polite"></p>
                </div>
                <footer class="entry-sheet__actions">
                    <button type="button" class="entry-fork__more" data-action="discard">
                        "Discard draft"
                    </button>
                    <button
                        type="button"
                        class="entry-button entry-button--primary"
                        data-action="publish"
                    >
                        "Publish workout"
                    </button>
                </footer>
            </div>
        </dialog>
    }
}

#[component]
async fn workout_queue() -> Result {
    view! {
        <section class="entry-queue" data-entry-queue="" aria-labelledby="entry-queue-title" hidden="">
            <header class="entry-queue__header">
                <p class="entry-prs__label">"device queue"</p>
                <h2 id="entry-queue-title" class="entry-queue__title">"Workout delivery"</h2>
            </header>
            <div class="entry-queue__list" data-entry-queue-list=""></div>
        </section>
    }
}

#[component]
async fn pending_workout_template() -> Result {
    view! {
        <template data-entry-pending-template="">
            <article class="entry-receipt" data-entry-queue-row="" data-state="pending" tabindex="-1">
                <header class="entry-receipt__header">
                    <div>
                        <p class="entry-receipt__state">"pending"</p>
                        <h3 class="entry-receipt__title" data-entry-queue-title=""></h3>
                    </div>
                    <button type="button" class="entry-receipt__action" data-action="flush-queue">
                        "Retry"
                    </button>
                </header>
                <p class="entry-receipt__copy">"Saved on this device and waiting to publish."</p>
                <p class="entry-receipt__link" data-entry-predicted-wrap="">
                    <code data-entry-predicted-location=""></code>
                    <span>" · not live yet"</span>
                </p>
            </article>
        </template>
    }
}

#[component]
async fn failed_workout_template() -> Result {
    view! {
        <template data-entry-failed-template="">
            <article class="entry-receipt" data-entry-queue-row="" data-state="failed" tabindex="-1">
                <header class="entry-receipt__header">
                    <div>
                        <p class="entry-receipt__state">"needs attention"</p>
                        <h3 class="entry-receipt__title" data-entry-queue-title=""></h3>
                    </div>
                    <button type="button" class="entry-receipt__action" data-action="restore-failed">
                        "Restore draft"
                    </button>
                </header>
                <p class="entry-receipt__copy" data-entry-failure=""></p>
            </article>
        </template>
    }
}

#[component]
async fn saved_workout_template() -> Result {
    view! {
        <template data-entry-saved-template="">
            <article class="entry-receipt" data-entry-queue-row="" data-state="saved" tabindex="-1">
                <header class="entry-receipt__header">
                    <div>
                        <p class="entry-receipt__state">"Workout Receipt"</p>
                        <h3 class="entry-receipt__title" data-entry-queue-title=""></h3>
                    </div>
                    <button type="button" class="entry-receipt__dismiss" data-action="dismiss-receipt" aria-label="Dismiss Workout Receipt">
                        "×"
                    </button>
                </header>
                <textarea class="entry-receipt__share" data-entry-share-text="" readonly="" rows="8" aria-label="Canonical workout share text"></textarea>
                <div class="entry-receipt__actions">
                    <button type="button" class="entry-button entry-button--primary" data-action="copy-receipt">
                        "Copy"
                    </button>
                    <a class="entry-button" data-action="open-receipt">"Open"</a>
                </div>
                <p class="entry-receipt__copy" data-entry-copy-status="" aria-live="polite"></p>
            </article>
        </template>
    }
}

#[component]
async fn exercise_template() -> Result {
    view! {
        <template data-entry-exercise-template="">
            <article class="entry-exercise" data-entry-exercise="">
                <header class="entry-exercise__header">
                    <div class="entry-exercise__heading">
                        <h2 class="entry-exercise__name" data-entry-exercise-name=""></h2>
                    </div>
                    <button
                        type="button"
                        class="entry-exercise__menu"
                        data-action="remove-exercise"
                        data-exercise-action=""
                        aria-label="Remove exercise"
                    >"×"</button>
                </header>
                <section
                    class="entry-prs"
                    data-entry-prs=""
                    aria-label="Current personal records"
                    hidden=""
                >
                    <ul class="entry-prs__list">
                        for _ in 0..4 {
                            <li class="entry-pr" data-entry-mark="" hidden="">
                                <span class="entry-pr__kind" data-entry-mark-kind=""></span>
                                <strong class="entry-pr__value" data-entry-mark-value=""></strong>
                                <span class="entry-pr__detail" data-entry-mark-detail=""></span>
                            </li>
                        }
                    </ul>
                </section>
                <div class="entry-sets">
                    <div class="entry-set-head" aria-hidden="true">
                        <span>"set"</span>
                        <span>"lb"</span>
                        <span>"reps"</span>
                        <span>"RIR"</span>
                        <span>"done"</span>
                        <span></span>
                    </div>
                    <div data-entry-set-list=""></div>
                </div>
                <button
                    type="button"
                    class="entry-fork__add"
                    data-action="add-set"
                    data-exercise-action=""
                >"+ Add set"</button>
            </article>
        </template>
    }
}

#[component]
async fn set_template() -> Result {
    view! {
        <template data-entry-set-template="">
            <div class="entry-set" data-entry-set="" role="group">
                <label class="entry-set__ordinal" data-entry-set-ordinal="">
                    <span class="entry-set__number" data-entry-set-number="">"1"</span>
                    <span class="entry-set__type" data-entry-set-type="">"work"</span>
                    <select data-entry-field="setType" aria-label="Set type">
                        for (set_type, label) in SET_TYPE_OPTIONS {
                            <option value=(set_type.as_str())>(label)</option>
                        }
                    </select>
                </label>
                <label class="entry-set__field entry-set__field--weight">
                    <span class="sr-only" data-entry-weight-label="">"Weight in pounds; optional"</span>
                    <input
                        type="text"
                        inputmode="decimal"
                        autocomplete="off"
                        placeholder="—"
                        data-entry-field="weight"
                    >
                </label>
                <label class="entry-set__field entry-set__field--reps">
                    <span class="sr-only">"Repetitions"</span>
                    <input
                        type="text"
                        inputmode="numeric"
                        autocomplete="off"
                        placeholder="0"
                        data-entry-field="reps"
                    >
                </label>
                <button
                    type="button"
                    class="entry-set__rir"
                    data-action="show-rir"
                    data-set-action=""
                    data-entry-field="effort"
                    aria-label="Reps in reserve; optional"
                    aria-expanded="false"
                >
                    <span data-entry-rir-value="">"—"</span>
                </button>
                <button
                    type="button"
                    class="entry-set__done"
                    data-action="toggle-set"
                    data-set-action=""
                    aria-pressed="false"
                    aria-label="Mark set complete"
                ></button>
                <button
                    type="button"
                    class="entry-set__remove"
                    data-action="remove-set"
                    data-set-action=""
                    aria-label="Remove set"
                >"×"</button>
                <div
                    class="entry-load-dock"
                    data-entry-load-dock=""
                    role="group"
                    aria-label="Set shortcuts"
                    tabindex="-1"
                    hidden=""
                >
                    <div class="entry-load-presets" data-entry-load-presets="">
                        for _ in 0..3 {
                            <button
                                type="button"
                                class="entry-load-preset"
                                data-action="use-load-preset"
                                data-entry-load-preset=""
                                data-set-action=""
                                aria-pressed="false"
                                hidden=""
                            >
                                <span
                                    class="entry-load-preset__kind"
                                    data-entry-load-preset-kind=""
                                ></span>
                                <span
                                    class="entry-load-preset__value"
                                    data-entry-load-preset-value=""
                                ></span>
                            </button>
                        }
                    </div>
                    <div class="entry-load-steps">
                        for (delta, label) in [
                            ("-10", "−10"),
                            ("-5", "−5"),
                            ("5", "+5"),
                            ("10", "+10"),
                        ] {
                            <button
                                type="button"
                                class="entry-load-step"
                                data-action="adjust-weight"
                                data-weight-delta=(delta)
                                data-set-action=""
                            >(label)</button>
                        }
                    </div>
                    <div
                        class="entry-rir-picker"
                        data-entry-rir-picker=""
                        role="radiogroup"
                        aria-label="Reps in reserve"
                    >
                        <span class="entry-rir-picker__label" aria-hidden="true">"RIR"</span>
                        <div class="entry-rir-options">
                            for (effort_hundredths, label, failure) in RIR_OPTIONS {
                                <label class="entry-rir-option">
                                    <input
                                        type="radio"
                                        name="entry-rir"
                                        class="sr-only entry-rir-option__control"
                                        data-action="set-rir"
                                        data-entry-rir-option=""
                                        data-effort-hundredths=(effort_hundredths)
                                        data-failure=(failure)
                                        data-set-action=""
                                        aria-label=(label)
                                    />
                                    <span aria-hidden="true">(label)</span>
                                </label>
                            }
                        </div>
                    </div>
                    <span
                        class="sr-only"
                        data-entry-set-status=""
                        aria-live="polite"
                    ></span>
                </div>
            </div>
        </template>
    }
}

async fn entry_guide(store: &FitnessStore) -> std::result::Result<GuideConfig, String> {
    let snapshot = store.snapshot().await.map_err(|error| error.to_string())?;
    let today = eastern::eastern_date(jiff::Timestamp::now());
    let page = snapshot.sets_page(&Filters {
        page: 1,
        per_page: usize::MAX,
        ..Filters::default()
    });
    let mut history: HashMap<String, ExerciseHistory> = HashMap::new();
    let recent_cutoff = today
        .checked_add((-20).days())
        .map_err(|error| error.to_string())?;
    let mut recent_workouts = 0usize;

    for workout in &page.workouts {
        let date = workout.started_at_local.get(..10).unwrap_or("");
        if date
            .parse::<jiff::civil::Date>()
            .is_ok_and(|date| date >= recent_cutoff && date <= today)
        {
            recent_workouts += 1;
        }
        let mut seen = HashSet::new();
        let mut session_sets: HashMap<String, Vec<LoadHistorySet>> = HashMap::new();
        for set in &workout.sets {
            let row = history.entry(set.exercise_name.clone()).or_default();
            row.set_count += 1;
            if seen.insert(set.exercise_name.as_str()) {
                row.workout_ids.insert(workout.id.clone());
            }
            if date > row.last_date.as_str() {
                row.last_date = date.to_string();
            }
            session_sets
                .entry(set.exercise_name.clone())
                .or_default()
                .push(LoadHistorySet {
                    weight_milli: set.weight_milli,
                    reps: set.reps,
                    set_type: set
                        .set_type
                        .parse()
                        .expect("snapshot set type was validated"),
                });
        }
        for (name, sets) in session_sets {
            let row = history
                .get_mut(&name)
                .expect("session exercise was inserted into history");
            if row.recent_sessions.len() < 3 {
                row.recent_sessions.push(ExerciseSession { sets });
            }
        }
    }

    let focus = snapshot.training_focus(today);
    // Reuse training-focus's regularity and recovery gates rather than
    // treating every raw deficit as a prescription. The browser suppresses
    // this need again once the muscle enters the in-progress workout.
    let muscle_needs: BTreeMap<String, u32> = focus
        .recommendation
        .as_ref()
        .map(|recommendation| {
            (
                recommendation.muscle_id.to_string(),
                recommendation.deficit_scaled,
            )
        })
        .into_iter()
        .collect();

    let mut names: Vec<String> = history.keys().cloned().collect();
    names.sort_unstable_by_key(|name| name.to_ascii_lowercase());
    let mut exercises = Vec::with_capacity(names.len());
    for name in names {
        let row = history.get(&name).expect("name came from history map");
        let muscles: Vec<(String, u32)> = snapshot
            .exercise_weight_map()
            .get(&name)
            .into_iter()
            .flatten()
            .map(|(muscle, ratio)| ((*muscle).to_string(), *ratio))
            .collect();
        let tags = snapshot
            .exercise_tag_map()
            .get(&name)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let movements: Vec<String> = tags
            .iter()
            .filter(|(kind, _)| kind == "movement")
            .map(|(_, value)| value.clone())
            .collect();
        let coarse_muscles: Vec<String> = tags
            .iter()
            .filter(|(kind, _)| kind == "muscle")
            .map(|(_, value)| value.clone())
            .collect();
        let bodyweight = tags
            .iter()
            .any(|(kind, value)| kind == "equipment" && value == "bodyweight");
        let (high_fatigue, high_axial_load) = fatigue_profile(&name, &movements, tags);
        let marks: Vec<GuideMark> = snapshot
            .exercise_current_bests(&name)
            .iter()
            .filter_map(|best| guide_mark(best, bodyweight))
            .collect();
        let loads = load_presets(row, bodyweight);
        let muscle_summary = primary_muscle_summary(&muscles);
        let picker_meta = if muscle_summary.is_empty() {
            format!(
                "{} · last {}",
                workout_count_label(row.workout_ids.len()),
                row.last_date
            )
        } else {
            format!(
                "{} · {} · last {}",
                muscle_summary,
                workout_count_label(row.workout_ids.len()),
                row.last_date
            )
        };
        let picker_mark = marks
            .first()
            .map(|mark| format!("{} {}", mark.kind, mark.value))
            .unwrap_or_default();
        exercises.push(ExerciseGuide {
            name,
            bodyweight,
            high_fatigue,
            high_axial_load,
            last_date: row.last_date.clone(),
            set_count: row.set_count,
            workout_count: row.workout_ids.len(),
            muscles,
            movements,
            coarse_muscles,
            marks,
            loads,
            picker_meta,
            picker_mark,
        });
    }

    Ok(GuideConfig {
        version: snapshot.version,
        today: today.to_string(),
        // Twenty-one dates are exactly three weeks. Add one before division
        // to round the tenths representation to the nearest third.
        weekly_pace_tenths: (recent_workouts.saturating_mul(10) + 1) / 3,
        muscle_needs,
        exercises,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadRegime {
    Null,
    Zero,
    Positive,
    Negative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SessionLoad {
    session_index: usize,
    weight_milli: Option<i64>,
}

fn load_presets(history: &ExerciseHistory, bodyweight: bool) -> Vec<LoadPreset> {
    let representatives: Vec<SessionLoad> = history
        .recent_sessions
        .iter()
        .enumerate()
        .filter_map(|(session_index, session)| {
            session_work_load(session).map(|weight_milli| SessionLoad {
                session_index,
                weight_milli,
            })
        })
        .collect();
    let Some(newest) = representatives.first() else {
        return Vec::new();
    };
    let regime = load_regime(newest.weight_milli);
    if regime == LoadRegime::Null && !bodyweight {
        // A missing load only has a concrete meaning for a canonically tagged
        // bodyweight movement. Elsewhere it is unknown, not a zero-load cue.
        return Vec::new();
    }
    let matching: Vec<SessionLoad> = representatives
        .into_iter()
        .filter(|representative| load_regime(representative.weight_milli) == regime)
        .collect();
    let work_weight = if matching.len() < 3 {
        // One observation is itself; with two, favor the newer session rather
        // than inventing a midpoint that may not exist on the equipment.
        matching[0].weight_milli
    } else {
        let mut weights: Vec<Option<i64>> = matching
            .iter()
            .take(3)
            .map(|representative| representative.weight_milli)
            .collect();
        weights.sort_unstable();
        weights[1]
    };

    let explicit_warmups = matching.iter().find_map(|representative| {
        let warmups = valid_explicit_warmups(
            &history.recent_sessions[representative.session_index],
            work_weight,
            bodyweight,
        );
        (!warmups.is_empty()).then_some(warmups)
    });
    let warmups = explicit_warmups.unwrap_or_else(|| generated_warmups(work_weight, bodyweight));
    let mut presets: Vec<LoadPreset> = warmups
        .into_iter()
        .take(2)
        .map(|weight_milli| LoadPreset::new("warm", weight_milli, SetType::Warmup, bodyweight))
        .collect();
    presets.push(LoadPreset::new(
        "work",
        work_weight,
        SetType::Normal,
        bodyweight,
    ));
    presets
}

fn fatigue_profile(name: &str, movements: &[String], tags: &[(String, String)]) -> (bool, bool) {
    let name = name.to_ascii_lowercase();
    let has_movement = |value: &str| movements.iter().any(|movement| movement == value);
    let has_equipment = |value: &str| {
        tags.iter()
            .any(|(kind, candidate)| kind == "equipment" && candidate == value)
    };
    let externally_stabilized = has_equipment("machine")
        || has_equipment("cable")
        || has_equipment("smith-machine")
        || name.contains("machine")
        || name.contains("leg press")
        || name.contains("hack squat");
    let named_axial_hinge = name.contains("deadlift") || name.contains("good morning");
    let named_axial_squat = name.contains("barbell")
        || name.contains("zercher")
        || name.contains("front squat")
        || name.contains("back squat")
        || name == "full squat";
    let high_axial_load = !externally_stabilized
        && (named_axial_hinge || (has_movement("squat-type") && named_axial_squat));
    let free_loaded = has_equipment("barbell")
        || has_equipment("sandbag")
        || has_equipment("landmine")
        || name.contains("barbell");
    let major_compound = [
        "squat-type",
        "hinge",
        "horizontal-push",
        "horizontal-pull",
        "vertical-push",
        "vertical-pull",
    ]
    .iter()
    .any(|movement| has_movement(movement));
    (
        high_axial_load || (free_loaded && major_compound),
        high_axial_load,
    )
}

fn session_work_load(session: &ExerciseSession) -> Option<Option<i64>> {
    modal_load(
        session
            .sets
            .iter()
            .filter(|set| set.set_type == SetType::Normal && set.reps.is_some_and(|reps| reps >= 1))
            .map(|set| set.weight_milli),
    )
}

fn modal_load(weights: impl Iterator<Item = Option<i64>>) -> Option<Option<i64>> {
    let mut counts: BTreeMap<Option<i64>, usize> = BTreeMap::new();
    for weight in weights {
        *counts.entry(weight).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|(left_weight, left_count), (right_weight, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| left_weight.cmp(right_weight))
        })
        .map(|(weight, _)| weight)
}

fn load_regime(weight_milli: Option<i64>) -> LoadRegime {
    match weight_milli {
        None => LoadRegime::Null,
        Some(0) => LoadRegime::Zero,
        Some(weight) if weight > 0 => LoadRegime::Positive,
        Some(_) => LoadRegime::Negative,
    }
}

fn valid_explicit_warmups(
    session: &ExerciseSession,
    work_weight: Option<i64>,
    bodyweight: bool,
) -> Vec<Option<i64>> {
    let mut warmups: Vec<Option<i64>> = session
        .sets
        .iter()
        .filter(|set| set.set_type == SetType::Warmup && set.reps.is_some_and(|reps| reps >= 1))
        .map(|set| set.weight_milli)
        .filter(|weight| valid_warmup(*weight, work_weight, bodyweight))
        .collect();

    // Repeated warm-up sets at the same load need only one preset. Walk from
    // the work set backwards so the two closest distinct loads win, then put
    // them back into performed order.
    let mut seen = HashSet::new();
    warmups.reverse();
    warmups.retain(|weight| seen.insert(*weight));
    warmups.truncate(2);
    warmups.reverse();
    warmups
}

fn valid_warmup(warmup_weight: Option<i64>, work_weight: Option<i64>, bodyweight: bool) -> bool {
    match (warmup_weight, work_weight) {
        (Some(warmup), Some(work)) => warmup < work,
        (None, Some(work)) => bodyweight && 0 < work,
        (Some(warmup), None) => bodyweight && warmup < 0,
        (None, None) => false,
    }
}

fn generated_warmups(work_weight: Option<i64>, bodyweight: bool) -> Vec<Option<i64>> {
    let Some(work) = work_weight.filter(|work| *work > 0) else {
        return Vec::new();
    };
    if bodyweight {
        return vec![None];
    }

    let mut warmups = Vec::with_capacity(2);
    for (numerator, denominator) in [(1, 2), (3, 4)] {
        let Some(weight) = round_fraction_to_five_pounds(work, numerator, denominator) else {
            continue;
        };
        if weight > 0 && weight < work && !warmups.contains(&Some(weight)) {
            warmups.push(Some(weight));
        }
    }
    warmups
}

fn round_fraction_to_five_pounds(
    weight_milli: i64,
    numerator: i128,
    denominator: i128,
) -> Option<i64> {
    const FIVE_POUNDS_MILLI: i128 = 5_000;
    let divisor = denominator.checked_mul(FIVE_POUNDS_MILLI)?;
    let scaled = i128::from(weight_milli).checked_mul(numerator)?;
    let units = scaled.checked_add(divisor / 2)?.checked_div(divisor)?;
    i64::try_from(units.checked_mul(FIVE_POUNDS_MILLI)?).ok()
}

fn guide_mark(best: &CurrentBest, bodyweight: bool) -> Option<GuideMark> {
    let detail = prescription(best.weight_milli, best.reps, bodyweight);
    match best.kind {
        Kind::OneRm => {
            let weight = i128::from(best.weight_milli?);
            let reps = i128::from(best.reps?);
            let estimate_milli = (weight.saturating_mul(30 + reps) + 15) / 30;
            let estimate_milli = i64::try_from(estimate_milli).ok()?;
            Some(GuideMark {
                kind: "e1RM".to_string(),
                value: format!("{} lb", format_e1rm(estimate_milli)),
                detail,
            })
        }
        Kind::MaxWeight => Some(GuideMark {
            kind: "load".to_string(),
            value: format!("{} lb", format_signed_scaled(best.weight_milli?, 1_000)),
            detail,
        }),
        Kind::Volume => {
            let volume_milli =
                i128::from(best.weight_milli?).checked_mul(i128::from(best.reps?))?;
            let rounded = (volume_milli + 500) / 1_000;
            let rounded = u64::try_from(rounded).ok()?;
            Some(GuideMark {
                kind: "volume".to_string(),
                value: format!("{} lb·reps", format_integer(rounded)),
                detail,
            })
        }
        Kind::Reps => Some(GuideMark {
            kind: "reps".to_string(),
            value: format!("{} reps", format_integer(u64::try_from(best.reps?).ok()?)),
            detail,
        }),
    }
}

fn format_e1rm(weight_milli: i64) -> String {
    let rounded = if weight_milli >= 0 {
        weight_milli.saturating_add(50)
    } else {
        weight_milli.saturating_sub(50)
    } / 100
        * 100;
    format_signed_scaled(rounded, 1_000)
}

fn prescription(weight_milli: Option<i64>, reps: Option<i64>, bodyweight: bool) -> String {
    match (weight_milli, reps) {
        (Some(weight), Some(reps)) => format!(
            "{} lb × {}",
            format_signed_scaled(weight, 1_000),
            format_integer(u64::try_from(reps).unwrap_or(0))
        ),
        (None, Some(reps)) if bodyweight => format!(
            "bodyweight × {}",
            format_integer(u64::try_from(reps).unwrap_or(0))
        ),
        (None, Some(reps)) => format!("{} reps", format_integer(u64::try_from(reps).unwrap_or(0))),
        (Some(weight), None) => format!("{} lb", format_signed_scaled(weight, 1_000)),
        (None, None) => String::new(),
    }
}

fn primary_muscle_summary(muscles: &[(String, u32)]) -> String {
    let mut labels: Vec<&str> = muscles
        .iter()
        .filter(|(_, ratio)| *ratio >= 75)
        .filter_map(|(muscle, _)| muscle_taxonomy::muscle_label(muscle))
        .take(2)
        .collect();
    labels.dedup();
    labels.join(" + ")
}

fn workout_count_label(count: usize) -> String {
    format!(
        "{} {}",
        count,
        if count == 1 { "workout" } else { "workouts" }
    )
}

#[route(POST "/fitness/entry/publish")]
async fn publish_native_workout(cx: &Cx, body: Body) -> Result<Response> {
    let Some(current) = viewer(cx) else {
        return Ok(entry_error(
            StatusCode::UNAUTHORIZED,
            "Sign in again, then retry.",
        ));
    };
    if !is_admin(&current.email) {
        return Ok(entry_error(StatusCode::NOT_FOUND, "not found"));
    }
    if !is_same_origin(headers(cx)) {
        return Ok(entry_error(StatusCode::FORBIDDEN, "forbidden"));
    }
    if !is_json_content_type(headers(cx)) {
        return Ok(entry_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        ));
    }
    match declared_body_length(headers(cx)) {
        Ok(Some(length)) if length > BODY_LIMIT_BYTES => {
            return Ok(entry_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "workout is too large",
            ));
        }
        Ok(_) => {}
        Err(()) => return Ok(entry_error(StatusCode::BAD_REQUEST, "bad Content-Length")),
    }
    let bytes = match to_bytes(body, BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(entry_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "workout is too large",
            ));
        }
    };
    let input: FinalizedWorkout = match serde_json::from_slice(&bytes) {
        Ok(input) => input,
        Err(_) => return Ok(entry_error(StatusCode::BAD_REQUEST, "bad workout JSON")),
    };
    let store = app_context::<FitnessStore>(cx);
    let snapshot = match store.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log_publish_failure(error);
            return Ok(entry_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "The workout archive is temporarily unavailable.",
            ));
        }
    };
    let built = match build_native_entry(input, &snapshot) {
        Ok(built) => built,
        Err(error) => {
            return Ok(entry_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                &error.to_string(),
            ));
        }
    };
    let handle = match app_context::<Data>(cx).db().await {
        Ok(handle) => handle,
        Err(error) => {
            log_publish_failure(error);
            return Ok(entry_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "The workout archive is temporarily unavailable.",
            ));
        }
    };
    let imported_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let outcome = match db::create_manual_workout(&handle, &built.payload, imported_at).await {
        Ok(outcome) => outcome,
        Err(error) => {
            log_publish_failure(error);
            return Ok(entry_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "The workout could not be published right now.",
            ));
        }
    };
    let duplicate = match outcome {
        ManualImportOutcome::Added => false,
        ManualImportOutcome::Duplicate => true,
        ManualImportOutcome::Conflict => {
            return Ok(entry_error(
                StatusCode::CONFLICT,
                "A different workout already uses that start time.",
            ));
        }
    };
    if let Err(error) = store.rebuild().await {
        // The exact frozen payload is safely queued and the write is
        // idempotent. A retry can classify it as a duplicate after a fresh
        // snapshot; never mint an approximate receipt from stale state.
        log_publish_failure(error);
        return Ok(entry_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "The workout was stored but its receipt is not ready yet.",
        ));
    }
    let (detail, _) = match fitness::load_workout_by_path(store, &built.public_path).await {
        Ok(detail) => detail,
        Err(error) => {
            log_publish_failure(error);
            return Ok(entry_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "The workout was stored but its receipt is not ready yet.",
            ));
        }
    };
    let Some(workout) = detail.workout else {
        return Ok(entry_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "The workout was stored but its receipt is not ready yet.",
        ));
    };
    let share_text = super::canonical_share_text(cx, &workout);
    Ok(json_response(
        StatusCode::OK,
        publish_receipt(&built.public_path, duplicate, share_text),
    ))
}

fn publish_receipt(public_path: &str, duplicate: bool, share_text: String) -> serde_json::Value {
    serde_json::json!({
        "location": format!("/fitness/lift/{public_path}"),
        "duplicate": duplicate,
        "share_text": share_text,
    })
}

fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn declared_body_length(headers: &HeaderMap) -> std::result::Result<Option<usize>, ()> {
    let mut values = headers.get_all(header::CONTENT_LENGTH).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    value
        .to_str()
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .map(Some)
        .ok_or(())
}

fn entry_error(status: StatusCode, message: &str) -> Response {
    json_response(
        status,
        serde_json::json!({
            "error": message,
        }),
    )
}

fn json_response(status: StatusCode, value: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, NO_STORE)
        .header(header::REFERRER_POLICY, "no-referrer")
        .header("x-content-type-options", "nosniff")
        .body(Body::from(value.to_string()))
        .expect("entry JSON response uses static headers")
}

fn log_publish_failure(error: impl std::fmt::Display) {
    eprintln!(
        "{}",
        serde_json::json!({
            "message": "native workout publish failed",
            "path": PUBLISH_PATH,
            "error": error.to_string(),
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELF: &str = include_str!("entry.rs");
    const ENTRY_JS_SOURCE: &str = include_str!("entry.js");

    fn load_set(set_type: &str, weight_milli: Option<i64>, reps: Option<u64>) -> LoadHistorySet {
        LoadHistorySet {
            weight_milli,
            reps,
            set_type: set_type.parse().unwrap(),
        }
    }

    fn load_history(sessions: Vec<Vec<LoadHistorySet>>) -> ExerciseHistory {
        ExerciseHistory {
            recent_sessions: sessions
                .into_iter()
                .map(|sets| ExerciseSession { sets })
                .collect(),
            ..ExerciseHistory::default()
        }
    }

    fn preset(
        label: &'static str,
        weight_milli: Option<i64>,
        set_type: &'static str,
    ) -> LoadPreset {
        LoadPreset::new(label, weight_milli, set_type.parse().unwrap(), false)
    }

    fn bodyweight_preset(
        label: &'static str,
        weight_milli: Option<i64>,
        set_type: &'static str,
    ) -> LoadPreset {
        LoadPreset::new(label, weight_milli, set_type.parse().unwrap(), true)
    }

    #[test]
    fn entry_markup_keeps_the_header_fixed_and_search_last() {
        let header_class = ["class=\"entry-", "header\""].concat();
        let header_close = ["</head", "er>"].concat();
        let scroll_class = ["class=\"entry-", "scroll\""].concat();
        let header_opens: Vec<_> = SELF.match_indices(&header_class).collect();
        let scrolls: Vec<_> = SELF.match_indices(&scroll_class).collect();
        assert_eq!(header_opens.len(), 2, "success and error headers diverged");
        assert_eq!(scrolls.len(), header_opens.len());
        for ((header_open, _), (scroll, _)) in header_opens.iter().zip(&scrolls) {
            assert!(header_open < scroll);
            assert!(SELF[*header_open..*scroll].contains(&header_close));
        }

        let surface = SELF
            .split_once("async fn entry_surface")
            .unwrap()
            .1
            .split_once("async fn suggestion_lane")
            .unwrap()
            .0;
        let exercises = surface.find("data-entry-exercises").unwrap();
        let suggestions = surface.find("data-entry-fork").unwrap();
        let add_console = surface.find("quick_entry(guide: guide)").unwrap();
        assert!(exercises < suggestions && suggestions < add_console);

        let quick = SELF
            .split_once("async fn quick_entry")
            .unwrap()
            .1
            .split_once("async fn finish_review")
            .unwrap()
            .0;
        let directions = quick.find("data-entry-directions").unwrap();
        let starters = quick.find("data-entry-starters").unwrap();
        let search = quick.find("data-entry-picker-search").unwrap();
        let results = quick.find("data-entry-search-results").unwrap();
        assert!(directions < starters && starters < search && search < results);
    }

    #[test]
    fn entry_client_is_a_worker_and_dom_adapter() {
        for needle in [
            "new MessageChannel()",
            "request(\"bootstrap\"",
            "request(\"transition\"",
            "request(\"finalize\"",
            "request(\"restore\"",
            "request(\"dismiss\"",
            "navigator.clipboard.writeText",
            "template.content.firstElementChild.cloneNode(true)",
            "registration.installing || registration.waiting",
            "await registration.update()",
            "refreshPending = true",
            "if (!value || value.error) return;",
            "window.addEventListener(\"online\"",
            "window.addEventListener(\"pageshow\"",
            "document.addEventListener(\"visibilitychange\"",
            "localStorage.removeItem(snapshot.legacy_storage_key)",
            "requestAnimationFrame(scrollSearchIntoView)",
            "scroller.scrollTo({ top: Math.max(0, target)",
            "failure: control.dataset.failure === \"true\"",
        ] {
            assert!(ENTRY_JS_SOURCE.contains(needle), "entry.js lost {needle:?}");
        }
        for obsolete in [
            "fetch(",
            "localStorage.setItem",
            "scoreCandidate",
            "poundsToMilli",
        ] {
            assert!(
                !ENTRY_JS_SOURCE.contains(obsolete),
                "entry.js regained {obsolete:?}"
            );
        }
    }

    #[test]
    fn every_queue_state_clones_server_markup() {
        for state in ["pending", "failed", "saved"] {
            assert!(SELF.contains(&format!("data-entry-{state}-template")));
        }
        assert!(SELF.contains("data-entry-share-text"));
        assert!(SELF.contains("data-action=\"copy-receipt\""));
        assert!(SELF.contains("data-action=\"open-receipt\""));
    }

    #[test]
    fn fresh_and_duplicate_publications_return_the_same_canonical_receipt() {
        let path = "2026-09-03T10-00-00-04-00";
        let share = "Workout\nhttps://ben.soy/fitness/lift/2026-09-03T10-00-00-04-00";
        let fresh = publish_receipt(path, false, share.to_string());
        let duplicate = publish_receipt(path, true, share.to_string());
        assert_eq!(fresh["location"], duplicate["location"]);
        assert_eq!(fresh["share_text"], duplicate["share_text"]);
        assert_eq!(fresh["duplicate"], false);
        assert_eq!(duplicate["duplicate"], true);
    }

    #[test]
    fn recent_positive_history_uses_all_normal_efforts_in_session_modes_then_median() {
        let history = load_history(vec![
            vec![
                load_set("NORMAL_SET", Some(135_000), Some(8)),
                load_set("NORMAL_SET", Some(135_000), Some(8)),
                load_set("NORMAL_SET", Some(140_000), Some(6)),
                load_set("NORMAL_SET", Some(200_000), Some(2)),
                load_set("NORMAL_SET", Some(200_000), Some(2)),
                load_set("NORMAL_SET", Some(200_000), Some(2)),
            ],
            vec![load_set("NORMAL_SET", Some(150_000), Some(5))],
            vec![
                load_set("NORMAL_SET", Some(225_000), Some(0)),
                load_set("NORMAL_SET", Some(120_000), Some(10)),
            ],
        ]);

        assert_eq!(
            load_presets(&history, false),
            vec![
                preset("warm", Some(75_000), "WARMUP_SET"),
                preset("warm", Some(115_000), "WARMUP_SET"),
                preset("work", Some(150_000), "NORMAL_SET"),
            ]
        );

        let two_sessions = load_history(vec![
            vec![load_set("NORMAL_SET", Some(90_000), Some(8))],
            vec![load_set("NORMAL_SET", Some(150_000), Some(5))],
        ]);
        assert_eq!(
            load_presets(&two_sessions, false)
                .last()
                .unwrap()
                .weight_milli,
            Some(90_000)
        );
    }

    #[test]
    fn explicit_warmups_replace_generated_loads_and_keep_last_two() {
        let history = load_history(vec![vec![
            load_set("WARMUP_SET", Some(20_000), Some(10)),
            load_set("WARMUP_SET", Some(40_000), Some(8)),
            load_set("WARMUP_SET", Some(60_000), Some(5)),
            load_set("WARMUP_SET", Some(60_000), Some(5)),
            load_set("WARMUP_SET", Some(80_000), Some(0)),
            load_set("WARMUP_SET", Some(110_000), Some(2)),
            load_set("NORMAL_SET", Some(100_000), Some(8)),
        ]]);

        assert_eq!(
            load_presets(&history, false),
            vec![
                preset("warm", Some(40_000), "WARMUP_SET"),
                preset("warm", Some(60_000), "WARMUP_SET"),
                preset("work", Some(100_000), "NORMAL_SET"),
            ]
        );
    }

    #[test]
    fn bodyweight_history_distinguishes_bw_from_added_load() {
        let weighted = load_history(vec![vec![load_set("NORMAL_SET", Some(25_000), Some(8))]]);
        assert_eq!(
            load_presets(&weighted, true),
            vec![
                bodyweight_preset("warm", None, "WARMUP_SET"),
                bodyweight_preset("work", Some(25_000), "NORMAL_SET"),
            ]
        );

        let unweighted = load_history(vec![vec![load_set("NORMAL_SET", None, Some(12))]]);
        assert_eq!(
            load_presets(&unweighted, true),
            vec![bodyweight_preset("work", None, "NORMAL_SET")]
        );
    }

    #[test]
    fn null_and_zero_loads_remain_distinct() {
        let null_history = load_history(vec![
            vec![load_set("NORMAL_SET", None, Some(12))],
            vec![load_set("NORMAL_SET", Some(0), Some(12))],
            vec![load_set("NORMAL_SET", Some(0), Some(10))],
        ]);
        let zero_history = load_history(vec![
            vec![load_set("NORMAL_SET", Some(0), Some(12))],
            vec![load_set("NORMAL_SET", None, Some(12))],
            vec![load_set("NORMAL_SET", None, Some(10))],
        ]);
        let null_presets = load_presets(&null_history, false);
        let zero_presets = load_presets(&zero_history, false);

        assert!(null_presets.is_empty());
        assert_eq!(zero_presets, vec![preset("work", Some(0), "NORMAL_SET")]);
        assert_eq!(
            serde_json::to_value(load_presets(&null_history, true)).unwrap(),
            serde_json::json!([{
                "label": "work",
                "weight_milli": null,
                "set_type": "NORMAL_SET",
                "display": "BW",
                "spoken": "bodyweight",
            }])
        );
        assert_eq!(
            serde_json::to_value(&zero_presets).unwrap(),
            serde_json::json!([{
                "label": "work",
                "weight_milli": 0,
                "set_type": "NORMAL_SET",
                "display": "0 lb",
                "spoken": "0 pounds",
            }])
        );
    }

    #[test]
    fn negative_assistance_only_uses_easier_explicit_warmups() {
        let explicit = load_history(vec![vec![
            load_set("WARMUP_SET", Some(-70_000), Some(8)),
            load_set("WARMUP_SET", Some(-55_000), Some(6)),
            load_set("WARMUP_SET", Some(-35_000), Some(4)),
            load_set("NORMAL_SET", Some(-40_000), Some(8)),
        ]]);
        assert_eq!(
            load_presets(&explicit, true),
            vec![
                bodyweight_preset("warm", Some(-70_000), "WARMUP_SET"),
                bodyweight_preset("warm", Some(-55_000), "WARMUP_SET"),
                bodyweight_preset("work", Some(-40_000), "NORMAL_SET"),
            ]
        );

        let no_warmups = load_history(vec![vec![load_set("NORMAL_SET", Some(-40_000), Some(8))]]);
        assert_eq!(
            load_presets(&no_warmups, true),
            vec![bodyweight_preset("work", Some(-40_000), "NORMAL_SET")]
        );
    }

    #[test]
    fn benchmark_copy_shows_current_prescriptions() {
        let mark = guide_mark(
            &CurrentBest {
                kind: Kind::OneRm,
                set_id: "set:1".into(),
                weight_milli: Some(225_000),
                reps: Some(5),
            },
            false,
        )
        .unwrap();
        assert_eq!(mark.kind, "e1RM");
        assert_eq!(mark.value, "262.5 lb");
        assert_eq!(mark.detail, "225 lb × 5");

        let reps = guide_mark(
            &CurrentBest {
                kind: Kind::Reps,
                set_id: "set:2".into(),
                weight_milli: None,
                reps: Some(12),
            },
            true,
        )
        .unwrap();
        assert_eq!(reps.value, "12 reps");
        assert_eq!(reps.detail, "bodyweight × 12");

        let unweighted = guide_mark(
            &CurrentBest {
                kind: Kind::Reps,
                set_id: "set:3".into(),
                weight_milli: None,
                reps: Some(12),
            },
            false,
        )
        .unwrap();
        assert_eq!(unweighted.detail, "12 reps");

        assert_eq!(format_e1rm(116_667), "116.7");
        assert_eq!(format_e1rm(116_649), "116.6");
    }

    #[test]
    fn entry_options_cover_effort_axis_and_every_structural_set_type() {
        assert_eq!(
            RIR_OPTIONS,
            [
                ("", "—", false),
                ("", "FAIL", true),
                ("1000", "0", false),
                ("950", "0.5", false),
                ("900", "1", false),
                ("850", "1.5", false),
                ("800", "2", false),
                ("750", "2.5", false),
                ("700", "3", false),
                ("650", "3.5", false),
                ("600", "4", false),
            ]
        );
        let mut rendered: Vec<SetType> = SET_TYPE_OPTIONS
            .iter()
            .map(|(set_type, _)| *set_type)
            .collect();
        rendered.sort_unstable();
        let mut domain = SetType::ALL.to_vec();
        domain.sort_unstable();
        assert_eq!(rendered, domain);
    }

    #[test]
    fn fatigue_profiles_separate_axial_compounds_from_stabilized_variants() {
        let squat = vec!["squat-type".to_string()];
        let hinge = vec!["hinge".to_string()];
        let barbell = vec![("equipment".to_string(), "barbell".to_string())];
        let machine = vec![("equipment".to_string(), "machine".to_string())];

        assert_eq!(
            fatigue_profile("Full Squat", &squat, &barbell),
            (true, true)
        );
        assert_eq!(
            fatigue_profile("Sumo Deadlift", &hinge, &barbell),
            (true, true)
        );
        assert_eq!(
            fatigue_profile("Hack Squat Machine", &squat, &machine),
            (false, false)
        );
    }

    #[test]
    fn json_content_type_accepts_parameters_only() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("Application/JSON; charset=UTF-8"),
        );
        assert!(is_json_content_type(&headers));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(!is_json_content_type(&headers));
    }
}
