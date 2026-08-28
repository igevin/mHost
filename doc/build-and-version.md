# mHost 打包编译与版本号管理

## 版本号配置位置

mHost 有三个地方写着版本号，它们各自影响不同的东西：

| 文件 | 字段 | 当前版本 | 用途 | 影响出包名 | 影响 check_update |
|-----|------|---------|------|-----------|-------------------|
| `src-tauri/tauri.conf.json` | `version` | `0.3.2` | Tauri 应用版本（出包文件名） | **是** | 否 |
| `package.json` | `version` | `0.3.2` | 前端版本，通过 `vite.config.ts` 注入为 `__APP_VERSION__` | 否 | **是** |
| `src-tauri/Cargo.toml` | `version` | `0.1.0` | Rust crate 版本 | 否 | 否 |

### `__APP_VERSION__` 的版本来源

`__APP_VERSION__` 在前端代码里充当全局常量，值来自 `package.json`（在 `vite.config.ts:13-22` 通过 `define` 注入）。它在 **Settings 页面** 有两处消费点：

| 消费点 | 作用 | 位置 |
|--------|------|------|
| **About 卡片** | 常驻显示当前版本号 | `src/pages/Settings.tsx:70` `<div className={styles.aboutVersion}>Version {__APP_VERSION__}</div>` |
| **检查更新按钮** | 与 GitHub Releases 最新 tag 比对 | `src/pages/Settings.tsx:32` `await checkUpdate(__APP_VERSION__)` → IPC → `src-tauri/src/commands/update.rs:34` `check_update(current_version)` |

数据流：

```
package.json → vite.config.ts (define) → __APP_VERSION__ → Settings.tsx
                                                │
                                                ├──► About 卡片(显示)
                                                └─► checkUpdate() → Rust check_update → GitHub Releases API
```

> - `__APP_VERSION__` 在 `src/css.d.ts:8` 通过 `declare const __APP_VERSION__: string;` 声明为 TS 全局类型。
> - 测试 (`src/pages/__tests__/Settings.test.tsx:11-12`) 通过 `globalThis.__APP_VERSION__ = "0.2.0"` stub 一个固定版本，避免快照/断言随 `package.json` 偶发变动。

因此 **`package.json` 的 version 必须与实际发版号一致**——否则 About 卡片会显示旧版本号，check_update 会误判当前版本（导致永远提示"有新版本"或永远看不到更新提示）。

## 修改版本号步骤

### 1. 修改 `src-tauri/tauri.conf.json`（必须）

影响出包文件名。

```diff
- "version": "0.3.2",
+ "version": "0.3.3",
```

### 2. 同步修改 `package.json`（必须）

影响 check_update 的版本比较。**必须与 tauri.conf.json 保持一致。**

```diff
- "version": "0.3.2",
+ "version": "0.3.3",
```

### 3. 发版时打 git tag 并推送（触发 CI 自动构建）

```bash
git tag v0.3.3
git push origin v0.3.3
```

推送 `v*` 格式的 tag 后，GitHub Actions 会自动触发 Release 构建（见下方 CI 自动构建章节）。

### 关于 Cargo.toml

工程里有 6 个 `Cargo.toml`，每个 `version` 字段都是 **Rust crate 自身的版本号**（在 workspace 内的依赖解析时使用），**不影响出包文件名，也不影响 check_update**。发版时不需要同步修改它们。具体路径如下：

```
src-tauri/Cargo.toml                           (workspace root)
src-tauri/crates/mhost-core/Cargo.toml
src-tauri/crates/mhost-hosts/Cargo.toml
src-tauri/crates/mhost-storage/Cargo.toml
src-tauri/crates/mhost-apply/Cargo.toml
src-tauri/crates/mhost-dns/Cargo.toml
```

> 如果未来要把 `mhost-*` 子 crate 发布到 crates.io，或因为依赖 API 不兼容需要打破 semver，
> 那时再为它们单独 bump version。当前（v0.3.x 阶段）保持 `0.1.0` 即可。

