use topcoat::{
    Result,
    context::Cx,
    router::{page, query_params},
    view::{component, view},
};

use crate::{
    components::{link_label, page_head, shell},
    content::{
        experience::{EDUCATION, ROLES, Role, SKILLS, Tech, TechKind},
        patches::PATCHES,
    },
};

fn org_line(role: &Role) -> String {
    if role.place.is_empty() {
        role.org.to_string()
    } else {
        format!("{} · {}", role.org, role.place)
    }
}

/// Kind → accent: languages oxidize, libraries and disciplines patina,
/// tools and infrastructure stay steel.
fn chip_class(tech: &Tech) -> &'static str {
    match tech.kind {
        TechKind::Language => "chip chip-oxide",
        TechKind::Library | TechKind::Discipline => "chip chip-patina",
        TechKind::Tool => "chip chip-steel",
    }
}

fn is_active(tech: &Tech, filter: Option<&Tech>) -> bool {
    filter.is_some_and(|active| active.name == tech.name)
}

/// Class for a stack chip, marking the one the page is filtered on.
fn stack_chip_class(tech: &Tech, filter: Option<&Tech>) -> String {
    if is_active(tech, filter) {
        format!("{} chip-active", chip_class(tech))
    } else {
        chip_class(tech).to_string()
    }
}

/// The active chip toggles the filter off; every other chip turns it on.
fn chip_href_for(tech: &Tech, filter: Option<&Tech>) -> String {
    if is_active(tech, filter) {
        "/resume".to_string()
    } else {
        filter_href(tech.name)
    }
}

/// The canonical chip for a (case-insensitively matched) tech name, if any
/// role's stack mentions it.
fn find_tech(name: &str) -> Option<&'static Tech> {
    ROLES
        .iter()
        .flat_map(|role| role.stack.iter())
        .find(|tech| tech.name.eq_ignore_ascii_case(name))
}

fn touches(role: &Role, name: &str) -> bool {
    role.stack.iter().any(|tech| tech.name == name)
}

/// Filter link for a chip: the résumé queried by one technology.
fn filter_href(name: &str) -> String {
    format!("/resume?tech={}", crate::util::urlencode(name))
}

#[query_params(error = redirect("?"))]
struct ResumeQuery {
    tech: Option<String>,
}

#[page("/resume")]
async fn resume(cx: &Cx) -> Result {
    let q = query_params::<ResumeQuery>(cx)?;
    let filter = q.tech.as_deref().and_then(|name| find_tech(name.trim()));
    let title = match filter {
        Some(active) => format!("Résumé · {}", active.name),
        None => "Résumé".to_string(),
    };

    view! {
        shell(
            title: title.as_str(),
            active: "resume",
            resume_content()
        )
    }
}

