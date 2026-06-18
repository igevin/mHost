# mHost 技术路线：Rust + Tauri

创建日期：2026-06-18

本文基于当前产品方向和技术讨论整理。结论是：mHost 采用 Rust + Tauri 是可行且匹配的技术路线；默认不采用本地 DNS 模式，而是以“安全、快速、可回滚地修改系统 hosts 文件”为主路径。

## 1. 技术结论

mHost 追求跨平台、极致性能、低后台开销和低打扰体验，因此更适合定位为“hosts 配置与切换工具”，而不是“常驻网络解析服务”。

推荐技术路线：

- 桌面应用：Tauri。
- 核心逻辑：Rust。
- 前端界面：Web 前端技术，优先选择轻量方案。
- 默认生效方式：直接修改系统 hosts 文件。
- 本地 DNS 模式：不作为主路径，不纳入 MVP。
- 权限策略：必要时请求权限，但要做到说明清楚、次数克制、写入可回滚。

这一决策会让 mHost 的核心能力更聚焦：Profile 管理、规则解析、冲突检测、安全写入、备份恢复、快速切换、远程规则和广告屏蔽规则管理。

## 2. 为什么不采用本地 DNS

本地 DNS 模式可以绕开一部分直接写 hosts 的心智问题，但它会让 mHost 从轻量配置工具变成系统网络链路的一部分。

主要问题：

- 需要常驻 DNS 服务，增加后台进程和运行状态管理。
- 需要修改系统 DNS 配置，仍然可能涉及权限和系统安全提示。
- 需要处理端口占用、上游 DNS、VPN、企业网络、安全软件、休眠唤醒等问题。
- 服务异常可能影响全局域名解析。
- 用户难以理解“为什么一个 hosts 工具接管了 DNS”。
- 跨平台差异比 hosts 文件写入更复杂。

从性能和稳定性看，直接修改 hosts 文件更符合 mHost 的目标。写入完成后，解析由系统负责，mHost 不在请求路径上；应用退出后配置仍然生效，不需要为了规则生效长期运行后台解析服务。

## 3. 默认 hosts 写入方案

默认方案是把多个 Profile、远程规则、广告屏蔽规则和白名单合并后，生成最终 hosts 内容，并安全写入系统 hosts 文件。

核心原则：

- 写入前必须解析、校验和生成 diff。
- 写入前必须创建备份。
- 写入过程应尽量原子化，避免写入半截内容。
- 写入失败不得破坏原 hosts 文件。
- 写入成功后应验证目标规则是否存在。
- 必要时刷新 DNS 缓存或提示用户刷新。
- 检测到外部修改时，不应静默覆盖。

建议采用“托管区块”方式，不完全接管用户原有 hosts 内容。

示例：

```txt
# ---- mHost start ----
# managed by mHost, do not edit manually
127.0.0.1 api.example.com
0.0.0.0 ads.example.com
# ---- mHost end ----
```

这样可以保留用户手动维护的内容，也能让 mHost 清楚地替换自己管理的部分。

## 4. 架构分层

建议把 mHost 拆成四层：UI 层、应用服务层、核心引擎层、系统适配层。

| 层级 | 技术 | 职责 |
| --- | --- | --- |
| UI 层 | Tauri + Web 前端 | 页面展示、编辑交互、托盘入口、设置界面 |
| 应用服务层 | Rust Tauri commands | 连接 UI 与核心能力，处理用户操作、任务编排、状态返回 |
| 核心引擎层 | Rust crates | Profile、规则解析、合并、冲突检测、导入导出、备份策略 |
| 系统适配层 | Rust + 平台 API | hosts 路径、权限写入、DNS 缓存刷新、开机启动、托盘、日志 |

前端不应承载核心规则逻辑。规则解析、合并、校验、写入和回滚都放在 Rust 侧，前端只接收结构化结果并展示。

## 5. 推荐工程结构