## 出包命名规则

Tauri 2 bundler 模板：

```
{productName}_{version}_{target-triple}
└── mHost     └── 0.3.3 └── aarch64-apple-darwin (简写 aarch64)
```

示例：`mHost_0.3.3_aarch64.dmg`

| 组成部分 | 值 | 说明 |
|---------|---|------|
| productName | `mHost` | tauri.conf.json 中的 productName |
| version | `0.3.3` | tauri.conf.json 中的 version |
| target | `aarch64` | Apple Silicon (aarch64-apple-darwin) |

## 打包编译命令

### 开发模式

```bash
pnpm tauri dev
```

### 生产构建（macOS）

```bash
# Apple Silicon (aarch64)
pnpm tauri build --target aarch64-apple-darwin

# Intel (x86_64)
pnpm tauri build --target x86_64-apple-darwin

# 当前架构（自动检测）
pnpm tauri build
```

### 构建产物位置

构建完成后，产物在 `src-tauri/target/release/bundle/` 目录下：

- `macos/` — `.app` 应用包
- `dmg/` — `.dmg` 安装镜像

## CI 自动构建（GitHub Actions）

### 触发条件

推送 `v*` 格式的 git tag 时自动触发，例如：

```bash
git tag v0.3.3
git push origin v0.3.3
```

配置文件：`.github/workflows/release.yml`

### 构建矩阵

push tag 后会**并行**触发两个构建 job（同一份 `tauri-action` 配置 + 两组 `--target`）：

| Job 步骤 | Runner | 构建参数（`args`） | 产物 |
|---------|--------|------------------|------|
| `build-arm` | `macos-latest`（M1） | `--target aarch64-apple-darwin` | `.dmg` / `.app` |
| `build-x86_64` | `macos-latest`（M1） | `--target x86_64-apple-darwin` | `.dmg` / `.app` |

> 两个 job 的 runner **都是 `macos-latest`**（当前指向 Apple Silicon / M1 镜像），
> 区别仅在 `--target` 参数。M1 runner 上做交叉编译即可产出 x86_64 安装包，
> 因此无需单独的 Intel runner。

两个 job 的产物会自动上传到**同一个 GitHub Release** 中。

### Release 策略

- **Draft 模式**：Release 先创建为草稿（draft），手动确认后再正式发布
- **fail-fast: false**：某个架构构建失败不影响另一个
- **无需额外配置**：使用 GitHub Actions 自带的 `GITHUB_TOKEN`

### 发版完整流程

```bash
# 1. 确保在 master 分支且工作区干净
git checkout master
git pull origin master

# 2. 跑全量测试确认发版就绪
cd src-tauri && cargo test --workspace --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all -- --check && cd ..
pnpm test && pnpm build

# 3. 修改版本号（两个文件必须同步）
#    - src-tauri/tauri.conf.json: "version": "0.3.3"
#    - package.json: "version": "0.3.3"

# 4. 提交并推送
git add src-tauri/tauri.conf.json package.json
git commit -m "chore: bump version to 0.3.3"
git push

# 5. 打 tag 并推送（触发 CI 构建）
git tag v0.3.3
git push origin v0.3.3

# 6. 等待 CI 完成，在 GitHub Releases 页面检查 draft release
# 7. 确认无误后，手动发布 release
```

### 发版前检查清单

- [ ] `cargo test --workspace --all-features` 全部通过（在 `src-tauri/` 下运行，显式覆盖全部成员 crate；`--workspace` 不可省略 — 裸 `cargo test` 默认只跑当前根 crate，会漏掉 `mhost-*` 子 crate 的测试）
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` 无 warning
- [ ] `cargo fmt --all -- --check` 干净
- [ ] `pnpm test` 全部通过
- [ ] `pnpm build` 干净
- [ ] 没有 open bug issue
- [ ] `tauri.conf.json` 和 `package.json` 的 version 已同步修改
- [ ] git tag 号与 version 一致（`v0.3.3` 对应 `0.3.3`）
