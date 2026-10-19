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
# Modes:
#   (default)  release: cross-build both Apple triples for `tauri build`
#             (beforeBuildCommand).
#   --dev      debug: build the native triple only (fast), place the binary
#             at the sidecar's runtime location `target/debug/` AND stage the
#             `bin/mhost-dns-proxy-<triple>` copy that tauri-build's
#             externalBin compile-time validation requires. Used by
#             `beforeDevCommand` so stock `pnpm tauri dev` works without
#             scripts/dev.sh (issue #160).
#
# See:
#   - src-tauri/tauri.conf.json        (bundle.externalBin / beforeDevCommand)
#   - .github/workflows/release.yml    (matrix + rust target list)

set -euo pipefail

# Run from the repo root regardless of where this script is invoked from.
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

mkdir -p src-tauri/bin

if [[ "${1:-}" == "--dev" ]]; then
    TRIPLE="$(rustc -vV | sed -n 's|host: ||p')"
    echo "[mhost-sidecar] building mhost-dns-proxy (debug, native $TRIPLE)"
    cargo build \
        --manifest-path src-tauri/Cargo.toml \
        --package mhost-dns \
        --bin mhost-dns-proxy

    SRC="src-tauri/target/debug/mhost-dns-proxy"
    if [[ ! -x "$SRC" ]]; then
        echo "[mhost-sidecar] expected built binary at $SRC; not found" >&2
        exit 1
    fi

    # Runtime location: sidecar path resolution is `<current_exe dir>/mhost-dns-proxy`
    # (see mhost_lib::resolve_proxy_sidecar_path), which for `tauri dev` is
    # exactly where cargo already placed it — nothing to copy.
    # Staging copy: satisfies tauri-build's externalBin compile-time check
    # (same reason as the CI step that calls this script with --dev).
    cp "$SRC" "src-tauri/bin/mhost-dns-proxy-${TRIPLE}"
    echo "[mhost-sidecar] dev sidecar ready at $SRC (staged bin/mhost-dns-proxy-${TRIPLE})"
    exit 0
fi

if [[ "${1:-}" != "" ]]; then
    echo "$0: unknown argument: $1" >&2
    echo "Usage: $0 [--dev]" >&2
    exit 2
fi

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
