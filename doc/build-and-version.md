# mHost 打包编译与版本号管理

## 版本号配置位置

出包文件名中的版本号由 **`src-tauri/tauri.conf.json`** 的 `version` 字段决定：

```json
{
  "productName": "mHost",
  "version": "0.1.0",   // ← 改这里
  ...
}
```

当前配置路径：`src-tauri/tauri.conf.json:4`

## 出包命名规则

Tauri 2 bundler 模板：

```
{productName}_{version}_{target-triple}
└── mHost     └── 0.1.0 └── aarch64-apple-darwin (简写 aarch64)
```

示例：`mHost_0.1.0_aarch64`

| 组成部分 | 值 | 说明 |
|---------|---|------|
| productName | `mHost` | tauri.conf.json 中的 productName |
| version | `0.1.0` | tauri.conf.json 中的 version |
| target | `aarch64` | Apple Silicon (aarch64-apple-darwin) |

## 修改版本号步骤

### 1. 修改主版本号（必须）

编辑 `src-tauri/tauri.conf.json`：

```diff
- "version": "0.1.0",
+ "version": "0.2.0",
```

### 2. 同步修改 package.json（建议）

编辑根目录 `package.json`：

```diff
- "version": "0.1.0",
+ "version": "0.2.0",
```

### 3. 发版时打 git tag 并推送（触发 CI 自动构建）

```bash
git tag v0.2.0
git push origin v0.2.0
```

推送 `v*` 格式的 tag 后，GitHub Actions 会自动触发 Release 构建（见下方 CI 自动构建章节）。

## 注意事项

- **不要改 Cargo.toml 的 version 来控制出包名称**：工作空间根 `src-tauri/Cargo.toml` 及各 crate（mhost-core、mhost-apply 等）的 `Cargo.toml` 也有 version 字段，但那是 Rust crate 的版本号，不影响 Tauri 出包文件名。
- **只需改 `src-tauri/tauri.conf.json` 的 version**，build 出的镜像名称就会随之变化。

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
git tag v0.2.0
git push origin v0.2.0
```

配置文件：`.github/workflows/release.yml`

### 构建矩阵

push tag 后会**并行**触发两个构建 job：

| Job | Runner | Target | 产物 |
|-----|--------|--------|------|
| macOS ARM | `macos-latest` (M1) | `aarch64-apple-darwin` | `.dmg` / `.app` |
| macOS Intel | `macos-latest` (M1) | `x86_64-apple-darwin` | `.dmg` / `.app` |

两个 job 的产物会自动上传到**同一个 GitHub Release** 中。

### Release 策略

- **Draft 模式**：Release 先创建为草稿（draft），手动确认后再正式发布
- **fail-fast: false**：某个架构构建失败不影响另一个
- **无需额外配置**：使用 GitHub Actions 自带的 `GITHUB_TOKEN`

### 发版完整流程

```bash
# 1. 修改版本号（tauri.conf.json + package.json）
# 2. 提交并推送
git add .
git commit -m "chore: bump version to 0.2.0"
git push

# 3. 打 tag 并推送（触发 CI 构建）
git tag v0.2.0
git push origin v0.2.0

# 4. 等待 CI 完成，在 GitHub Releases 页面检查 draft release
# 5. 确认无误后，手动发布 release
```

## 项目版本号一览

| 文件 | 字段 | 用途 | 是否影响出包名 |
|-----|------|------|-------------|
| `src-tauri/tauri.conf.json` | `version` | Tauri 应用版本 | **是（主要）** |
| `package.json` | `version` | 前端 npm 包版本 | 否，但建议同步 |
| `src-tauri/Cargo.toml` | `version` | Rust crate 版本 | 否 |
| `src-tauri/crates/*/Cargo.toml` | `version` | 子 crate 版本 | 否 |
