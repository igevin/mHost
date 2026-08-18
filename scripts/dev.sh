#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# scripts/dev.sh — 一键启动 dev 模式
#
# issue #155 的工作流：`pnpm tauri dev` 默认不会构建 `mhost-dns-proxy`
# 这个独立 sidecar binary；本文先 build proxy 再启 dev，避免磁盘清理
# / 新克隆后 dev 模式拿不到 53 端口的 listener。
#
# 用法：
#   bash scripts/dev.sh                  # debug 构建 + 启动 dev（默认）
#   bash scripts/dev.sh --release        # release 构建 + 启动 dev
#   bash scripts/dev.sh -h|--help        # 打印用法
#
# Bash 兼容性（F6, PR #156 review）：
#   * 不在 `set -u` 下展开空数组 —— macOS 自带的 /bin/bash 3.2.57 会
#     把 "${arr[@]}" 在空数组时当成 unbound variable
#   * 用 `case` + `if` 显式分流，避免 silent ignore（F7）
# ---------------------------------------------------------------------------
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

# 1. 参数解析（F6/F7）
case "${1:-}" in
    "")
        CARGO_FLAGS=()
        TARGET_DIR="debug"
        PROFILE_LABEL="debug"
        ;;
    "--release")
        CARGO_FLAGS=(--release)
        TARGET_DIR="release"
        PROFILE_LABEL="--release"
        ;;
    "-h"|"--help")
        cat <<USAGE
Usage: $0 [OPTIONS]

Build the mhost-dns-proxy sidecar binary and start 'pnpm tauri dev'.
Without arguments, builds the debug profile (target/debug/mhost-dns-proxy).

OPTIONS:
  --release      build the release profile and invoke 'pnpm tauri dev --release'
  -h, --help     print this help and exit

USAGE
        exit 0
        ;;
    *)
        echo "$0: unknown argument: $1" >&2
        echo "Try '$0 --help' for usage." >&2
        exit 2
        ;;
esac

SRC_TAURI_TARGET="src-tauri/target/${TARGET_DIR}"

echo "==> Building mhost-dns-proxy (${PROFILE_LABEL} profile)..."
(
    cd src-tauri
    # 必须用 `-p mhost-dns --bin ...` 显式指定包：workspace root 的
    # `cargo build --bin mhost-dns-proxy` 会 fail 报 "no bin target named
    # 'mhost-dns-proxy' in default-run packages"，因为 mhost-dns 是
    # workspace member、不是 default-run package。
    if [ "${#CARGO_FLAGS[@]}" -gt 0 ]; then
        cargo build --package mhost-dns --bin mhost-dns-proxy "${CARGO_FLAGS[@]}"
    else
        cargo build --package mhost-dns --bin mhost-dns-proxy
    fi
)

PROXY_BIN="${SRC_TAURI_TARGET}/mhost-dns-proxy"
if [[ ! -x "$PROXY_BIN" ]]; then
    echo "❌ Build reported success but ${PROXY_BIN} not found or not executable." >&2
    exit 1
fi

# 2. 启动 dev
echo "==> mhost-dns-proxy ready at ${PROXY_BIN}"
echo "==> Starting pnpm tauri dev (${PROFILE_LABEL})..."
# F6 fix：不在 set -u 下展开空数组
if [ "${#CARGO_FLAGS[@]}" -gt 0 ]; then
    exec pnpm tauri dev "${CARGO_FLAGS[@]}"
else
    exec pnpm tauri dev
fi
