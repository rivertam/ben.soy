#!/usr/bin/env bash
#
# Run Podrick against the local dev stack: lift content comes from the local
# site and all Podrick state (including Pants Off history) stays in the local
# database. Discord channels themselves are still real, so `.env.dev` must
# name test channels.
#
# `just dev` starts this automatically whenever `.env.dev` names a channel and
# a token resolves; run it by hand for a one-off pass, a `--dry-run`, or a
# `--reset` — pair those with `just dev --no-podrick` so two bots are not
# polling the same database.
#
# Everything Podrick reads is pinned local here: the SurrealDB connection and
# the API origin it fetches message content from. `.env.dev` is sourced FIRST
# and the locals are applied AFTER, so a stray production endpoint or
# credential in `.env.dev` cannot point this at the real database — the same
# ordering, for the same reason, as scripts/dev.sh.
#
# What comes from `.env.dev`: the PODRICK_* channel ids, and
# DISCORD_BOT_TOKEN if you keep it there. Otherwise the token falls back to
# ~/.config/benjisponge/podrick.token, like the other sync clients.

set -Eeuo pipefail

site_port=3000
surreal_endpoint="ws://127.0.0.1:5800"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

# Pull out the local-only flags; everything else is Podrick's own (`--help`).
forwarded=()
reset=no
while [[ $# -gt 0 ]]; do
    case "$1" in
        --port)
            site_port="${2:-}"
            [[ -n "${site_port}" ]] || { printf 'podrick-local: --port needs a value\n' >&2; exit 2; }
            shift 2
            ;;
        --reset) reset=yes; shift ;;
        *) forwarded+=("$1"); shift ;;
    esac
done

if [[ -f "${repo_root}/.env.dev" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "${repo_root}/.env.dev"
    set +a
fi

if [[ -z "${PODRICK_LIFT_CHANNEL_ID:-}${PODRICK_PANTS_CHANNEL_ID:-}${PODRICK_INFARCTIONS_CHANNEL_ID:-}" ]]; then
    printf 'podrick-local: no Podrick channel is set.\n' >&2
    printf '  Put test channel ids in %s/.env.dev (see .env.dev.example).\n' "${repo_root}" >&2
    exit 2
fi
if [[ -n "${PODRICK_PANTS_CHANNEL_ID:-}" && -z "${PODRICK_INFARCTIONS_CHANNEL_ID:-}" ]]; then
    printf 'podrick-local: PODRICK_INFARCTIONS_CHANNEL_ID is required with PODRICK_PANTS_CHANNEL_ID.\n' >&2
    exit 2
fi
if [[ -z "${PODRICK_PANTS_CHANNEL_ID:-}" && -n "${PODRICK_INFARCTIONS_CHANNEL_ID:-}" ]]; then
    printf 'podrick-local: PODRICK_PANTS_CHANNEL_ID is required with PODRICK_INFARCTIONS_CHANNEL_ID.\n' >&2
    exit 2
fi

site_api="http://127.0.0.1:${site_port}"

# The lift job reads workout content over HTTP, so a dead site would look like
# a string of announce failures rather than "start `just dev` first". Pants
# Off by itself needs only Discord and the local database.
if [[ -n "${PODRICK_LIFT_CHANNEL_ID:-}" ]] &&
   ! curl --silent --fail --output /dev/null --max-time 5 "${site_api}/api/fitness/facets"; then
    printf 'podrick-local: no site on %s — start `just dev %s` first.\n' \
        "${site_api}" "${site_port}" >&2
    exit 1
fi

# Local-only escape hatch. Production has no reset command and must not grow
# one (docs/podrick.md); clearing the LOCAL state just lets an experiment
# start over.
if [[ "${reset}" == yes ]]; then
    printf 'podrick-local: clearing all local Podrick cursors, claims, and Pants Off history\n'
    reset_output="$(
        printf '%s\n' 'BEGIN TRANSACTION; DELETE podrick_announcements; DELETE podrick_pants_actions; DELETE podrick_pants_messages; DELETE podrick_meta; COMMIT TRANSACTION;' |
            docker exec -i benjisponge-surrealdb /surreal sql \
                --endpoint http://127.0.0.1:8000 \
                --namespace benjisponge \
                --database benjisponge \
                --username root \
                --password dev \
                --json \
                --hide-welcome
    )"
    if [[ "${reset_output}" != '[null,[],[],[],[],null]' ]]; then
        printf 'podrick-local: reset failed:\n%s\n' "${reset_output}" >&2
        exit 1
    fi
fi

printf 'podrick-local: local database and API (%s); Discord channels are live test channels\n' \
    "${site_api}"
printf 'podrick-local: a first run seeds this local watermark at the NEWEST lift that\n'
printf '               already exists and announces nothing; lifts pasted from now on\n'
printf '               are announced. Pants history is imported silently before live\n'
printf '               infarctions and kwerms begin. `--reset` starts both over.\n'

cd "${repo_root}"
SURREALDB_ENDPOINT="${surreal_endpoint}" \
    SURREALDB_NAMESPACE=benjisponge \
    SURREALDB_DATABASE=benjisponge \
    SURREALDB_USERNAME=root \
    SURREALDB_PASSWORD=dev \
    exec cargo run --bin podrick -- --api "${site_api}" "${forwarded[@]}"