可按 workspace 拆分 Rust crate，避免所有逻辑堆在 Tauri 主进程中。

```txt
mhost/
  src-tauri/
    src/
      main.rs
      commands/
      state/
      platform/
    crates/
      mhost-core/
      mhost-hosts/
      mhost-storage/
      mhost-apply/
      mhost-adblock/
  src/
    app/
    pages/
    components/
    stores/
```

建议职责：

| 模块 | 职责 |
| --- | --- |
| `mhost-core` | 核心数据模型、错误类型、通用结果结构 |
| `mhost-hosts` | hosts 文本解析、格式化、语法校验、IPv4/IPv6/域名校验 |
| `mhost-storage` | Profile、规则源、设置、备份记录的本地存储 |
| `mhost-apply` | 规则合并、优先级计算、diff、写入计划、回滚 |
| `mhost-adblock` | 广告屏蔽规则导入、白名单、远程规则缓存 |
| `platform` | macOS、Windows、Linux 的 hosts 路径、权限、DNS 缓存刷新 |
| `commands` | Tauri command，对 UI 暴露稳定接口 |

## 6. 核心数据模型

第一版建议保持模型克制，避免过早加入团队同步、本地 API 和复杂权限系统。

```rust
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub protected: bool,
    pub tags: Vec<String>,
    pub rules: Vec<HostRule>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct HostRule {
    pub id: RuleId,
    pub ip: IpAddr,
    pub domains: Vec<String>,
    pub enabled: bool,
    pub comment: Option<String>,
    pub source: RuleSource,
}

pub enum RuleSource {
    Manual,
    Remote { source_id: SourceId },
    AdBlock { source_id: SourceId },
}
```

写入系统前不应直接拿 Profile 拼字符串，而应生成中间结果。

```rust
pub struct ApplyPlan {
    pub rules: Vec<ResolvedRule>,
    pub conflicts: Vec<RuleConflict>,
    pub diff: HostsDiff,
    pub backup_required: bool,
}
```

这样 UI 可以先展示即将变更的内容，再由用户确认应用。

## 7. 规则合并策略

推荐默认优先级：

```txt
白名单 > 手动 Profile > 远程 Profile > 广告屏蔽规则
```

原因：

- 白名单用于纠正误杀，优先级最高。
- 手动 Profile 通常代表用户当前工作环境，应高于订阅规则。
- 远程 Profile 适合团队或外部配置，但不应覆盖用户明确选择。
- 广告屏蔽规则数量大，误伤概率更高，应处于较低优先级。

冲突处理建议：

- 同一域名映射到相同 IP：合并为一条。
- 同一域名映射到不同 IP：标记冲突。
- 手动规则与广告屏蔽冲突：手动规则优先。
- 白名单命中的域名：不写入屏蔽规则。
- 受保护 Profile 与当前规则冲突：显示更强提示。

## 8. 权限与写入策略

直接修改 hosts 文件的核心挑战是权限。建议把权限问题设计成明确、可解释的产品流程，而不是隐藏在后台。

### macOS

可选路径：

- 初期：通过系统授权执行受限写入。
- 后期：引入 privileged helper，减少重复授权。

需要处理：

- `/etc/hosts` 实际路径。
- 写入前备份。
- 外部变更检测。
- DNS 缓存刷新。
- 应用签名和 notarization。

### Windows

可选路径：

- 初期：以管理员权限执行写入。
- 后期：考虑 Windows service 或一次性提权辅助进程。

需要处理：

- `C:\Windows\System32\drivers\etc\hosts`。
- UAC 提示。
- 文件占用或安全软件拦截。
- DNS 缓存刷新。
- 安装包签名。

### Linux

Linux 暂不作为首要平台，但架构上不要阻断。

需要处理：

- `/etc/hosts`。
- `sudo` 或 polkit。
- 不同发行版 DNS 缓存刷新差异。

## 9. 安全写入流程

建议把应用流程固定为以下步骤：

