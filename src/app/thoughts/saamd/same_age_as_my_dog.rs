use jiff::{Timestamp, ToSpan, civil::Date, tz::TimeZone};
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::Cx,
    router::{page, query_params},
    view::view,
};

use crate::components::{back_link, page_head, rail_prose, rail_section, shell};

crate::register_post!(
    essay,
    slug: "same-age-as-my-dog",
    title: "I'm the same age as my dog!",
    date: "2026-07-28",
    teaser: "This week, Felix and I are both 33.2 years old!",
    photo: {
        src: SAAMD_WITH_FELIX,
        alt: "Ben and Felix relaxing together in a hammock",
        width: 1060,
        height: 1413,
    },
    read_label: "calculate your SAAMD day →",
    tags: &["dogs"],
);

const SECONDS_PER_DAY: i64 = 24 * 60 * 60;
const DAYS_PER_YEAR: f64 = 365.2425;
const SAAMD_WITH_FELIX: Asset = asset!("./saamd with felix.jpg", rename: "saamd-with-felix");

#[query_params(error = redirect("?"))]
struct AgeQuery {
    human: Option<String>,
    dog: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Crossing {
    Date { date: Date, age_years: f64 },
    NoSharedLifetime,
}

fn crossing(human: Date, dog: Date) -> Crossing {
    if dog < human {
        return Crossing::NoSharedLifetime;
    }

    // At the dog's birth, the human has a head start of `gap_days`.
    // A dog aging at 7× closes that gap at a relative 6×, so the crossing
    // falls one sixth of the gap after the dog's birthday.
    let gap_days = human.duration_until(dog).as_secs() / SECONDS_PER_DAY;
    let days_after_dog_birth = gap_days / 6;
    let date = dog
        .checked_add(days_after_dog_birth.days())
        .expect("valid birthdays produce a representable crossing date");
    let age_days = human.duration_until(date).as_secs() / SECONDS_PER_DAY;

    Crossing::Date {
        date,
        age_years: age_days as f64 / DAYS_PER_YEAR,
    }
}

fn today_in_new_york() -> Date {
    let eastern = TimeZone::get("America/New_York").expect("bundled time zone exists");
    Timestamp::now().to_zoned(eastern).date()
}

fn parse_birthdays(
    query: &AgeQuery,
    today: Date,
) -> std::result::Result<Option<(Date, Date)>, &str> {
    let (Some(human), Some(dog)) = (query.human.as_deref(), query.dog.as_deref()) else {
        return Ok(None);
    };
    let human = human
        .parse::<Date>()
        .map_err(|_| "Enter both birthdays as valid dates.")?;
    let dog = dog
        .parse::<Date>()
        .map_err(|_| "Enter both birthdays as valid dates.")?;
    if human > today {
        return Err("Your birthday cannot be in the future.");
    }
    if dog > today {
        return Err("Your dog’s birthday cannot be in the future.");
    }
    Ok(Some((human, dog)))
}

fn format_date(date: Date) -> String {
    date.strftime("%B %-d, %Y").to_string()
}

#[page("/thoughts/same-age-as-my-dog")]
async fn same_age_as_my_dog(cx: &Cx) -> Result {
    let query = query_params::<AgeQuery>(cx)?;
    let today = today_in_new_york();
    let today_value = today.strftime("%Y-%m-%d").to_string();
    let human_value = query.human.as_deref().unwrap_or("");
    let dog_value = query.dog.as_deref().unwrap_or("");
    let parsed = parse_birthdays(query, today);

    view! {
        shell(title: "Same age as my dog", active: "", runtime: false, marker_font: true,
            <article>
                page_head(
                    stamp: POST.date,
                    title: "Same Age As My Dog",
                    lede: "Calculate when you and your dog are the same age",
                )

                rail_prose(stamp: "dog years",
                    <p>"
                      In case you didn't know, \"Dog Years\" are this silly concept where
                      dogs are supposed to age 7x as quickly as people, so when they're 1,
                      developmentally they're like a 7 year old. It's been pointed out to
                      me that this is stupid.
                    "</p>
                )

                rail_section(stamp: "birthdays",
                    <form method="get" class="max-w-xl rounded-lg border border-hairline bg-card p-5 sm:p-6">
                        <div class="grid gap-5 sm:grid-cols-2">
                            <label class="block">
                                <span class="font-meta text-xs font-semibold tracking-wide text-ink2">
                                    "Your birthday"
                                </span>
                                <input
                                    type="date"
                                    name="human"
                                    value=(human_value)
                                    max=(today_value.as_str())
                                    required=""
                                    class="mt-2 block w-full rounded-md border border-hairline bg-page px-3 py-2 font-body text-base text-ink focus:border-oxide focus:outline-none"
                                >
                            </label>
                            <label class="block">
                                <span class="font-meta text-xs font-semibold tracking-wide text-ink2">
                                    "Your dog’s birthday"
                                </span>
                                <input
                                    type="date"
                                    name="dog"
                                    value=(dog_value)
                                    max=(today_value.as_str())
                                    required=""
                                    class="mt-2 block w-full rounded-md border border-hairline bg-page px-3 py-2 font-body text-base text-ink focus:border-oxide focus:outline-none"
                                >
                            </label>
                        </div>
                        <button
                            type="submit"
                            class="mt-5 w-full cursor-pointer rounded-md bg-oxide px-4 py-2.5 font-meta text-sm font-semibold text-white hover:bg-oxide-hot focus:outline-2 focus:outline-offset-2 focus:outline-oxide"
                        >
                            "When do we match?"
                        </button>
                    </form>
                )

                match parsed {
                    Err(message) => {
                        rail_section(stamp: "result",
                            <div role="alert" class="max-w-xl border-l-4 border-oxide bg-card px-5 py-4">
                                <p class="font-display text-xl font-semibold text-oxide">"Try that again"</p>
                                <p class="mt-1 text-ink2">(message)</p>
                            </div>
                        )
                    }
                    Ok(Some((human, dog))) => {
                        match crossing(human, dog) {
                            Crossing::NoSharedLifetime => {
                                rail_section(stamp: "result",
                                    <div class="max-w-xl border-l-4 border-steel bg-card px-5 py-4" aria-live="polite">
                                        <p class="font-display text-2xl font-semibold">
                                            "There isn’t a matching day."
                                        </p>
                                        <p class="mt-2 text-ink2">
                                            "Your dog was born first, so under the seven-to-one rule \
                                             they were already ahead when your shared timeline began."
                                        </p>
                                    </div>
                                )
                            }
                            Crossing::Date { date, age_years } => {
                                let relation = if date < today {
                                    "You were"
                                } else if date == today {
                                    "Today, you are"
                                } else {
                                    "You will be"
                                };
                                let date_copy = if date == today {
                                    "today".to_string()
                                } else {
                                    format!("on {}", format_date(date))
                                };
                                rail_section(stamp: "result",
                                    <div class="max-w-xl border-l-4 border-patina bg-card px-5 py-4" aria-live="polite">
                                        <p class="font-display text-2xl font-semibold">
                                            (relation)
                                            " the same age "
                                            (date_copy.as_str())
                                            "."
                                        </p>
                                        <p class="mt-2 font-meta text-sm text-ink2">
                                            (format!("About {:.1} years old each.", age_years))
                                        </p>
                                    </div>
                                )
                            }
                        }
                    }
                    Ok(None) => {}
                }
                rail_prose(stamp: "mine",
                    <p>
                    <figure class="mx-auto w-[94%] max-w-md -rotate-[0.4deg] rounded-[3px] border border-[#d8cdbd] bg-[#f7f1e7] p-5 pb-7 shadow-[0_10px_24px_rgba(50,43,34,0.12)] sm:p-7 sm:pb-9">
                        <img
                            src=(SAAMD_WITH_FELIX)
                            alt="Ben and Felix relaxing together in a hammock"
                            width="1060"
                            height="1413"
                            loading="lazy"
                            decoding="async"
                            class="block h-auto w-full rounded-[1px] contrast-[0.98] saturate-[0.94] sepia-[0.06]"
                        >
                        <figcaption
                            class="rotate-[0.3deg] px-3 pt-6 text-center text-lg leading-snug font-bold text-[#29241f]"
                            style="font-family: 'Kalam', cursive"
                        >
                            "just a couple of 33.2 year old guys squinting in the sunlight!"
                        </figcaption>
                    </figure>
                    <p>"
                      My birthday is May 20, 1993, so our SAAMD day was July 28, 2026.
                    "</p>


                      <a class="oxlink" href="/felix">"Felix"</a>
                      " is a rescue dingus, so no one really knows how old he is.
                      They tend to estimate dog ages by looking at their teeth.
                      Supposedly he was 1 year old in early October 2022, but personally
                      I think he was a little younger. His foster name was Bat Boy and, as
                      it was the lead-up to Halloween, a lot of his first toys were
                      (and one of his favorites still is) spooky and bat themed. So I
                      pretend his birthday is October 31, 2021 as that feels right to me.
                      Hey, maybe it is right."
                    </p>
                )
                back_link(href: "/thoughts", label: "all thoughts")
            </article>
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(value: &str) -> Date {
        value.parse().unwrap()
    }

    #[test]
    fn crossing_is_one_sixth_of_the_head_start_after_dogs_birth() {
        let Crossing::Date {
            date: crossing_date,
            age_years,
        } = crossing(date("1990-01-01"), date("2020-01-01"))
        else {
            panic!("a later dog birthday must produce a crossing");
        };
        assert_eq!(crossing_date, date("2024-12-31"));
        assert!((age_years - 35.0).abs() < 0.01);
    }

    #[test]
    fn matching_birthdays_cross_at_birth() {
        assert_eq!(
            crossing(date("2020-06-12"), date("2020-06-12")),
            Crossing::Date {
                date: date("2020-06-12"),
                age_years: 0.0,
            }
        );
    }

    #[test]
    fn a_dog_born_first_has_no_crossing_during_both_lifetimes() {
        assert_eq!(
            crossing(date("2020-01-01"), date("2019-01-01")),
            Crossing::NoSharedLifetime
        );
    }
}
