#!/usr/bin/env bash

set -Eeuo pipefail

site_port="${1:-3000}"
pg_container=benjisponge-pg
pg_port=5490
pg_url="postgresql://postgres:dev@127.0.0.1:${pg_port}/benjisponge"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

# Local Postgres (18, matching production) for everything the site reads
# in-process. Seed data with:
#   just sync-spire --api "http://127.0.0.1:${site_port}"
#   just reset-fitness-local   (or: just sync-fitness <csv> --api ...)
printf 'dev: ensuring Postgres on 127.0.0.1:%s\n' "${pg_port}"
if [[ "$(docker inspect -f '{{.State.Running}}' "${pg_container}" 2>/dev/null)" != "true" ]]; then
    if docker inspect "${pg_container}" >/dev/null 2>&1; then
        docker start "${pg_container}" >/dev/null
    else
        docker run -d --name "${pg_container}" \
            -e POSTGRES_PASSWORD=dev -e POSTGRES_DB=benjisponge \
            -v benjisponge-pg-data:/var/lib/postgresql \
            -p "127.0.0.1:${pg_port}:5432" \
            postgres:18-alpine >/dev/null
    fi
fi
until docker exec "${pg_container}" pg_isready -U postgres -q 2>/dev/null; do
    sleep 0.3
done
# The container stays up between dev sessions (named volume
# benjisponge-pg-data holds the data); `docker rm -f benjisponge-pg`
# to reclaim it.

printf 'dev: applying migrations\n'
(cd "${repo_root}" && POSTGRES_URL="${pg_url}" cargo run --quiet --bin migrate -- migration apply)

printf 'dev: starting Topcoat on port %s\n' "${site_port}"
cd "${repo_root}"
# Optional local secrets (mainly Google OAuth). See .env.dev.example /
# docs/auth.md. Loaded before the pinned locals below so a misplaced
# POSTGRES_URL in .env.dev cannot point the app at production.
if [[ -f "${repo_root}/.env.dev" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "${repo_root}/.env.dev"
    set +a
fi
# COOKIE_KEY is a fixed non-secret so viewer logins survive rebuilds;
# Google credentials come from .env.dev or the shell environment.
POSTGRES_URL="${pg_url}" \
    SPIRE_SYNC_TOKEN=local-development \
    FITNESS_SYNC_TOKEN=local-development \
    COOKIE_KEY=local-development-cookie-key-not-a-secret \
    SITE_ORIGIN="http://localhost:${site_port}" \
    GOOGLE_OAUTH_CLIENT_ID="${GOOGLE_OAUTH_CLIENT_ID:-}" \
    GOOGLE_OAUTH_CLIENT_SECRET="${GOOGLE_OAUTH_CLIENT_SECRET:-}" \
    HIDDEN_PAGE_ACCESS="${HIDDEN_PAGE_ACCESS:-}" \
    PORT="${site_port}" topcoat dev --bin benjisponge