1. 读取当前系统 hosts。
2. 检测 mHost 托管区块。
3. 检测外部变更。
4. 合并启用的 Profile、远程规则、广告屏蔽规则和白名单。
5. 校验规则格式。
6. 检测冲突。
7. 生成 ApplyPlan 和 diff。
8. 用户确认。
9. 创建备份。
10. 写入临时文件。
11. 校验临时文件内容。
12. 替换系统 hosts。
13. 刷新 DNS 缓存。
14. 验证写入结果。
15. 记录应用历史。

写入失败时：

- 不删除备份。
- 不更新当前启用状态。
- 保留错误日志。
- 给出明确失败原因。
- 提供恢复上一版入口。

## 10. 性能原则

mHost 的性能目标不是只追求单次解析速度，而是整体上减少后台开销和用户等待。

建议原则：

- 不默认启动本地 DNS、代理或常驻网络服务。
- app 退出后，已应用的 hosts 规则仍然有效。
- 规则解析和合并在 Rust 侧完成。
- 大规则列表采用增量解析和缓存。
- 远程规则刷新不阻塞主界面。
- UI 虚拟化展示大列表。
- 保存、备份和写入操作异步执行。
- 托盘模式只保留必要状态，不做高频轮询。

可设定初步性能目标：

| 场景 | 目标 |
| --- | --- |
| 冷启动 | 主窗口尽快可交互，规则库延迟加载 |
| Profile 切换 | 常规规则量下 1 秒内完成写入计划生成 |
| 大规则文件 | 10 万行 hosts 规则可解析、可搜索、不卡死 UI |
| 后台资源 | 无本地 DNS 服务，无高频网络请求，无无意义轮询 |
| 写入安全 | 任意失败点都可恢复到写入前状态 |

具体数值需要在原型阶段用真实规则文件压测后调整。

## 11. 前端选择建议

Tauri 可以搭配 React、Vue、Svelte 或 Solid。mHost 的 UI 复杂度中等，核心瓶颈不在前端框架。

建议优先级：

- 如果团队熟悉 React：React + Vite 可直接采用。
- 如果追求更轻运行时：Svelte 或 Solid 更贴近轻量目标。
- 如果希望生态和组件更成熟：React 更稳妥。

无论选择哪个框架，都应遵守：

- UI 状态不等于核心状态，核心状态以 Rust 侧存储为准。
- Tauri command 接口保持稳定。
- 大文本编辑器选择成熟方案，例如 CodeMirror。
- 大规则列表必须虚拟滚动。
- 不在前端重复实现 hosts parser。

## 12. 本地存储

建议采用文件型本地存储，保持透明、可备份、易迁移。

可选结构：

```txt
mHost/
  manifest.json
  profiles/
    {profile_id}.json
  remote_sources/
    {source_id}.json
  cache/
    remote/
  backups/
    hosts-{timestamp}.bak
  logs/
    app.log
  settings.json
```

设计原则：

- 用户配置与缓存分离。
- 备份文件可直接查看。
- 数据格式版本化。
- 导出文件不包含敏感日志。
- 远程规则缓存失败时不影响已有配置。

## 13. 广告屏蔽技术策略

广告屏蔽第一版建议只支持 hosts 格式规则，不支持 EasyList 等浏览器过滤规则。

原因：

- hosts 格式和 mHost 主能力一致。
- 实现复杂度低。
- 规则应用路径统一。
- 不需要引入代理或浏览器扩展。

建议支持：

- 远程 hosts 规则源。
- 手动屏蔽域名。
- 白名单。
- 规则来源展示。
- 规则更新。
- 与 Profile 合并时的优先级控制。

暂不建议支持：

- CSS 隐藏规则。
- URL 路径级拦截。
- 请求级命中统计。
- 浏览器扩展。

这些能力会把 mHost 带向广告拦截器，而不是 hosts 管理器。

## 14. 诊断能力

诊断能力应围绕“为什么规则没有生效”设计。