/// The résumé body: the role timeline, education, skills, and patches. The
/// standalone page wraps it in the shell; the home deck renders it as the
/// phone's résumé pane. It reads `?tech=` from the request itself, so the
/// chip filter works on `/resume` and is simply absent on `/`.
#[component]
pub(crate) async fn resume_content(cx: &Cx) -> Result {
    let q = query_params::<ResumeQuery>(cx)?;
    // An unrecognized tech silently falls back to the full timeline.
    let filter = q.tech.as_deref().and_then(|name| find_tech(name.trim()));

    let shown: Vec<&Role> = match filter {
        Some(active) => ROLES.iter().filter(|r| touches(r, active.name)).collect(),
        None => ROLES.iter().collect(),
    };
    let filter_line = filter.map(|active| {
        format!(
            "{} of {} roles touched {}.",
            shown.len(),
            ROLES.len(),
            active.name
        )
    });

    view! {
            page_head(stamp: "timeline", title: "Résumé", lede: "")
            if let Some(line) = filter_line {
                <div class="rail-row mt-8">
                    <p class="rail-stamp rail-stamp-label">"filter"</p>
                    <div class="flex min-w-0 flex-wrap items-baseline gap-x-4 gap-y-1">
                        <p class="text-ink2">(line)</p>
                        if let Some(active) = filter {
                            if let Some(href) = active.href {
                                <a class="oxlink font-meta text-sm" href=(href)>
                                    link_label(label: "project page →")
                                </a>
                            }
                        }
                        <a class="oxlink font-meta text-sm" href="/resume">
                            "clear ×"
                        </a>
                    </div>
                </div>
            }
            <section class="mt-14 space-y-12">
                for role in shown.iter() {
                    <article class="rail-row">
                        <p class="rail-stamp">(role.span)</p>
                        <div class="min-w-0">
                            <h2
                                class="font-display text-2xl leading-snug font-semibold"
                            >
                                (role.title)
                            </h2>
                            <p class="mt-1 text-ink2">(org_line(role))</p>
                            if !role.bullets.is_empty() {
                                <ul
                                    class="role-bullets mt-3 max-w-prose space-y-1.5 text-ink2"
                                >
                                    for bullet in role.bullets.iter() {
                                        <li>(*bullet)</li>
                                    }
                                </ul>
                            }
                            <p class="mt-3 font-meta text-xs text-muted">
                                (role.dates)
                            </p>
                            if !role.stack.is_empty() {
                                <div class="mt-2 flex flex-wrap gap-1.5">
                                    for tech in role.stack.iter() {
                                        <a
                                            class=(stack_chip_class(tech, filter))
                                            href=(chip_href_for(tech, filter))
                                        >
                                            (tech.name)
                                            if is_active(tech, filter) {
                                                " ×"
                                            }
                                        </a>
                                    }
                                </div>
                            }
                        </div>
                    </article>
                }
            </section>

            <section class="mt-16 space-y-10 border-t border-hairline pt-10">
                <article class="rail-row">
                    <p class="rail-stamp">(EDUCATION.span)</p>
                    <div class="min-w-0">
                        <h2 class="font-display text-2xl leading-snug font-semibold">
                            (EDUCATION.school)
                        </h2>
                        <p class="mt-1 text-ink2">(EDUCATION.degree)</p>
                        <p class="mt-1 text-ink2">(EDUCATION.note)</p>
                    </div>
                </article>
                <div class="rail-row">
                    <p class="rail-stamp rail-stamp-label">"Skills"</p>
                    <div class="flex min-w-0 flex-wrap gap-1.5">
                        for skill in SKILLS.iter() {
                            // Skills that appear in a role's stack filter the
                            // timeline like any chip; the rest are plain tags.
                            if find_tech(skill.name).is_some() {
                                <a
                                    class=(stack_chip_class(skill, filter))
                                    href=(chip_href_for(skill, filter))
                                >
                                    (skill.name)
                                    if is_active(skill, filter) {
                                        " ×"
                                    }
                                </a>
                            } else if let Some(href) = skill.href {
                                <a class=(chip_class(skill)) href=(href)>(skill.name)</a>
                            } else {
                                <span class=(chip_class(skill))>(skill.name)</span>
                            }
                        }
                    </div>
                </div>
            </section>

            // The aside: hand-picked merged patches, shortlog-style. Small type
            // on purpose — the timeline above is the résumé; this is a hobby.
            <section class="mt-16 space-y-3 border-t border-hairline pt-10">
                <div class="rail-row">
                    <p class="rail-stamp rail-stamp-label">"patches"</p>
                    <p class="min-w-0 max-w-prose text-ink2">
                        "I technically made these contributions to these projects."
                        <br />
                        "Almost all meaningless, but, hey, I've been \
                     interested in cool stuff for a long time!"
                    </p>
                </div>
                for patch in PATCHES.iter() {
                    <div class="rail-row">
                        <p class="rail-stamp">(patch.year)</p>
                        <p class="min-w-0 font-meta text-sm text-ink2">
                            (patch.repo)
                            " — "
                            <a class="oxlink" href=(patch.url)>(patch.title)</a>
                        </p>
                    </div>
                }
            </section>
    }
}
