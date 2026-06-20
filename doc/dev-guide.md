# mHost 开发指南

## 环境要求

| 组件 | 最低版本 | 当前验证版本 |
|------|----------|-------------|
| Rust | 1.78+ | 1.96.0 |
| Node.js | 20+ LTS | v26.3.0 |
| pnpm | 9+ | 10.29.3 |

确认命令：

```bash
rustc --version
node --version
pnpm --version
```

---

## 项目结构

```
mHost/
  src-tauri/                        # Rust 后端（Tauri v2）
    Cargo.toml                      # Workspace 根配置
    tauri.conf.json                  # Tauri 应用配置
    src/
      main.rs                       # 二进制入口
      lib.rs                        # Tauri Builder + command 注册
      commands/                     # 前后端接口
        profile.rs                  # Profile CRUD + enable/disable
        apply.rs                    # Apply plan + apply hosts + rollback
      state/mod.rs                  # AppState
      platform/                     # 平台适配
        mod.rs
        macos.rs
    crates/
      mhost-core/                   # 核心数据模型 + 错误类型
      mhost-hosts/                  # hosts 解析 / 格式化 / 校验
      mhost-storage/                # 本地持久化存储
      mhost-apply/                  # 规则合并 / diff / 系统写入
  src/                              # React 前端
    main.tsx                        # React 入口
    App.tsx                         # 根组件 + 路由
    pages/                          # 页面
      ProfileList.tsx                # Profile 列表
      ProfileEdit.tsx                # Profile 编辑
      Settings.tsx                  # 设置
    components/Layout.tsx            # 侧边栏 + 主内容区布局
    stores/profiles.ts               # Jotai 状态管理
    types/index.ts                  # TypeScript 类型（与 Rust 对应）
    lib/tauri.ts                    # Tauri API 封装
  package.json                      # 前端依赖
  vite.config.ts                    # Vite 配置
  tsconfig.json                     # TypeScript 配置
  .github/workflows/ci.yml          # CI 流水线
```

---

## 常用命令

### 安装依赖

```bash
pnpm install
```

### 开发模式（推荐日常使用）

```bash
pnpm tauri dev
```

同时启动 Vite 热更新（http://localhost:1420）和 Rust 后端，弹出桌面窗口。前端改代码自动刷新，Rust 改代码自动重编译。

### 运行测试

```bash
cd src-tauri
cargo test --workspace              # 全部 122 个测试
cargo test -p mhost-core           # 只跑核心模型
cargo test -p mhost-hosts          # 只跑解析器
cargo test -p mhost-storage        # 只跑存储层
cargo test -p mhost-apply          # 只跑合并引擎 + 写入
```

### 代码质量检查

```bash
cd src-tauri
cargo fmt --check                  # 格式检查（不修改文件）
cargo fmt                           # 自动格式化
cargo clippy --all-targets --all-features -- -D warnings  # lint 检查
```

### 编译

```bash
cd src-tauri
cargo build                         # debug 编译
cargo build --release               # release 编译
```

### 前端构建

```bash
pnpm build                          # 输出到 dist/
```

### 构建生产安装包

```bash
pnpm tauri build                    # 产出 .dmg 和 .app
```

产物路径：`src-tauri/target/release/bundle/macos/`

---

## 验证清单

阶段 0 验收标准：

- [x] `cargo test --workspace` 全部通过（122/122）
- [x] `cargo fmt --check` 通过
- [x] `cargo clippy --all-targets --all-features -- -D warnings` 通过
- [x] `pnpm build` 通过
- [x] `pnpm tauri dev` 正常启动桌面窗口
- [x] hosts 解析器支持标准语法、IPv6、错误标记
- [x] 规则合并支持冲突检测（同域名不同 IP）
- [x] 系统写入支持托管区块、备份、回滚
- [x] 前端应用壳可展示 Profile 列表和编辑页
- [x] 集成测试验证完整应用流程

---

## Workspace Crates 说明

| Crate | 职责 | 测试数 |
|-------|------|--------|
| mhost-core | ID 类型、Profile、HostRule、RuleSource、ApplyPlan、错误类型 | 20 |
| mhost-hosts | hosts 文本解析、格式化、域名校验、托管区块识别 | 25 |
| mhost-storage | FileStorage、Manifest、原子写入、备份管理 | 20 |
| mhost-apply | 规则合并、冲突检测、Diff 生成、HostsWriter、集成测试 | 57 |

---

## 分支说明

| 分支 | 用途 |
|------|------|
| `master` | 主分支，存放稳定版本 |
| `feature/phase-0` | 阶段 0 开发分支（当前） |

---

## 存储路径

macOS 下数据存储在：

```
~/Library/Application Support/mHost/
  manifest.json          # 数据格式版本
  profiles/              # Profile JSON 文件
    {uuid}.json
  backups/               # hosts 备份
    hosts-{timestamp}.bak
  settings.json
```

系统 hosts 路径：`/etc/hosts`

托管区块标记：

```
# ---- mHost start ----
127.0.0.1 example.com
# ---- mHost end ----
```
