#!/usr/bin/env bash
#
# Delete one workout and its sets through the public REST resource:
# DELETE /api/fitness/workouts/by-path/{path}.
#
# The archive is otherwise create-only, so this is the "explicit replacement
# operation" docs/fitness.md reserves for corrections — delete, then repaste
# or resync. It shows the workout and asks before doing anything.

set -Eeuo pipefail

usage() {
    cat >&2 <<'USAGE'
usage: just delete-lift <path> [--api <origin>] [--token <token>] [--yes]

  <path>    the permanent path segment, e.g. 2026-07-27T13-42-00-04-00
            (the last component of https://ben.soy/lifting/...)
  --api     API origin (default: https://ben.soy)
  --token   FITNESS_SYNC_TOKEN; default $FITNESS_SYNC_TOKEN, then
            ~/.config/benjisponge/fitness.token
  --yes     skip the confirmation prompt
USAGE
    exit 2
}

workout_path=""
api="https://ben.soy"
token="${FITNESS_SYNC_TOKEN:-}"
assume_yes=no

while [[ $# -gt 0 ]]; do
    case "$1" in
        --api) api="${2:-}"; [[ -n "${api}" ]] || usage; shift 2 ;;
        --token) token="${2:-}"; [[ -n "${token}" ]] || usage; shift 2 ;;
        --yes | -y) assume_yes=yes; shift ;;
        -h | --help) usage ;;
        -*) printf 'delete-lift: unknown flag %s\n' "$1" >&2; usage ;;
        *) [[ -z "${workout_path}" ]] || usage; workout_path="$1"; shift ;;
    esac
done
[[ -n "${workout_path}" ]] || usage

# Accept a pasted workout URL as well as a bare path segment — that URL is
# what the browser has on screen when you notice the lift is wrong.
workout_path="${workout_path##*/lifting/}"
workout_path="${workout_path%%[?#]*}"

if [[ -z "${token}" ]]; then
    token_file="${HOME}/.config/benjisponge/fitness.token"
    if [[ -r "${token_file}" ]]; then
        token="$(tr -d '[:space:]' <"${token_file}")"
    fi
fi
if [[ -z "${token}" ]]; then
    printf 'delete-lift: no token; pass --token or create %s\n' \
        "${HOME}/.config/benjisponge/fitness.token" >&2
    exit 1
fi

resource="${api}/api/fitness/workouts/by-path/${workout_path}"

# Read it back first. This both validates the path and shows what is about to
# go: a mistyped offset is a different workout, not an error.
workout="$(curl --silent --show-error --fail-with-body "${resource}" 2>&1)" || {
    printf 'delete-lift: no workout at %s\n%s\n' "${resource}" "${workout}" >&2
    exit 1
}
printf '%s' "${workout}" | python3 -c '
import json, sys
workout = json.load(sys.stdin)["workout"]
sets = workout["sets"]
title = workout["title"]
started = workout["started_at_local"]
print(f"  {title}")
print(f"  {started} · {len(sets)} sets")
seen = []
for one in sets:
    if one["exercise_name"] not in seen:
        seen.append(one["exercise_name"])
print("  " + ", ".join(seen))
'

if [[ "${assume_yes}" != yes ]]; then
    printf '\nDelete this workout and its sets from %s?\n' "${api}"
    printf 'Manual (pasted) workouts are not in the CSV and cannot be resynced.\n'
    read -r -p 'Type the workout path to confirm: ' confirmation
    if [[ "${confirmation}" != "${workout_path}" ]]; then
        printf 'delete-lift: not confirmed; nothing was deleted\n' >&2
        exit 1
    fi
fi

receipt="$(
    curl --silent --show-error --fail-with-body \
        --request DELETE \
        --header "Authorization: Bearer ${token}" \
        "${resource}" 2>&1
)" || {
    printf 'delete-lift: the delete failed\n%s\n' "${receipt}" >&2
    exit 1
}
printf '%s\n' "${receipt}"

# A `workout-data-csv` workout comes straight back on the next `just
# sync-fitness`, because sync resends any workout holding a missing set.
printf '%s' "${receipt}" | python3 -c '
import json, sys
receipt = json.load(sys.stdin)
deleted = receipt["sets_deleted"]
version = receipt["version"]
print(f"deleted {deleted} sets; fitness version is now {version}")
if receipt["source"] == "workout-data-csv":
    print("NOTE: source is workout-data-csv, so the next `just sync-fitness` will")
    print("      re-import this workout. Remove it from the export first.")
'
