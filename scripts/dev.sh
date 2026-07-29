#!/usr/bin/env bash

set -Eeuo pipefail

site_port=3000
run_podrick=auto
podrick_reset=no
surreal_container=benjisponge-surrealdb
surreal_image=surrealdb/surrealdb:v3.2.3
surreal_port=5800
surreal_endpoint="ws://127.0.0.1:${surreal_port}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

usage='usage: just dev [port] [--no-podrick] [--podrick-reset]'

# The port stays positional because it predates the flags. `--no-podrick` is
# for a session where you want to drive the bot by hand in its own terminal;
# `--podrick-reset` clears all LOCAL Podrick state first, so the local
# experiment can start over. Re-announcing a lift you already have means
# deleting it too, since the reseed lands on the newest workout that exists
# (see docs/podrick.md — production has no such switch and must not grow one).
while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-podrick) run_podrick=off; shift ;;
        --podrick-reset) podrick_reset=yes; shift ;;
        -h|--help) printf '%s\n' "${usage}"; exit 0 ;;
        -*) printf 'dev: unknown flag: %s (%s)\n' "$1" "${usage}" >&2; exit 2 ;;
        *)
            [[ "$1" =~ ^[0-9]+$ ]] ||
                { printf 'dev: expected a port number, got: %s\n' "$1" >&2; exit 2; }
            site_port="$1"
            shift
            ;;
    esac
done

if [[ "${run_podrick}" == off && "${podrick_reset}" == yes ]]; then
    printf 'dev: --podrick-reset needs podrick running; drop --no-podrick\n' >&2
    exit 2
fi

# Local SurrealDB for everything the site reads in-process. The application
# bootstraps its schema after connecting. Seed data with:
#   just sync-spire --api "http://127.0.0.1:${site_port}"
#   just reset-fitness-local   (or: just sync-fitness <csv> --api ...)
printf 'dev: ensuring SurrealDB on 127.0.0.1:%s\n' "${surreal_port}"
if [[ "$(docker inspect -f '{{.State.Running}}' "${surreal_container}" 2>/dev/null)" != "true" ]]; then
    if docker inspect "${surreal_container}" >/dev/null 2>&1; then
        docker start "${surreal_container}" >/dev/null
    else
        docker run -d --name "${surreal_container}" \
            -e SURREAL_BIND=0.0.0.0:8000 \
            -e SURREAL_PATH=rocksdb:///home/nonroot/data.db \
            -e SURREAL_USER=root \
            -e SURREAL_PASS=dev \
            -v benjisponge-surrealdb-data:/home/nonroot \
            -p "127.0.0.1:${surreal_port}:8000" \
            "${surreal_image}" start >/dev/null
    fi
fi
until docker exec "${surreal_container}" /surreal is-ready \
    --endpoint http://127.0.0.1:8000 >/dev/null 2>&1; do
    sleep 0.3
done
# The container stays up between dev sessions (named volume
# benjisponge-surrealdb-data holds the data); remove the container and
# volume explicitly to reclaim them.

printf 'dev: starting Topcoat on port %s\n' "${site_port}"
cd "${repo_root}"
# Optional local secrets (mainly Google OAuth). See .env.dev.example /
# docs/auth.md. Loaded before the pinned locals below so a misplaced
# SurrealDB endpoint or credential in .env.dev cannot point the app at
# production.
if [[ -f "${repo_root}/.env.dev" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "${repo_root}/.env.dev"
    set +a
fi

# Podrick beside the site, so lift announcements and Pants Off ingestion can
# be exercised without remembering to start a second terminal. It runs only
# when `.env.dev` names one of its channels and a token resolves, so a checkout
# without Discord configured behaves exactly as it did before.
podrick_pid=

stop_podrick() {
    local pid="${podrick_pid}"
    podrick_pid=
    [[ -n "${pid}" ]] || return 0
    # One pid is the whole chain: this script backgrounds a subshell that
    # `exec`s podrick-local.sh, which `exec`s `cargo run`, which itself
    # exec-replaces with the binary on Unix. Nothing is left polling.
    kill "${pid}" 2>/dev/null || true
}

# Ctrl-C and closing the terminal signal the whole process group, so podrick
# dies with everything else and these traps are the backstop for Topcoat simply
# exiting. Signalling *only* this script is the one case that lags: bash runs
# the trap after the foreground Topcoat returns.
trap stop_podrick EXIT
trap 'stop_podrick; exit 130' INT
trap 'stop_podrick; exit 143' TERM

if [[ "${run_podrick}" == auto &&
      -n "${PODRICK_LIFT_CHANNEL_ID:-}${PODRICK_PANTS_CHANNEL_ID:-}${PODRICK_INFARCTIONS_CHANNEL_ID:-}" ]]; then
    if [[ -n "${DISCORD_BOT_TOKEN:-}" || -s "${HOME:-}/.config/benjisponge/podrick.token" ]]; then
        printf 'dev: podrick will start once the site answers (test Discord channels only)\n'
        podrick_args=(--port "${site_port}")
        if [[ "${podrick_reset}" == yes ]]; then
            podrick_args+=(--reset)
        fi
        # Tighter than the deployed 60s: locally, the wait between pasting a
        # lift and seeing it in the channel is the whole point.
        podrick_args+=(run --interval 10)
        (
            # Wait for the app, not just the port: podrick reads workout
            # content over HTTP, and a cold build takes a while. Bounded, so a
            # build that never succeeds cannot leave a poller waiting forever.
            deadline=$((SECONDS + 300))
            until curl --silent --fail --output /dev/null --max-time 5 \
                "http://127.0.0.1:${site_port}/api/fitness/facets"; do
                if ((SECONDS > deadline)); then
                    printf 'dev: site never answered on port %s — podrick not started\n' \
                        "${site_port}" >&2
                    exit 1
                fi
                sleep 1
            done
            exec bash "${script_dir}/podrick-local.sh" "${podrick_args[@]}"
        ) &
        podrick_pid=$!
    else
        printf 'dev: skipping podrick — a Podrick channel is set but no bot\n' >&2
        printf '     token was found (set DISCORD_BOT_TOKEN in .env.dev, or write\n' >&2
        printf '     ~/.config/benjisponge/podrick.token). See docs/podrick.md.\n' >&2
    fi
fi

# COOKIE_KEY is a fixed non-secret so viewer logins survive rebuilds;
# Google credentials come from .env.dev or the shell environment.
SURREALDB_ENDPOINT="${surreal_endpoint}" \
    SURREALDB_NAMESPACE=benjisponge \
    SURREALDB_DATABASE=benjisponge \
    SURREALDB_USERNAME=root \
    SURREALDB_PASSWORD=dev \
    SPIRE_SYNC_TOKEN=local-development \
    FITNESS_SYNC_TOKEN=local-development \
    COOKIE_KEY=local-development-cookie-key-not-a-secret \
    SITE_ORIGIN="http://localhost:${site_port}" \
    GOOGLE_OAUTH_CLIENT_ID="${GOOGLE_OAUTH_CLIENT_ID:-}" \
    GOOGLE_OAUTH_CLIENT_SECRET="${GOOGLE_OAUTH_CLIENT_SECRET:-}" \
    PORT="${site_port}" topcoat dev --bin benjisponge
