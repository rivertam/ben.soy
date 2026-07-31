#!/usr/bin/env bash
# Commit and push pending thought changes (page modules + mod decl).
# Post metadata lives beside each page via register_post! — src/content/posts.rs
# is shared registry infrastructure and is not staged here.
#
# Usage:
#   just thought publish
#   THOUGHT_PUBLISH_YES=1 just thought publish   # non-interactive
#
# Only stages thought paths — other dirty files are left alone.

set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
cd "$repo_root"

thought_paths=(
    src/app/thoughts.rs
    src/app/thoughts
)

# Porcelain lines for thought-related paths only (including untracked).
mapfile -t changes < <(
    git status --porcelain --untracked-files=all -- "${thought_paths[@]}"
)

if [[ ${#changes[@]} -eq 0 ]]; then
    printf 'nothing to publish (no pending thought changes)\n' >&2
    exit 0
fi

slugs=()
while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    # XY PATH or XY ORIG -> PATH (rename); path starts at column 4.
    path="${line:3}"
    path="${path##* -> }"
    if [[ "$path" != *.rs || ! -f "$path" ]]; then
        continue
    fi
    # Prefer the register_post! slug when present (works for flat + folder posts).
    while IFS= read -r slug_line; do
        if [[ "$slug_line" =~ slug:\ \"([^\"]+)\" ]]; then
            slugs+=("${BASH_REMATCH[1]}")
            break
        fi
    done < <(grep -E '^\s*slug: "' "$path" || true)
done < <(printf '%s\n' "${changes[@]}")

# Deduplicate while preserving order.
declare -A seen=()
unique_slugs=()
for s in "${slugs[@]+"${slugs[@]}"}"; do
    [[ -z "${seen[$s]+x}" ]] || continue
    seen[$s]=1
    unique_slugs+=("$s")
done

if [[ ${#unique_slugs[@]} -eq 1 ]]; then
    msg="Add thought: ${unique_slugs[0]}"
elif [[ ${#unique_slugs[@]} -gt 1 ]]; then
    joined="$(printf '%s, ' "${unique_slugs[@]}")"
    msg="Add thoughts: ${joined%, }"
else
    msg="Update thoughts"
fi

printf 'Pending thought changes:\n' >&2
printf '%s\n' "${changes[@]}" >&2
printf '\nCommit: %s\nPush to remote after commit.\n' "$msg" >&2

if [[ -t 0 ]]; then
    printf 'proceed? [Y]: ' >&2
    if ! IFS= read -r answer; then
        printf '\nerror: unexpected EOF\n' >&2
        exit 1
    fi
    answer="${answer:-Y}"
    case "$answer" in
        Y|y|yes|YES) ;;
        *)
            printf 'aborted\n' >&2
            exit 1
            ;;
    esac
elif [[ "${THOUGHT_PUBLISH_YES:-}" != 1 ]]; then
    printf 'error: refusing non-interactive publish (set THOUGHT_PUBLISH_YES=1)\n' >&2
    exit 1
fi

git add -- "${thought_paths[@]}"

# Re-check index; nothing staged → abort (e.g. assume-unchanged oddities).
if git diff --cached --quiet -- "${thought_paths[@]}"; then
    printf 'error: nothing staged after git add\n' >&2
    exit 1
fi

git commit -m "$(cat <<EOF
${msg}

EOF
)"

if git rev-parse --abbrev-ref --symbolic-full-name '@{u}' >/dev/null 2>&1; then
    git push
else
    git push -u origin HEAD
fi

printf 'published\n'
