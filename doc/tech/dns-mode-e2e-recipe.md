# mHost DNS 模式 E2E 验证脚本

> Issue: #152 — 完整 DNS disable 兜底链路验证
> 适用: macOS only（DNS mode 仅在 macOS 启用）
> 版本: 1.0
> 日期: 2026-08-08

本文档是 issue #152 的端到端验证手册。每个 scenario 都假设用户在 macOS 开发机上以 home Wi-Fi 连接，且系统 DNS 当前是 DHCP-empty 状态（即 `networksetup -getdnsservers Wi-Fi` 输出 `There aren't any DNS Servers set on Wi-Fi`）。

---

## 0. 前置准备

```bash
# 0.1 确认系统 DNS 是 DHCP-empty（绝大多数家用 Wi-Fi 默认状态）
networksetup -getdnsservers Wi-Fi
# 期望输出: There aren't any DNS Servers set on Wi-Fi

# 0.2 准备 runtime dir 测试副本，避免污染真实 runtime
export MHOST_RUNTIME_DIR=/tmp/mhost-e2e-$USER-$$
mkdir -p "$MHOST_RUNTIME_DIR"

# 0.3 准备日志收集
export RUST_LOG=mhost_dns=debug,mhost_dns_proxy=debug,mhost=info
LOGFILE=/tmp/mhost-e2e-$$.log
echo "logging to $LOGFILE"

# 0.4 启动 mhost（foreground，方便观察日志）
pnpm tauri dev 2>&1 | tee "$LOGFILE" &
APP_PID=$!
```

---

## 1. Scenario A — Happy path DhcpEmpty

**目标**: 验证 disable 后系统 DNS 正确还原成 Empty。

```bash
# 1.1 启用 DNS 模式（UI 操作 or `set_dns_mode true` IPC）
# 期望：30 秒内看到以下日志
grep -E 'enable_dns_mode: entered|invoking osascript|received shutdown signal|system DNS restored' "$LOGFILE"
#   - [INFO] enable_dns_mode: entered (dns_port=1053)
#   - [INFO] enable_dns_mode: invoking osascript (timeout=60s)
#   - [INFO] enable_dns_mode: osascript returned: status=ExitStatus(0)

# 1.2 验证系统 DNS 已切到 127.0.0.1
networksetup -getdnsservers Wi-Fi
# 期望: 127.0.0.1

# 1.3 验证域名解析走 mhost（query 一个简单域名）
dig +short @127.0.0.1 example.com
# 期望: 一个真实 IP（说明 mhost DNS server 在 work）

# 1.4 等 30 秒，让用户做 disable 操作
# 期望：disable 后 5 秒内还原
sleep 5
networksetup -getdnsservers Wi-Fi
# 期望: There aren't any DNS Servers set on Wi-Fi

# 1.5 验证日志链路
grep -E 'received shutdown signal|restoring system DNS|system DNS restored' "$LOGFILE"
#   - [mhost-dns-proxy] restoring system DNS on Wi-Fi to Empty (DHCP default)
#   - [mhost-dns-proxy] system DNS restored

# 1.6 验证 on-disk original.txt 没有 127.0.0.1 污染
cat "$MHOST_RUNTIME_DIR/mhost-dns-original.txt" 2>/dev/null
# 期望: 文件不存在（DhcpEmpty 不写）或内容不含 127.0.0.1
```

---

## 2. Scenario B — 快速 re-enable（D3-2 race 回归）

**目标**: 验证 disable→re-enable 在 1 秒内发生时，新 enable 不会误杀正在 self-restore 的 proxy。

```bash
# 2.1 enable（先让 proxy 跑起来）
# UI 操作 Enable DNS Mode
sleep 2

# 2.2 立刻 disable（手动 5 秒倒计时之前完成）
# UI 操作 Disable DNS Mode

# 2.3 disable 还没完成（5s 等待循环中）时，立刻 re-enable
# UI 操作 Enable DNS Mode（重新启用）

# 2.4 期望：整套操作在 10 秒内完成，没有 hang 60s 也没失败
grep -E 'disable_dns_mode|enable_dns_mode: osascript returned' "$LOGFILE"
#   - 期望三条调用都 status=ExitStatus(0)
#   - 不期望 'recovery marker found'（说明本次成功还原，没有触发兜底）

# 2.5 验证最终 DNS 状态正确
networksetup -getdnsservers Wi-Fi
# 期望: There aren't any DNS Servers set on Wi-Fi

# 2.6 验证 log 中 inline orphan-kill 走的 PID-targeted 路径
grep -E 'kill -TERM|kill -KILL|ps -p.*comm=|stat -f %m' "$LOGFILE"
# 期望看到 ps -p <pid> -o comm= 的精确匹配调用，不是无脑 pgrep
```

