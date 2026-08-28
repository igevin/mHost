#!/usr/bin/env bash
#
# Build `mhost-dns-proxy` for both Apple Silicon targets and copy them
# to `src-tauri/bin/mhost-dns-proxy-<target-triple>` so Tauri 2's
# `bundle.externalBin` can pick them up and place the correct arch into
# `mhost.app/Contents/MacOS/mhost-dns-proxy` at build time.
#
# Why both targets in one run:
#   release.yml runs tauri build in two matrix entries:
#     - --target aarch64-apple-darwin
#     - --target x86_64-apple-darwin
#   Each entry needs the matching `-<triple>` file. Building both here
#   in beforeBuildCommand means both matrix entries can use the same
#   `bin/` directory. Cross-compile aarch64 -> x86_64 is supported by
#   stock Rust on macOS (no special toolchain needed once the rustup
#   target is added).
#
# Failure mode (intentional):
#   If either target is missing (e.g., local dev machine without
#   `rustup target add x86_64-apple-darwin`), the script fails fast
#   rather than silently shipping only one arch.
#
# See:
#   - src-tauri/tauri.conf.json        (bundle.externalBin)
#   - .github/workflows/release.yml    (matrix + rust target list)

set -euo pipefail

# Run from the repo root regardless of where this script is invoked from.
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

mkdir -p src-tauri/bin

for TRIPLE in aarch64-apple-darwin x86_64-apple-darwin; do
    echo "[mhost-sidecar] building mhost-dns-proxy for $TRIPLE"
    cargo build --release \
        --manifest-path src-tauri/Cargo.toml \
        --package mhost-dns \
        --target "$TRIPLE" \
        --bin mhost-dns-proxy

    SRC="src-tauri/target/$TRIPLE/release/mhost-dns-proxy"
    DST="src-tauri/bin/mhost-dns-proxy-$TRIPLE"

    if [[ ! -f "$SRC" ]]; then
        echo "[mhost-sidecar] expected built binary at $SRC; not found" >&2
        exit 1
    fi

    cp "$SRC" "$DST"
    chmod +x "$DST"
    echo "[mhost-sidecar] $TRIPLE -> $DST"
done

echo
echo "[mhost-sidecar] bundle directory ready:"
ls -la src-tauri/bin/