建议包含：

- 当前 hosts 文件是否包含 mHost 托管区块。
- 当前启用 Profile 是否已写入。
- 指定域名最终应该解析到哪个 IP。
- 是否存在冲突规则。
- 是否被白名单覆盖。
- 系统 hosts 是否被外部程序修改。
- DNS 缓存是否可能未刷新。
- 写入权限是否不足。

后期可增加：

- hostname resolver。
- 诊断报告导出。
- 常见问题提示。
- 一键恢复最近备份。

## 15. 阶段实施

### 阶段 0：技术原型

目标是验证 Rust + Tauri + hosts 写入的闭环。

范围：

- Tauri 应用壳。
- Rust hosts parser。
- 读取系统 hosts。
- 生成托管区块。
- 写入前备份。
- 安全写入原型。
- macOS 或 Windows 单平台验证。

不做：

- 本地 DNS。
- 广告屏蔽完整规则源。
- 团队同步。
- 本地 API。

### 阶段 1：MVP

目标是完成可用的多 Profile 切换。

范围：

- Profile CRUD。
- hosts 文本编辑。
- 单 Profile 启用。
- 规则校验。
- 冲突检测基础版。
- 应用前 diff。
- 写入、备份、恢复。
- macOS + Windows 支持。

### 阶段 2：长期使用体验

目标是提升编辑效率和安全感。

范围：

- 语法高亮。
- 查找替换。
- 规则启停。
- 导入导出。
- 回收站。
- 修改历史。
- 托盘快速切换。
- 外部变更检测。

### 阶段 3：远程规则与广告屏蔽

目标是支持广告屏蔽与远程 hosts 订阅。

范围：

- 远程 hosts URL。
- 手动刷新。
- 启动时刷新。
- 定时刷新。
- 本地缓存。
- 广告屏蔽开关。
- 白名单。
- 规则来源展示。

### 阶段 4：诊断与扩展

目标是完善复杂问题排查，并为高级用户提供扩展入口。

范围：

- 域名解析诊断。
- DNS 缓存刷新。
- 诊断报告。
- 应用后命令。
- CLI。
- 本地 API。

本地 API 和 CLI 默认不应影响普通用户体验，可以作为高级设置关闭。

## 16. GitHub Actions 构建发布

Rust + Tauri 的跨平台构建适合交给 GitHub Actions 承担。它可以在不同系统 runner 上分别产出 macOS、Windows 和 Linux 包，减少本地构建环境差异，也方便后续接入自动发布、校验文件和 Tauri updater。

推荐把构建发布拆成两个 workflow：

```txt
.github/
  workflows/
    ci.yml
    release.yml
```

### 16.1 CI 检查

`ci.yml` 面向 pull request 和 main 分支提交，目标是保证主分支随时可构建。

建议触发：

```yaml
on:
  pull_request:
  push:
    branches:
      - main
```

建议检查内容：

- Rust format。
- Rust clippy。
- Rust 单元测试。
- 前端 lint。
- 前端 build。
- Tauri build smoke test。

CI 阶段不需要产出正式安装包，也不需要签名。它只负责发现代码质量、依赖和基础构建问题。

### 16.2 Release 构建

`release.yml` 面向 tag 或手动触发，目标是产出用户可安装的程序包。

建议触发：

```yaml
on:
  workflow_dispatch:
  push:
    tags:
      - "v*"
```

建议平台：

| 平台 | runner | 产物 |
| --- | --- | --- |
| macOS | `macos-latest` | `.app`、`.dmg` |
| Windows | `windows-latest` | `.exe`、`.msi` 或 `.nsis` |
| Linux | `ubuntu-latest` | `.AppImage`、`.deb` |

mHost 第一阶段可以先发布 macOS 和 Windows。Linux 可以保留构建配置，但不一定作为正式首发平台。

### 16.3 签名与公证

早期内部测试包可以不签名，但正式发布前必须规划签名链路。

