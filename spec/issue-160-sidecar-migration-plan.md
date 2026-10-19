# Issue #160 开发计划：mhost-dns-proxy 迁移到 Tauri sidecar

> 目标 issue: [#160](https://github.com/flyhigher139/mHost/issues/160)
> 状态: **已实施**（分支 `feat/160-dns-sidecar-migration`）
> 关联: #155 (原始报告) / #156 (已合并的安全修复) / #158 #159 (延后项) / #67 (跨平台，依赖本 issue)

## 0. 实施期决策修正（与第 2 节对照阅读）

调研阶段发现原计划 Step 1–2（引入 tauri-plugin-shell、用 `app.shell().sidecar()`
解析）的前提不成立，经确认后改为**不加插件**的方案：

- `tauri-plugin-shell`（2.0–2.2.1，核对到 2.2.1 源码
  `src/process/mod.rs::relative_command_path`）的 sidecar 解析实现就是
  `current_exe().parent().join(name)` —— 与要替换掉的探测**完全等价**，
  引入插件对路径发现零收益，还白增依赖与能力面。
- Tauri 2 核心（2.11.3）的 `tauri::process` 模块不含 sidecar 功能，
  运行时解析只此一家。
- 冷启动恢复 `try_recover_dns`（`AppState::new` 内）早于 Tauri app
  创建，拿不到 `AppHandle`，插件 API 在那里用不了。

最终形态：Tauri 主 crate 单一 helper
`resolve_proxy_sidecar_path`（`src-tauri/src/lib.rs`），语义注明与插件
一致；`commands/dns.rs` 与 `state/mod.rs` 两处调用共用；
`platform::enable_dns_mode(proxy_path, …)` 接收注入。其余步骤
（dev 工作流、CI、文档）按原计划执行。

## 1. 背景与现状盘点

Issue 目标是把 proxy 的「`current_exe().parent()` 手探路径 + 手写 osascript 提权脚本」换成 Tauri 2 一等公民的 sidecar 机制。

**已完成（不在本计划范围）**：

- ✅ `tauri.conf.json::bundle.externalBin: ["bin/mhost-dns-proxy"]`（PR #146），sidecar 随 .app 发布
- ✅ `beforeBuildCommand` 调 `scripts/build-proxy-sidecar.sh` 交叉编译两个 Apple triple
- ✅ `validate_proxy_binary` pre-check（#156），静默失效已有明确报错

**未完成（本计划范围）**：

- ❌ 二进制定位仍是 `current_exe().parent().join("mhost-dns-proxy")` 探测（`platform.rs:796-799`）
- ❌ `scripts/dev.sh` 仍必需，裸 `pnpm tauri dev` 不工作
- ❌ 提权脚本里仍内嵌路径探测/校验逻辑

## 2. 两个决定方案形态的关键事实（调研结论）

### 2.1 sidecar API 无提权能力

`app.shell().sidecar("mhost-dns-proxy").spawn()` 启动的是 **app 同 uid 的用户进程**。proxy 需要 root 才能 `bind 127.0.0.1:53`，所以提权步骤无法被 sidecar 取代。Issue 正文 Approach 第 6 条的 "Practical answer" 与此一致：

> keep `osascript` for the bind-53 step but launch **just** the binary via `Command::sidecar()`.

净收益重新定位：

1. **消灭 `current_exe()` 探测** —— sidecar 路径由 Tauri 按 `externalBin` 声明统一解析，dev（`target/debug/` 解析规则）和 release（`Contents/MacOS/`）一致，不再耦合 Tauri 内部布局。
2. **dev 工作流一等化** —— 用 `beforeDevCommand` 让裸 `pnpm tauri dev` 可工作，`scripts/dev.sh` 退休。
3. **能力边界收口** —— 二进制路径由 capability/插件体系解析，不再是 IPC 层不可见的 out-of-band 约定。

### 2.2 crates/ 无 Tauri 依赖的架构约束

`src-tauri/crates/mhost-dns` 是纯逻辑 crate（不依赖 tauri），`platform.rs::enable_dns_mode` 不能调用 `app.shell()`。因此：

- **路径解析上移到 IPC 层**：`commands/dns.rs::set_dns_mode_enable` 用 `ShellExt` 解析出 sidecar 绝对路径，作为参数传入 `platform::enable_dns_mode`。
- `platform.rs` 的签名从「自己找二进制」变成「接收二进制路径」，纯逻辑边界保持干净，也正好符合 #67 跨平台复用的方向（Windows/Linux 的 IPC 层各自解析自己的路径）。

### 2.3 root 进程的清理语义不能变

#163 的核心发现：root proxy 无法被用户态 `kill()`（EPERM）。当前 cleanup 依赖 pid 文件 + `ps -o comm=` 精确匹配 + sudo 脚本兜底，这套机制在迁移后**原样保留**——`Command::sidecar` spawn 拿不到 root 子进程的 `Child` handle（root 子进程由 osascript-spawned sh 启动并 disown），issue 正文里 "cleanup 变成 Rust Drop" 的想法在本阶段不成立，标记为后续（依赖 SMJobBless 类方案）。

## 3. 实施步骤

### Step 1 — 引入 tauri-plugin-shell

- `src-tauri/Cargo.toml`：加 `tauri-plugin-shell = "2"`（与 `tauri = "2"` 同 major，lock 里当前 tauri 2.11.3）。
- `src-tauri/src/lib.rs::run()`：`.plugin(tauri_plugin_opener::init())`（约 L129-130）旁边注册 `.plugin(tauri_plugin_shell::init())`。
- `src-tauri/capabilities/default.json`：**不新增 `shell:*` 权限**。纯 Rust 侧 `app.shell().sidecar()` 不经 capability 检查；前端不直接 spawn。这是本方案在安全边界上的收益点（验收标准第 3 条），不要图省事加 `shell:allow-execute`。

### Step 2 — IPC 层解析 sidecar 路径

`src-tauri/src/commands/dns.rs::set_dns_mode_enable`（约 L111）：

```rust
use tauri_plugin_shell::ShellExt;

let sidecar = app
    .shell()
    .sidecar("mhost-dns-proxy")
    .map_err(|e| /* 包装成明确 Err，禁止静默失败 */)?;
let proxy_path = sidecar.get_program().to_path_buf(); // 解析后的绝对路径
```

- `set_dns_mode_enable` 需要拿到 `app: tauri::AppHandle`（`set_dns_mode` 入口补参数，lib.rs 注册的命令签名同步）。
- **dev 模式验证点**：Tauri 2 在 dev 下同样按 `externalBin` 规则解析到 `src-tauri/bin/mhost-dns-proxy-<triple>`（不是 `target/debug/`）。Step 4 的 staging 必须落到这里。

### Step 3 — platform.rs 改为接收路径

`src-tauri/crates/mhost-dns/src/platform.rs`：

1. `enable_dns_mode` 签名改为
   `pub fn enable_dns_mode(proxy_path: &Path, dns_port: u16, original_dns: &[IpAddr]) -> Result<(), PlatformError>`
   —— 删除 L796-799 的 `current_exe()` 探测块，`validate_proxy_binary(&proxy_path)` 保留在原位（提权弹窗前的最后一次廉价校验，错误信息里的 `PROXY_BUILD_INSTR` 更新为指向新的 dev 工作流说明）。
2. `EnableScriptInputs` 增加/复用 `proxy_path` 字段（当前构建脚本时已经把路径拼进去，确认透传即可）。
3. 提权脚本内容**基本不动**：Layer 1 `[ -x "$PROXY" ]`、`"$PROXY" --listen 53 ... &`、pid 文件、readiness 轮询、trap 清理全部保留——它们解决的是 root 生命周期问题，与路径发现正交。净删除的只有 shell 脚本外层的探测逻辑。
4. 更新 `validate_proxy_binary` 的错误文案和单测；`platform.rs` 内 `#[cfg(test)]` 中引用旧签名的测试同步改。

### Step 4 — dev 工作流：beforeDevCommand + dev.sh 退休

- `src-tauri/tauri.conf.json::build` 增加：
  ```json
  "beforeDevCommand": "bash scripts/stage-proxy-sidecar-dev.sh"
  ```
- 新增 `scripts/stage-proxy-sidecar-dev.sh`（或给 `build-proxy-sidecar.sh` 加 `--dev` 参数，倾向后者少一个文件）：
  - `cargo build -p mhost-dns --bin mhost-dns-proxy`（debug，仅当前 host triple，`rustc -vV` 取 `host:` 行）
  - 拷贝到 `src-tauri/bin/mhost-dns-proxy-<triple>`
  - release 路径（beforeBuildCommand）行为不变
- `scripts/dev.sh` 退休：删除文件，`package.json` 的 `dev:full` 脚本改为直接 `pnpm tauri dev`。
- 同步更新 `doc/dev-guide.md:73-109`（写明 `current_exe()` 探测约定的那段）和 `readme.md:153,168,177` 中对 dev.sh 的引用。

### Step 5 — CI 与测试

- `.github/workflows/ci.yml` L98-117（debug sidecar staging）与 L240-273（release 打包 + `test -x` 断言）预期**不需要改**：打包链路没动。要确认的是 CI 的 debug staging 目录与 Step 2 的 dev 解析路径一致（都是 `src-tauri/bin/`）。
- `platform.rs` 单测：`build_enable_script` 系列测试改为显式传 `proxy_path`；新增一个「路径含空格/引号」用例确认 `shell_single_quote` 仍覆盖（sidecar 解析出的路径同样可能带特殊字符）。
- 新增集成级验证（手动验收即可，进验收清单）：裸 `pnpm tauri dev`（不跑 dev.sh）→ 开启 DNS 模式 → 验证 53 转发工作 + `disable` 后恢复原 DNS。

## 4. 验收标准（对齐 issue Suggested acceptance criteria）

1. 全新 clone 上**裸 `pnpm tauri dev`**（不跑 scripts/dev.sh）：DNS 模式可启用、可用、可关闭恢复。
2. `pnpm tauri build` 产物中 sidecar 仍为 `mHost.app/Contents/MacOS/mhost-dns-proxy`（CI 既有断言继续通过），codesign/notarization 在 aarch64 + x86_64 双矩阵不受影响（该链路 PR #146 已验证，本次未触碰）。
3. `capabilities/default.json` **不出现** `shell:allow-execute` 或任何 `shell:*`；sidecar 名仅出现在 `bundle.externalBin`。
4. sidecar 缺失时 `enable_dns_mode` 返回明确 `Err`（复用 `validate_proxy_binary` 的错误形状），无静默失败。
5. disable/cleanup 幂等：pid 文件 + comm 精确匹配 + sudo 脚本兜底机制不变，#163 的 EPERM 语义不回退。
6. `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`pnpm test`、`pnpm build` 全绿。

## 5. 明确不做的事（Out of scope）

| 项 | 原因 | 去向 |
|---|---|---|
| 消灭 osascript 提权（SMJobBless / SUID 安装器） | scope 大一个量级，需要安装期权限模型 | 独立 issue，#160 落地后二进制定位干净了才好做 |
| cleanup 改 Rust `Drop`（基于 Child handle） | root 子进程拿不到 handle，#163 EPERM 结论 | 同上，随 SMJobBless 一起做 |
| Windows / Linux 权限模型 | #67 依赖本 issue 的「路径由 IPC 层注入」形态，本 PR 只是铺路 | #67 |
| #158 / #159 | issue 注明 "will be made simpler by this refactor" | 本 PR 落地后单独跟 |

## 6. 风险与回滚

- **风险：dev 下 sidecar 解析路径与 staging 目录不一致** → 首要手动验证项；不一致表现为 enable 时 `validate_proxy_binary` 报错（响亮失败，非静默）。
- **风险：`app.shell().sidecar()` 在 dev 模式需要 `tauri dev` 的运行时上下文** → 解析发生在 IPC handler 内（运行时），不在 setup 期，无问题。
- **回滚**：改动集中在 `commands/dns.rs`（解析）+ `platform.rs`（签名）+ 脚本/workflow，无存储格式变更，无 manifest schema 变更，单 PR revert 即可。分支建议 `feat/160-dns-sidecar-migration`。
- **PR 纪律**：遵守 AGENTS.md 的 PR workflow——review 开始后只加新 commit，不 amend 不 force-push。

## 7. 工作量估计

- Step 1-3（Rust 主体）：约半天，含测试修改。
- Step 4（dev 工作流 + 文档）：约 1-2 小时。
- Step 5（CI 确认 + 手动验收）：约 1 小时。
- 合计：**约 1 个工作日**，适合单 PR 交付。
