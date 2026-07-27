#!/usr/bin/env bash

set -Eeuo pipefail

site_port="${1:-3000}"
surreal_container=benjisponge-surrealdb
surreal_image=surrealdb/surrealdb:v3.2.3
surreal_port=5800
surreal_endpoint="ws://127.0.0.1:${surreal_port}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

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