macOS 需要关注：

- Apple Developer 账号。
- Developer ID Application 证书。
- notarization。
- stapling。
- Tauri 打包配置。

Windows 需要关注：

- 代码签名证书。
- 安装包签名。
- SmartScreen 信任积累。
- 安全软件误报风险。

如果 mHost 涉及 hosts 写入和权限提升，签名会直接影响用户信任和系统提示体验。发布流程不应只验证 dev 模式，必须验证安装包中的 hosts 写入、备份、回滚和权限提示。

### 16.4 缓存与构建效率

Rust + Tauri 构建耗时较长，建议在 GitHub Actions 中加入缓存。

建议缓存：

- Cargo registry。
- Cargo git index。
- Cargo target。
- pnpm、npm 或 yarn 缓存。
- 前端构建缓存。

依赖管理建议：

- 锁定 `Cargo.lock`。
- 锁定前端 lockfile。
- CI 中使用固定 Node 和 Rust 版本。
- 发布构建尽量使用稳定版本 runner。

### 16.5 推荐实施顺序

第一步先建立 `ci.yml`，保证 Rust 和前端代码能稳定检查。

第二步建立 `release.yml`，通过 `workflow_dispatch` 手动构建 macOS 和 Windows 包。

第三步用 `v*` tag 触发正式 release，把构建产物上传到 GitHub Release。

第四步补齐 macOS notarization 和 Windows code signing。

第五步再考虑 Tauri updater、自动 changelog、checksum 和 nightly 构建。

### 16.6 对 mHost 的约束

构建发布流程必须覆盖 mHost 的关键系统能力，而不只是打包成功。

发布前至少验证：

- 应用能读取系统 hosts。
- 应用能生成托管区块。
- 写入前备份可用。
- 写入失败不会破坏原 hosts。
- 回滚可以恢复上一版。
- macOS 和 Windows 权限提示符合预期。
- 打包后的应用路径、权限和资源文件都正确。

## 17. README 需要同步调整

当前 README 仍强调“免超级用户授权”和“优先采用不直接修改系统 Hosts 文件的方案”。基于本次技术决策，建议后续调整为更准确的表述：

```md
mHost 默认采用安全、可回滚的系统 hosts 写入方式，尽量减少权限请求，并在需要权限时清楚说明原因。应用不会默认启动本地 DNS 或代理服务，以降低后台开销并保持系统行为可预期。
```

对应 Roadmap 中的“支持无需超级用户授权的默认使用模式”也建议调整为：

```md
- 支持安全、可回滚的 hosts 写入流程。
- 尽量减少重复权限请求。
- 提供清晰的权限说明、备份和恢复能力。
```

## 18. 待决策问题

1. MVP 首发平台是 macOS 优先、Windows 优先，还是两端同步。
2. 前端框架选择 React、Svelte、Vue 还是 Solid。
3. macOS 权限方案第一版是否接受每次写入授权，还是直接设计 privileged helper。
4. Windows 第一版是否要求管理员启动，还是通过辅助进程写入。
5. Profile 是否允许多选叠加，还是第一版只允许单 Profile 启用。
6. 远程规则是否纳入 MVP，还是放到阶段 3。
7. 广告屏蔽是否只支持 hosts 格式规则。
8. GitHub Actions 首期是否只构建 macOS 和 Windows。
9. 正式发布前是否必须完成 macOS notarization 和 Windows code signing。

## 19. 当前建议

第一版不要追求“看起来最完整”，而要先把系统 hosts 写入这条路径做到稳定、清楚、可恢复。

最小技术闭环应是：

- Rust 解析和校验 hosts。
- Tauri 提供跨平台桌面壳。
- Profile 生成托管区块。
- 写入前展示 diff。
- 自动备份。
- 原子写入。
- 失败回滚。
- 写入后验证。

这条路线更符合 mHost 的性能目标，也更容易在 macOS 和 Windows 上形成一致体验。