---

## 3. Scenario C — Mid-restore kill（force-restore 兜底）

**目标**: 验证 disable 中途杀 proxy，下次启动的 try_recover_dns 兜底生效。

```bash
# 3.1 enable
# UI 操作 Enable DNS Mode
sleep 2

# 3.2 启动 disable
# UI 操作 Disable DNS Mode
# （disable 5s 等待循环中）

# 3.3 在等待循环期间硬杀 proxy
PROXY_PID=$(awk '{print $1}' "$MHOST_RUNTIME_DIR/mhost-dns-proxy.pid")
echo "killing proxy PID=$PROXY_PID"
kill -9 "$PROXY_PID"
# 此时 mhost 端 `kill(pid,0)!=0` 检测到 proxy 死，进入 post-restore verify

# 3.4 期望日志（顺序）:
grep -E 'disable_dns_mode: signal sent|proxy exited but system DNS still|escalating to sudo|force restore' "$LOGFILE"
#   - [mHost] dns mode disable: signal sent to proxy, waiting for exit
#   - [mHost] dns mode disable: proxy exited but system DNS still points at loopback; escalating to sudo fallback
#   - interactive 路径弹 sudo 让用户授权
#   - [mHost] dns mode disable: osascript restore succeeded
#   - DNS = Empty（用户授权后 sudo 兜底成功）
# 或（interactive=false 路径）:
#   - 保留 recovery marker 文件
#   - 下次启动 try_recover_dns 看到 marker，弹 sudo，DNS = Empty

# 3.5 验证 on-disk marker 状态（interactive 路径应被清掉）
ls -la "$MHOST_RUNTIME_DIR/mhost-dns-disable-recovery.marker" 2>&1
# 期望（interactive=true）: No such file or directory（成功路径）
# 期望（interactive=false）: 文件存在，content="pending"
```

---

## 4. Scenario D — configd 抖动（networksetup 调用失败）

**目标**: 模拟 proxy 的 `restore_dns_and_exit` 里 networksetup 调用失败（configd 抖），验证 mhost 端 post-restore verify 能探测到并升级到 sudo 兜底。

```bash
# 4.1 把真实的 mhost-dns-proxy binary 替换成 stub，让其 networksetup 调用一定失败
#    （这个 stub 模拟「proxy 启动成功但 networksetup 调不通」场景）
cat > /tmp/mhost-dns-proxy-stub.sh <<'EOF'
#!/bin/sh
# 模拟 proxy：bind 一个假的 UDP socket 假装 ready，但 disable 时不调 networksetup
trap "" TERM INT
echo "ready" > "$1/mhost-dns-proxy.ready"  # 传 runtime_dir 作为 $1
echo $$ > "$1/mhost-dns-proxy.pid"
echo "$0" >> "$1/mhost-dns-proxy.pid"
# 等 disable 信号
while true; do
    sleep 1
done
EOF
chmod +x /tmp/mhost-dns-proxy-stub.sh

# 4.2 替换 mhost 安装目录下的 mhost-dns-proxy
#   （用本地 dev build 的 mhost.app/Contents/MacOS/mhost-dns-proxy）
cp "$(find . -path '*/MacOS/mhost-dns-proxy' -type f | head -1)" /tmp/mhost-dns-proxy-backup
cp /tmp/mhost-dns-proxy-stub.sh "$(find . -path '*/MacOS/mhost-dns-proxy' -type f | head -1)"

# 4.3 启动 mhost，enable，然后 disable
# 期望：disable 后 5s 等待 → proxy 退出（kill by trap） → post-restore verify 失败
#       → 升级到 sudo 兜底 → 用户授权 → DNS = Empty

grep -E 'proxy exited but system DNS still|post-restore verify failed|escalating to sudo' "$LOGFILE"
# 期望看到 escalate 日志

# 4.4 验证最终 DNS 状态
networksetup -getdnsservers Wi-Fi
# 期望: There aren't any DNS Servers set on Wi-Fi（sudo 兜底成功）

# 4.5 还原真实 binary
cp /tmp/mhost-dns-proxy-backup "$(find . -path '*/MacOS/mhost-dns-proxy' -type f | head -1)"
```

---

## 5. Scenario E — Legacy data migration（pre-fix manifest 污染）

**目标**: 验证 pre-fix manifest 里如果写了 `original_dns: ["127.0.0.1"]`，mhost 启动时 OriginalDns 反序列化会过滤掉 loopback，不会再写回系统 DNS。

