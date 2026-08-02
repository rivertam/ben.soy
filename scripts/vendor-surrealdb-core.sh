#!/usr/bin/env bash
# Materialize vendor/surrealdb-core: the pristine crates.io 3.2.3 tarball
# plus deploy/surrealdb-core-wasm-time.patch (a wasm-only fix for the
# tokio::time::Instant panic — upstream issue #6711's unresolved half).
# Only the diary-worker wasm build consumes this via [patch.crates-io];
# the site's server workspace never sees it. Idempotent: a completed
# vendor dir is left alone, so `just wasm` stays cheap.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=3.2.3
SHA256=d6add99c7f91bda7acb5d3b9a1b7c0db81a1adebdaedd4b9145f9ccf65fa2a25
DEST=vendor/surrealdb-core
STAMP="$DEST/.patched"
PATCH=deploy/surrealdb-core-wasm-time.patch

if [ -f "$STAMP" ] && [ "$PATCH" -ot "$STAMP" ]; then
    exit 0
fi

mkdir -p vendor
TARBALL="vendor/surrealdb-core-$VERSION.crate"
if [ ! -f "$TARBALL" ]; then
    CACHED=$(ls "$HOME"/.cargo/registry/cache/*/surrealdb-core-$VERSION.crate 2>/dev/null | head -1 || true)
    if [ -n "$CACHED" ]; then
        cp "$CACHED" "$TARBALL"
    else
        curl -fsSL "https://static.crates.io/crates/surrealdb-core/surrealdb-core-$VERSION.crate" \
            -o "$TARBALL"
    fi
fi
echo "$SHA256  $TARBALL" | sha256sum -c - >/dev/null

rm -rf "$DEST"
mkdir -p "$DEST"
tar xzf "$TARBALL" -C "$DEST" --strip-components=1
patch -p1 -s -d "$DEST" < "$PATCH"
touch "$STAMP"
echo "vendored surrealdb-core $VERSION with wasm time patch into $DEST"
