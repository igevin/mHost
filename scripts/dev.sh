#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# scripts/dev.sh — 一键启动 dev 模式
#
# issue #155 的临时缓解：`pnpm tauri dev` 默认不会构建 `mhost-dns-proxy`
# 这个独立 sidecar binary；磁盘清理 / 全新克隆后 binary 不在
# target/debug/，DNS mode 会被静默失败。本文先构建 proxy 再启 dev。
#
# 用法：
#   bash scripts/dev.sh                    # debug 构建 + 启动 dev
#   bash scripts/dev.sh --release          # release 构建 + dev（少见）
#
# 不写 `pnpm dev:full` 是因为 build-and-version.md / dev-guide.md 这些文档里
# 已经到处引用 `pnpm tauri dev` / `pnpm tauri build`，改文档比改工具链风险
# 小。脚本化的入口更明确、不会让 `pnpm tauri` 系列命令泄漏到工程别处。
# ---------------------------------------------------------------------------
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

# 1. 选 profile
PROFILE_FLAGS=()
if [[ "${1:-}" == "--release" ]]; then
    PROFILE_FLAGS=(--release)
    TARGET_DIR="release"
    echo "==> Building in --release profile"
else
    TARGET_DIR="debug"
fi
SRC_TAURI_TARGET="src-tauri/target/${TARGET_DIR}"

# 2. 构建 mhost-dns-proxy sidecar binary
#    `cargo build --bin mhost-dns-proxy` 只是构建 [[bin]] target；workspace
#    root 的 [[bin]] (mhost) 由 pnpm tauri dev 自己跑 cargo run 的时候会构建，
#    不会重复构建已 up-to-date 的依赖。
echo "==> Building mhost-dns-proxy (debug)..."
(
    cd src-tauri
    # 注意：必须用 `-p mhost-dns --bin ...` 显式指定包。
    # `cargo build --bin mhost-dns-proxy` 在 workspace root 会失败：
    # "no bin target named `mhost-dns-proxy` in default-run packages"，
    # 因为 mhost-dns 是 workspace member、不是 default-run package。
    cargo build --package mhost-dns --bin mhost-dns-proxy "${PROFILE_FLAGS[@]}"
)

PROXY_BIN="${SRC_TAURI_TARGET}/mhost-dns-proxy"
if [[ ! -x "$PROXY_BIN" ]]; then
    echo "❌ Build reported success but ${PROXY_BIN} not found or not executable." >&2
    exit 1
fi

# 3. 启动 dev
echo "==> mhost-dns-proxy ready at ${PROXY_BIN}"
echo "==> Starting pnpm tauri dev..."
exec pnpm tauri dev "${PROFILE_FLAGS[@]}"
