#!/usr/bin/env bash
# Scaffold a new thoughts post: page module with register_post! + mod decl.
# Indexes/feeds/routes pick the post up from the inventory registry — do not
# edit src/content/posts.rs.
#
# Usage:
#   just thought new                          # interactive prompts
#   just thought new <slug>                   # prompt for title/teaser/tags
#   just thought new <slug> "<title>" ["<teaser>" ["<tags>"]]
#   bash scripts/thought-new.sh …
#
# Slug is the URL segment (kebab-case). Date is today (local). Tags are
# comma-separated lowercase labels (at least one required).

set -Eeuo pipefail

usage() {
    printf 'usage: just thought new [<slug> ["<title>" ["<teaser>" ["<tags>"]]]]\n' >&2
    printf '  no args  prompt for slug, title, teaser, tags\n' >&2
    printf '  slug     kebab-case URL segment, e.g. pesky-code\n' >&2
    printf '  title    display title, e.g. "Pesky code"\n' >&2
    printf '  teaser   index-card blurb (default: "TODO")\n' >&2
    printf '  tags     comma-separated lowercase labels, e.g. "ai, dogs"\n' >&2
    exit 2
}

[[ $# -le 4 ]] || usage

slug="${1-}"
title="${2-}"
teaser="${3-}"
tags_raw="${4-}"
did_prompt=0

slug_to_title() {
    local s="${1//-/ }"
    printf '%s%s' "$(printf '%s' "${s:0:1}" | tr '[:lower:]' '[:upper:]')" "${s:1}"
}

prompt() {
    # prompt NAME DEFAULT -> writes answer to $REPLY (empty → default)
    local name="$1" default="${2-}" hint
    if [[ -n "$default" ]]; then
        hint=" [$default]"
    else
        hint=""
    fi
    printf '%s%s: ' "$name" "$hint" >&2
    if ! IFS= read -r REPLY; then
        printf '\nerror: unexpected EOF while reading %s\n' "$name" >&2
        exit 1
    fi
    if [[ -z "$REPLY" ]]; then
        REPLY="$default"
    fi
    did_prompt=1
}

# Parse "ai, dogs" / "ai,dogs" into a bash array of validated tags.
# Writes tag strings to the nameref array; returns non-zero on empty/invalid.
parse_tags() {
    local -n _out="$1"
    local raw="$2"
    _out=()
    local IFS=','
    # shellcheck disable=SC2086
    set -- ${raw}
    local tag
    for tag in "$@"; do
        # trim whitespace
        tag="${tag#"${tag%%[![:space:]]*}"}"
        tag="${tag%"${tag##*[![:space:]]}"}"
        [[ -n "$tag" ]] || continue
        if [[ ! "$tag" =~ ^[a-z]+$ ]]; then
            printf 'error: tag "%s" must be lowercase ascii letters only\n' "$tag" >&2
            return 1
        fi
        _out+=("$tag")
    done
    if [[ ${#_out[@]} -eq 0 ]]; then
        printf 'error: at least one tag is required\n' >&2
        return 1
    fi
    return 0
}

if [[ -z "$slug" ]]; then
    while true; do
        prompt "slug"
        slug="$REPLY"
        if [[ -z "$slug" ]]; then
            printf '  slug is required\n' >&2
            continue
        fi
        if [[ "$slug" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]]; then
            break
        fi
        printf '  slug must be kebab-case ([a-z0-9]+(-[a-z0-9]+)*)\n' >&2
    done
elif [[ ! "$slug" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]]; then
    printf 'error: slug must be kebab-case ([a-z0-9]+(-[a-z0-9]+)*)\n' >&2
    exit 1
fi

if [[ -z "$title" ]]; then
    prompt "title" "$(slug_to_title "$slug")"
    title="$REPLY"
    if [[ -z "$title" ]]; then
        printf 'error: title is required\n' >&2
        exit 1
    fi
fi

if [[ -z "$teaser" ]]; then
    # Prompt when interactive, or when earlier fields were prompted (piped
    # full-interactive answers include a teaser line). Quiet default otherwise.
    if [[ -t 0 || "$did_prompt" -eq 1 ]]; then
        prompt "teaser" "TODO"
        teaser="$REPLY"
    else
        teaser="TODO"
    fi
fi
[[ -n "$teaser" ]] || teaser="TODO"

tags=()
if [[ -z "$tags_raw" ]]; then
    if [[ -t 0 || "$did_prompt" -eq 1 ]]; then
        while true; do
            prompt "tags (comma-separated)"
            tags_raw="$REPLY"
            if parse_tags tags "$tags_raw"; then
                break
            fi
        done
    else
        printf 'error: tags are required (comma-separated lowercase labels)\n' >&2
        exit 1
    fi
elif ! parse_tags tags "$tags_raw"; then
    exit 1
fi

mod_name="${slug//-/_}"
date="$(date +%Y-%m-%d)"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
page_path="${repo_root}/src/app/thoughts/${mod_name}.rs"
thoughts_mod="${repo_root}/src/app/thoughts.rs"
thoughts_dir="${repo_root}/src/app/thoughts"

if [[ -e "$page_path" ]]; then
    printf 'error: %s already exists\n' "$page_path" >&2
    exit 1
fi

if [[ -d "${thoughts_dir}/${mod_name}" ]]; then
    printf 'error: directory %s already exists\n' "${thoughts_dir}/${mod_name}" >&2
    exit 1
fi

if grep -qE "^pub mod ${mod_name};" "$thoughts_mod"; then
    printf 'error: mod %s already declared in thoughts.rs\n' "$mod_name" >&2
    exit 1
fi

if grep -rqE "slug: \"${slug}\"" "$thoughts_dir"; then
    printf 'error: slug "%s" already registered under src/app/thoughts/\n' "$slug" >&2
    exit 1
fi

# Confirm when we prompted for anything.
if [[ "$did_prompt" -eq 1 ]]; then
    printf '\nCreate /thoughts/%s (%s)?\n' "$slug" "$date" >&2
    printf '  title:  %s\n' "$title" >&2
    printf '  teaser: %s\n' "$teaser" >&2
    local tags_joined
    tags_joined="$(IFS=', '; echo "${tags[*]}")"
    printf '  tags:   %s\n' "$tags_joined" >&2
    prompt "proceed?" "Y"
    case "$REPLY" in
        Y|y|yes|YES) ;;
        *)
            printf 'aborted\n' >&2
            exit 1
            ;;
    esac
fi

# Write the page module (Python so $/"/\ in titles stay literal).
python3 - "$page_path" "$slug" "$mod_name" "$title" "$date" "$teaser" "${tags[@]}" <<'PY'
import sys
from pathlib import Path

page_path = Path(sys.argv[1])
slug, mod_name, title, date, teaser = sys.argv[2:7]
tags = sys.argv[7:]

def rust_str(s: str) -> str:
    return s.replace("\\", "\\\\").replace('"', '\\"')

title_esc, teaser_esc = rust_str(title), rust_str(teaser)
tags_lit = ", ".join(f'"{rust_str(t)}"' for t in tags)

page_path.write_text(
    f'''use topcoat::{{Result, router::page, view::view}};

use crate::components::shell;

crate::register_post!(
    essay,
    slug: "{slug}",
    title: "{title_esc}",
    date: "{date}",
    teaser: "{teaser_esc}",
    tags: &[{tags_lit}],
);

#[page("/thoughts/{slug}")]
async fn {mod_name}() -> Result {{
    view! {{
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
                        "TODO"
                    </p>
                </div>
            </article>
        )
    }}
}}
'''
)
PY

# Insert `pub mod …;` in alphabetical order among existing pub mod lines.
insert_before="$(
    grep -nE '^pub mod [a-z0-9_]+;' "$thoughts_mod" \
        | while IFS=: read -r lineno decl; do
            name="${decl#pub mod }"
            name="${name%;}"
            if [[ "$name" > "$mod_name" ]]; then
                printf '%s\n' "$lineno"
                break
            fi
        done
)"
tmp="$(mktemp)"
if [[ -n "$insert_before" ]]; then
    {
        head -n "$((insert_before - 1))" "$thoughts_mod"
        printf 'pub mod %s;\n' "$mod_name"
        tail -n +"$insert_before" "$thoughts_mod"
    } >"$tmp"
else
    last_mod_line="$(grep -nE '^pub mod [a-z0-9_]+;' "$thoughts_mod" | tail -1 | cut -d: -f1)"
    if [[ -z "$last_mod_line" ]]; then
        printf 'error: no pub mod lines found in thoughts.rs\n' >&2
        exit 1
    fi
    {
        head -n "$last_mod_line" "$thoughts_mod"
        printf 'pub mod %s;\n' "$mod_name"
        tail -n +"$((last_mod_line + 1))" "$thoughts_mod"
    } >"$tmp"
fi
mv "$tmp" "$thoughts_mod"

printf 'created %s\n' "src/app/thoughts/${mod_name}.rs"
printf 'wired   mod %s in src/app/thoughts.rs\n' "$mod_name"
printf 'registered /thoughts/%s (%s) via register_post!\n' "$slug" "$date"
printf 'edit the body in src/app/thoughts/%s.rs\n' "$mod_name"
