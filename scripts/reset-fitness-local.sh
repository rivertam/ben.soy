#!/usr/bin/env bash

set -Eeuo pipefail

workout_csv="${1:-/home/benji/Downloads/WorkoutData.csv}"
fitness_api="${FITNESS_API:-http://127.0.0.1:3000}"
surreal_container=benjisponge-surrealdb
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

if [[ ! -r "${workout_csv}" ]]; then
    printf 'reset-fitness-local: CSV is not readable: %s\n' "${workout_csv}" >&2
    exit 1
fi

# Prove the local site is up and the local token works before destroying
# anything: an authorized-but-empty import must come back 400.
auth_status="$(
    curl --silent --output /dev/null --write-out '%{http_code}' \
        --request POST \
        --header 'Authorization: Bearer local-development' \
        --header 'Content-Type: application/json' \
        --data '{}' \
        "${fitness_api}/api/fitness/import" || true
)"
if [[ "${auth_status}" != 400 ]]; then
    printf 'reset-fitness-local: run `just dev` first (local API auth probe returned %s)\n' \
        "${auth_status}" >&2
    exit 1
fi

printf 'reset-fitness-local: deleting the local fitness records\n'
# The CLI submits each input line separately, so keep the transaction on one
# line. It exits zero for statement errors; the exact JSON result is the
# success check before the import is allowed to continue.
reset_query='BEGIN TRANSACTION; DELETE sets RETURN NONE;'\
' DELETE exercise_tags RETURN NONE; DELETE exercises RETURN NONE;'\
' DELETE exercise_muscles RETURN NONE; DELETE muscles RETURN NONE;'\
' DELETE workouts RETURN NONE;'\
' UPSERT fitness_meta:version SET k = "version", v = 0 RETURN NONE; COMMIT TRANSACTION;'
reset_output="$(
    printf '%s\n' "${reset_query}" |
        docker exec -i "${surreal_container}" /surreal sql \
            --endpoint http://127.0.0.1:8000 \
            --namespace benjisponge \
            --database benjisponge \
            --username root \
            --password dev \
            --json \
            --hide-welcome
)"
if [[ "${reset_output}" != '[null,[],[],[],[],[],[],[],null]' ]]; then
    printf 'reset-fitness-local: database reset failed:\n%s\n' "${reset_output}" >&2
    exit 1
fi

printf 'reset-fitness-local: importing %s\n' "${workout_csv}"
cd "${repo_root}"
FITNESS_SYNC_TOKEN=local-development cargo run --bin fitness_sync -- \
    "${workout_csv}" \
    --api "${fitness_api}" \
    --json
