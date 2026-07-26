# Build context is the repo root (see wrangler.jsonc image_build_context / Justfile).

FROM rust:1.97-slim AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install topcoat-cli --version 0.4.0 --locked
# topcoat-cli strips CARGO_HOME before its internal `cargo build`, so the
# inner build falls back to /root/.cargo. Point the outer build there too or
# `topcoat asset bundle` recompiles every dependency from scratch.
ENV CARGO_HOME=/root/.cargo
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs .gitignore ./
COPY src ./src
COPY styles ./styles
COPY data ./data
# build.rs downloads the standalone Tailwind v4 CLI here (needs network + certs)
RUN cargo build --release
# --bin: the crate also builds the spire_sync CLI, and the bundler needs to
# know which binary's assets to extract.
RUN topcoat asset bundle --release --bin benjisponge

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/benjisponge /app/benjisponge
# AssetBundle::load() walks up from the exe looking for assets/manifest.toml
COPY --from=builder /build/target/assets /app/assets
ENV HOST=0.0.0.0 PORT=8080
EXPOSE 8080
CMD ["/app/benjisponge"]
