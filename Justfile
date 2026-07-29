default:
    @just --list

# Enable the repository-managed Git hooks in this checkout
install-hooks:
    proto install lefthook
    lefthook install

# Start local SurrealDB, Topcoat with live reload, and Podrick if .env.dev configures it
dev *args:
    bash scripts/dev.sh {{args}}

# Replace local fitness tables and import a Strong CSV (run while `just dev` is active)
reset-fitness-local csv="/home/benji/Downloads/WorkoutData.csv":
    bash scripts/reset-fitness-local.sh "{{csv}}"

# Build the debug binary and extract its assets
build:
    cargo build
    topcoat asset bundle --bin benjisponge

# Build the release binary and extract its assets
release:
    cargo build --release
    topcoat asset bundle --release --bin benjisponge

# Optional: redeploy the web service and purge the Cloudflare CDN cache
deploy:
    #!/usr/bin/env bash
    set -euo pipefail
    railway link \
      --project 096cd9a2-678d-42bc-9212-4d0fbe1e1ecc \
      --environment 07803718-f8a6-4bc9-945a-a08f6a75584e \
      --service 9b0ab183-4157-4654-bc62-e13cdc59ce68
    railway up --ci -m "deploy $(git rev-parse --short HEAD)"
    ZONE_ID="$(curl -sS "https://api.cloudflare.com/client/v4/zones?name=benjisponge.com" \
      -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"][0]["id"])')"
    curl -sS -X POST "https://api.cloudflare.com/client/v4/zones/${ZONE_ID}/purge_cache" \
      -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" \
      -H "Content-Type: application/json" \
      -d '{"purge_everything":true}' \
      | python3 -c 'import json,sys; r=json.load(sys.stdin); assert r["success"], r'

# Upload new Slay the Spire 1 and 2 runs to the site's database (see --help)
sync-spire *args:
    cargo run --bin spire_sync -- {{args}}

# Upload a Strong workout CSV export to the site's fitness database (see --help)
sync-fitness csv *args:
    cargo run --bin fitness_sync -- "{{csv}}" {{args}}

# Delete one lift and its sets by permanent path; `just delete-lift --help`
delete-lift *args:
    bash scripts/delete-lift.sh {{args}}

# Run the Podrick Discord bot (see --help); defaults to the PRODUCTION api
podrick *args:
    cargo run --bin podrick -- {{args}}

# Podrick against the local stack by hand; `just dev` already runs it (--no-podrick opts out)
podrick-local *args:
    bash scripts/podrick-local.sh {{args}}

# Thought posts: `just thought new`, `just thought publish` (see `just thought`)
mod thought

# Run formatting, lint, and test checks
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Run the test suite
test:
    cargo test