```bash
# 5.1 在 manifest 里手工注入污染数据（模拟老用户从 pre-fix 版本升级）
MANIFEST="$HOME/Library/Application Support/mHost/manifest.json"
# 备份
cp "$MANIFEST" /tmp/manifest-backup.json

# 5.2 用 jq 注入 legacy 污染数据
jq '.original_dns = ["127.0.0.1"]' "$MANIFEST" > /tmp/manifest-polluted.json
mv /tmp/manifest-polluted.json "$MANIFEST"

# 5.3 启动 mhost，enable → disable
# 期望：disable 时 restore_argv() 把 ["127.0.0.1"] 过滤成 []
#       然后 fallback 到 ["Empty"]，DNS = Empty

networksetup -getdnsservers Wi-Fi
# 期望: There aren't any DNS Servers set on Wi-Fi
# 不期望: 127.0.0.1（说明污染数据被滤掉，没有被当 original 还原）

# 5.4 还原 manifest
cp /tmp/manifest-backup.json "$MANIFEST"
```

---

## 6. 日志 grep 一览表

| 期望日志 | 含义 |
|----------|------|
| `enable_dns_mode: entered` | enable 路径入口 |
| `enable_dns_mode: invoking osascript (timeout=60s)` | 进入提权脚本 |
| `enable_dns_mode: osascript returned: status=ExitStatus(0)` | enable 成功 |
| `received shutdown signal` | proxy 检测到 disable signal |
| `restoring system DNS on Wi-Fi to Empty` | proxy 自管恢复开始 |
| `system DNS restored` | proxy 自管恢复成功 |
| `kill -TERM "$proxy_pid"` | disable 等待循环检测到 proxy 退出 |
| `proxy exited but system DNS still points at loopback; escalating to sudo fallback` | post-restore verify 失败，升级到 sudo |
| `post-restore verify failed` | verify_dns_restored_against_loopback 自身失败 |
| `try_recover_dns: disable recovery marker found at ...` | 下次启动兜底命中 |
| `force restore failed` | sudo 兜底也失败 |
| `recovery marker left at ...` | marker 保留，下次启动 retry |

| 不期望日志 | 含义 |
|------------|------|
| `bind: Address already in use` | port 53 被占（orphan proxy 没清干净） |
| `Failed to enable DNS mode` | enable 失败（一般配合 osascript 超时） |
| `dns-proxy failed to become ready within 5s` | ready 文件超时（proxy 启动失败） |
| `recovery marker found` 在 successful disable 后 | 误报（说明 marker 没被清） |

---

## 7. 清理

```bash
# 7.1 停 mhost
kill "$APP_PID" 2>/dev/null
wait "$APP_PID" 2>/dev/null

# 7.2 清理临时文件
rm -rf "$MHOST_RUNTIME_DIR"
rm -f "$LOGFILE"

# 7.3 如果 Scenario D 替换了 binary，确认已还原
ls -la "$(find . -path '*/MacOS/mhost-dns-proxy' -type f | head -1)"
# 应该看到正常的 mhost-dns-proxy Mach-O 二进制，不是 stub 脚本
```

---

## 8. 已知限制

- **真实 sudo 弹窗**: Scenario C / D / E 都依赖真实 sudo 授权（macOS TCC），无法在 CI 中跑。手动跑过一次后即可确认行为。
- **Wi-Fi 切换**: Scenario A 假设用户稳定连接 Wi-Fi；如果中途断网 / 切到有线，networksetup 输出会改变，建议在稳定的 home Wi-Fi 环境下测。
- **macOS 版本差异**: 早期 macOS（< 12）的 networksetup 输出格式略有差异，但只要是 DHCP-empty 状态，输出都是 `There aren't any DNS Servers set on Wi-Fi`。
- **TCC 缓存**: 第一次跑 Scenario A 之前用户可能需要授权一次 mhost（系统弹窗）。授权后 5 分钟内不再弹（macOS 默认缓存策略）。

---

## 9. 关联文档

- `dns-mode-tech-design.md` — DNS 模式整体架构
- `dns-mode-development-plan.md` — DNS 模式开发里程碑
- `rust-tauri-hosts-tech-route.md` — Rust + Tauri 技术路径
- Issue #152 讨论历史 — 完整 regression diff + 候选 root cause 分析
- `src-tauri/crates/mhost-dns/src/platform.rs` — `verify_dns_restored_against_loopback` 实现 + tests
- `src-tauri/crates/mhost-core/src/models.rs` — `OriginalDns::restore_argv` 防御层 + tests
