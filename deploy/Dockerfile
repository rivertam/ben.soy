# Build context is the repo root (see wrangler.jsonc image_build_context / Justfile).

FROM rust:1.97-slim AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install topcoat-cli --version 0.5.0 --locked
# topcoat-cli strips CARGO_HOME before its internal `cargo build`, so the
# inner build falls back to /root/.cargo. Point the outer build there too or
# `topcoat asset bundle` recompiles every dependency from scratch.
ENV CARGO_HOME=/root/.cargo
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs .gitignore ./
COPY .cargo ./.cargo
COPY crates ./crates
COPY src ./src
COPY styles ./styles
COPY data ./data
# build.rs downloads the standalone Tailwind v4 CLI here (needs network + certs)
RUN cargo build --release
# --bin: the crate also builds the spire_sync CLI, and the bundler needs to
# know which binary's assets to extract.
RUN topcoat asset bundle --release --bin benjisponge

# The diary queue's Rust half (docs/diary-sync.md): diary-worker compiled for
# wasm32 with the size-first `wasm` profile, then wasm-bindgen emits the
# no-modules glue the service worker importScripts. BuildKit runs this stage
# in parallel with the site build above. The CLI version must match
# diary-worker's pinned wasm-bindgen crate exactly.
FROM rust:1.97-slim AS wasm-builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl patch \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
RUN cargo install wasm-bindgen-cli --version 0.2.126 --locked
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs .gitignore ./
COPY .cargo ./.cargo
COPY crates ./crates
COPY src ./src
COPY scripts/vendor-surrealdb-core.sh ./scripts/vendor-surrealdb-core.sh
COPY deploy/surrealdb-core-wasm-time.patch ./deploy/surrealdb-core-wasm-time.patch
RUN bash scripts/vendor-surrealdb-core.sh
RUN cargo build --manifest-path crates/diary-worker/Cargo.toml \
    --profile wasm --target wasm32-unknown-unknown
RUN wasm-bindgen --target no-modules --out-dir wasm-dist --out-name diary_sync \
    crates/diary-worker/target/wasm32-unknown-unknown/wasm/diary_worker.wasm

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/benjisponge /app/benjisponge
# The Podrick Discord bot ships in the same image and runs as its own Railway
# service with `/app/podrick run` as its start command (docs/podrick.md). It
# serves no HTTP and needs no assets.
COPY --from=builder /build/target/release/podrick /app/podrick
# AssetBundle::load() walks up from the exe looking for assets/manifest.toml
COPY --from=builder /build/target/assets /app/assets
# diary_sync.rs reads the wasm pair from ./wasm-dist relative to the working
# directory; missing files would quietly disable the diary's offline queue.
COPY --from=wasm-builder /build/wasm-dist /app/wasm-dist
ENV HOST=0.0.0.0 PORT=8080
EXPOSE 8080
CMD ["/app/benjisponge"]
