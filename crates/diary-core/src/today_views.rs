//! Today screen and its offline connection notice.
use crate::{
    entry::DiaryEntry,
    today::{self, Day},
    views,
};
use topcoat::{
    Result,
    view::{component, view},
};

#[component]
pub async fn lane_nav(active: &'static str) -> Result {
    view! {
        <nav class="diary-lanes" aria-label="Diary">
            <a href="/diary/now" aria-current=(if active == "now" { Some("page") } else { None })>"Now"</a>
            <a href="/diary/today" aria-current=(if active == "today" { Some("page") } else { None })>"Today"</a>
            <span>"just you"</span>
        </nav>
    }
}

#[component]
pub async fn today_page(
    day: String,
    current_day: String,
    days: Vec<Day>,
    cues: Vec<DiaryEntry>,
    store_ok: bool,
) -> Result {
    let found = days.iter().find(|entry| entry.day == day);
    let body = found.map(|entry| entry.body.as_str()).unwrap_or("");
    let closed = found.is_some_and(|entry| entry.closed);
    let past = day != current_day;
    let remaining = found
        .map(Day::remaining_ms)
        .unwrap_or(today::BUDGET_MS)
        .div_ceil(1000);
    let year: i16 = day
        .get(..4)
        .and_then(|year| year.parse().ok())
        .unwrap_or(2026);
    let dates = today::calendar(year);
    let cells: std::collections::BTreeMap<_, _> = days
        .iter()
        .map(|entry| (entry.day.as_str(), entry.status()))
        .collect();
    let start_offset = dates
        .first()
        .and_then(|day| day.parse::<jiff::civil::Date>().ok())
        .map(|date| date.weekday().to_sunday_zero_offset())
        .unwrap_or(0);
    let cues = today::memory_cues(cues, &day, |entry| entry);
    view! {
        <section class="diary-today" id="diary-today" data-day=(day.as_str())
            data-reflection=(serde_json::to_string(&found).expect("reflection serializes"))
            data-current-day=(current_day.as_str()) data-closed=(if closed { "true" } else { "false" })>
            lane_nav(active: "today")
            <div class="diary-day-heading">
                <h1>(if past { day.clone() } else { "Today".to_string() })</h1>
                <form method="get" action="/diary/today" class="diary-day-picker">
                    <label for="diary-day">"Browse a day"</label>
                    <input id="diary-day" type="date" name="day" value=(day.as_str()) max=(current_day.as_str()) required="">
                    <button type="submit">"Open"</button>
                </form>
            </div>
            <p class="diary-day-note">"A day begins at 4 a.m. in New York."</p>
            if !store_ok {
                <p role="status">"The diary store is unavailable. Connect again to load your saved reflection."</p>
            }
            <div class="diary-writing-bar">
                <span><output id="diary-time">(format!("{}:{:02}", remaining / 60, remaining % 60))</output>" writing left"</span>
                <span id="diary-clock-state">(if closed { "Closed" } else if past { "Read only" } else { "Ready" })</span>
            </div>
            if !past && !closed {
                <p class="diary-day-note">"Autosaves online. The clock pauses after 5 seconds idle, or when you leave. Take as long as you need to think."</p>
            }
            <div id="diary-writing" hidden=(!closed && !past)>
                <div id="diary-prompts" hidden="">
                    <details class="diary-prompts">
                        <summary>"A place to start, if you want one"</summary>
                        <details><summary>"Explore something on your mind"</summary>
                            <p>"What is staying with you? Explore the feelings underneath it and what it means to you. Let the words be unfinished."</p>
                        </details>
                        <details><summary>"Look at a thought more closely"</summary>
                            <p>"What happened, and what did you think and feel? What supports that thought, and what complicates it? What would a fairer view sound like?"</p>
                        </details>
                        <details><summary>"Stay with something good"</summary>
                            <p>"Recall a moment of warmth, interest, relief, or joy. What happened? Notice the details and why that moment mattered to you."</p>
                        </details>
                    </details>
                </div>
                <label for="diary-reflection" class="sr-only">"Daily reflection"</label>
                <textarea id="diary-reflection" class="diary-reflection" readonly=""
                    maxlength=(crate::entry::MAX_ENTRY_CHARS.to_string())
                    placeholder=(if closed { "This reflection was closed without text." } else if past { "No reflection written." } else { "What do you want to put into words today?" })
                    spellcheck="true" autocomplete="off">(body)</textarea>
            </div>
            <div class="diary-today-actions">
                <button type="button" id="diary-start" disabled="" aria-controls="diary-writing" aria-expanded="false" hidden=(closed || past)>(if found.is_some() { "Resume" } else { "Start" })</button>
                <button type="button" id="diary-finish" hidden="" disabled="">"Finish for today"</button>
            </div>
            <p id="diary-today-status" role="status" aria-live="polite">
                (if closed { "Saved and closed. You can read this whenever you like." }
                  else if past { "This day is available to read." }
                  else { "Loading your saved writing time…" })
            </p>
            <noscript><p>"Today needs JavaScript to autosave and keep your writing time across visits."</p></noscript>
            <details class="diary-cues" open=(!cues.is_empty())>
                <summary>"Now entries for this day"</summary>
                <div id="diary-cues">
                    if cues.is_empty() { <p class="diary-day-note">"No Now entries for this day."</p> }
                    for entry in cues {
                        views::bubble(item: views::Bubble::synced(&entry))
                    }
                </div>
                <template id="diary-cue-template">
                    views::bubble(item: views::Bubble::synced(&DiaryEntry::from_parts("", 0, "")))
                </template>
            </details>
            <section class="diary-calendar" aria-label="Daily reflections">
                <div class="diary-calendar-heading">
                    <h2>(format!("{year} reflections"))</h2>
                    <div>
                        <a href=(format!("/diary/today?day={}-12-31", year - 1))>"Earlier"</a>
                        if day < current_day {
                            <a href="/diary/today">"Today"</a>
                        }
                    </div>
                </div>
                <div class="diary-heatmap" style=(format!("--diary-start: {}", start_offset + 1))>
                    for date in dates {
                        if date <= current_day {
                            <a href=(format!("/diary/today?day={date}"))
                                data-day=(date.as_str())
                                data-status=(cells.get(date.as_str()).copied().unwrap_or("empty"))
                                aria-label=(format!("{}: {}", date, cells.get(date.as_str()).copied().unwrap_or("empty")))
                                title=(format!("{}: {}", date, cells.get(date.as_str()).copied().unwrap_or("empty")))
                                aria-current=(if date == day { Some("date") } else { None })></a>
                        } else {
                            <span data-status="future"></span>
                        }
                    }
                </div>
                <p class="diary-legend"><span data-status="empty">"Empty"</span>
                    <span data-status="started">"Started"</span><span data-status="closed">"Closed"</span></p>
                <p class="diary-day-note">"Every closed reflection counts equally. Now entries don’t count."</p>
            </section>
        </section>
    }
}

#[component]
pub async fn connection_required() -> Result {
    view! {
        <section class="diary-today">
            lane_nav(active: "today")
            <h1>"Today"</h1>
            <p>"Today needs a live connection to load and autosave your reflection."</p>
            <p><a href="/diary/today">"Try again"</a>" · "<a href="/diary/now">"Write in Now offline"</a></p>
        </section>
    }
}
