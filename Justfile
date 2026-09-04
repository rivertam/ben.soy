default:
    @just --list

# Enable the repository-managed Git hooks in this checkout
install-hooks:
    proto install lefthook
    lefthook install

# Start local SurrealDB, Topcoat with live reload, and Podrick if .env.dev configures it
dev *args:
    just fitness-wasm
    bash scripts/dev.sh {{args}}

# Replace local fitness tables and import a Strong CSV (run while `just dev` is active)
reset-fitness-local csv="/home/benji/Downloads/WorkoutData.csv":
    bash scripts/reset-fitness-local.sh "{{csv}}"

# Build the debug binary and extract its assets
build:
    just fitness-wasm
    cargo build
    topcoat asset bundle --bin benjisponge

# Build the required Fitness entry core + glue pair.
fitness-wasm:
    cargo build -p fitness-entry-worker --profile wasm --target wasm32-unknown-unknown
    wasm-bindgen --target no-modules --out-dir wasm-dist --out-name fitness_entry \
        target/wasm32-unknown-unknown/wasm/fitness_entry_worker.wasm

# Build Diary's optional offline module. Its own workspace carries the
# wasm-only SurrealDB patch and must stay separate from the server graph.
diary-wasm:
    bash scripts/vendor-surrealdb-core.sh
    cargo build --manifest-path crates/diary-worker/Cargo.toml \
        --profile wasm --target wasm32-unknown-unknown
    wasm-bindgen --target no-modules --out-dir wasm-dist --out-name diary_sync \
        crates/diary-worker/target/wasm32-unknown-unknown/wasm/diary_worker.wasm

# Build both browser Rust modules into wasm-dist/.
wasm: diary-wasm fitness-wasm

# Build the release binary and extract its assets
release:
    just fitness-wasm
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
    ZONE_ID="$(curl -sS "https://api.cloudflare.com/client/v4/zones?name=ben.soy" \
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

# Run formatting, lint, and test checks for the root workspace (site, shared
# cores, and the Fitness wasm skin; no wasm target needed). crates/diary-worker
# is deliberately NOT covered — it lives in its own excluded workspace, so a
# diary-core API change that breaks it surfaces at `just wasm` or the Docker
# wasm stage, not here.
check:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

# Run the test suite
test:
    cargo test
