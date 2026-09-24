# mHost

> A lightweight, fast, low-footprint hosts & local DNS manager for desktop — built with Tauri 2 + Rust. *Cross-platform Hosts management for dev/staging/production switching and ad blocking.*

一个轻量、快速、低打扰的跨平台 Hosts 管理应用，面向开发、测试、生产环境切换以及广告屏蔽场景。

mHost 的目标不是简单地提供一个 `/etc/hosts` 编辑器，而是把常见的域名解析切换、环境隔离、广告拦截规则管理做成一个更安全、更易用的桌面工具。它优先考虑性能、稳定性和用户体验，尽可能减少后台开销，并通过安全的写入、备份和回滚机制降低操作风险。

## 功能概览

- 多 Profile 管理：为开发、测试、预发、生产等场景创建不同的 Hosts 配置。
- 一键切换环境：快速启用指定 Profile，减少手动改 hosts 文件带来的错误。
- 跨平台支持：面向 macOS 和 Windows 用户设计，后续可扩展到更多桌面系统。
- 广告屏蔽：支持维护广告域名规则，用于屏蔽常见广告、追踪和干扰性请求。
- 低资源占用：应用保持轻量运行，降低 CPU、内存和后台进程开销。
- 安全写入：修改系统 Hosts 文件前自动备份，写入失败可回滚，减少误操作风险。
- 安静不打扰：减少弹窗、权限请求和不必要通知，让工具稳定工作在后台。

## 使用场景

### 开发环境切换

开发者经常需要在本地、测试、预发和生产环境之间切换同一组域名。例如：

```txt
127.0.0.1 api.example.com
192.168.1.20 admin.example.com
10.0.0.5 static.example.com
```

使用 mHost 后，可以把这些配置保存为独立 Profile，例如：

- `Development`
- `Testing`
- `Staging`
- `Production`

切换 Profile 后，应用会让当前环境的域名解析立即生效，避免反复手动编辑 Hosts 文件。

### 测试环境验证

测试人员可以为不同测试环境准备独立配置，快速验证接口、静态资源、后台管理系统或灰度环境是否正常工作。每个 Profile 都可以保存自己的域名映射，降低配置混乱带来的误测风险。

### 生产环境保护

生产 Profile 可以保持干净、只读或受保护状态，防止开发环境配置误留在线上访问场景中。对于需要频繁切换环境的团队，这可以减少“本地配置影响真实访问”的问题。

### 广告与追踪屏蔽

mHost 可以维护广告屏蔽规则，把广告域名、追踪域名或不希望访问的域名指向本地无效地址，从而减少广告加载、统计请求和页面干扰。

示例：

```txt
0.0.0.0 ads.example.com
0.0.0.0 tracker.example.com
0.0.0.0 telemetry.example.com
```

## 为什么做 mHost

传统 Hosts 管理方式通常依赖手动编辑系统文件，存在几个明显问题：

- 操作繁琐，切换环境效率低。
- 容易改错、漏改或忘记恢复。
- 经常需要管理员权限，带来额外风险。
- 多环境配置难以复用和管理。
- 广告屏蔽规则和开发配置混在一起，不易维护。

mHost 希望把这些问题收敛到一个简单、稳定的桌面应用中：开发者可以专注写代码，测试人员可以快速验证环境，普通用户也可以使用广告屏蔽能力，而不需要理解系统 Hosts 文件的细节。

## 设计原则

### 快速

Profile 切换应尽可能即时生效。应用应避免复杂的后台任务和沉重的运行时开销，让环境切换成为一个低成本操作。

### 轻量

mHost 不应成为另一个占用大量资源的常驻应用。它应该保持较小的内存占用、较低的 CPU 使用率，并尽量减少系统托盘、后台服务和网络请求带来的负担。

### 安全

修改系统 Hosts 文件涉及权限，mHost 应在必要时请求授权，但做到说明清楚、次数克制。每次写入前自动备份，写入失败可回滚，让用户清楚知道改了什么、如何恢复。

### 不打扰

工具型应用应该在用户需要时出现，在用户不需要时安静运行。mHost 会尽量减少弹窗、强提醒和不必要的确认步骤。

### 可理解

Hosts 配置、Profile 状态、启用规则和广告屏蔽规则都应清楚展示。用户需要知道当前启用了什么配置，以及它会影响哪些域名。

## 核心能力

### Profile

Profile 是 mHost 的核心概念。每个 Profile 保存一组独立的域名解析规则，可以用于不同环境。

一个 Profile 可以包含：

- Profile 名称
- 域名映射规则
- 启用状态
- 备注说明
- 分组标签
- 创建和更新时间

示例：

```txt
# Development
127.0.0.1 api.example.com
127.0.0.1 web.example.com

# Testing
192.168.10.12 api.example.com
192.168.10.13 web.example.com
```

### 广告屏蔽规则

广告屏蔽可以作为独立 Profile，也可以作为全局规则启用。这样开发环境配置和广告屏蔽配置不会互相污染。

推荐支持：

- 手动添加屏蔽域名
- 导入规则列表
- 启用或暂停广告屏蔽
- 查看命中规则
- 为特定域名设置白名单

## 两种模式

mHost 同时支持两种域名解析管理方式，互不冲突：

### Hosts 模式（默认）

通过直接编辑 `/etc/hosts` 切换域名映射。改动立刻生效，**不需要 root**（首次授权后系统会记住）。

适用：开发、测试、生产环境切换，长期稳定的内网域名。

### DNS 模式（v0.2+）

通过本地 DNS server（默认监听 `127.0.0.1:53`）拦截域名，未命中规则则转发到上游 DNS。**支持 suffix 匹配**——一条 `example.com` 规则屏蔽所有 `*.example.com` 子域名。

适用：广告屏蔽、追踪域名拦截、灰名单过滤。

**使用方式**：

1. 创建 DNS 模式 Profile（Settings → DNS Profiles → New），添加规则
2. Settings → DNS Mode → Enable
3. 系统会提示输入管理员密码（macOS 上 `mhost-dns-proxy` 需要 root 转发 53 端口）
4. 浏览器 / 应用的 DNS 查询现在走 mHost

**与 Hosts 模式共存**：两个模式可以同时启用。Hosts 模式写 `/etc/hosts`；DNS 模式监听 53 端口拦截查询。规则按 Profile 模式区分存储。

**已知限制**：

- macOS 优先；Windows 仍在适配（详见 #67 进展）
- 与某些 VPN / Clash / Surge 等代理软件可能冲突（取决于代理软件的 DNS 处理方式）
- 启用/禁用需要一次管理员授权

## 计划支持的平台

| 平台 | DNS 模式 | Hosts 模式 | 说明 |
| --- | --- | --- | --- |
| macOS | ✅ v0.2 | ✅ v0.1 | 完整支持，`mhost-dns-proxy` 以 root 转发 53→1053 |
| Windows | 🚧 规划中 | ✅ v0.1 | DNS 模式需 Windows 服务化（详见 #67） |
| Linux | 🚧 规划中 | ✅ v0.1 | 需用户态 DNS 转发方案 |

## 技术栈

mHost 采用 **Tauri 2** 构建：Rust 实现核心逻辑，Web 前端负责界面，兼顾安装包体积、运行性能和系统集成能力。

- **后端核心**：Rust workspace（`mhost-core` / `mhost-hosts` / `mhost-storage` / `mhost-apply` / `mhost-dns`），解析、合并、校验、写入、回滚全部在 Rust 侧完成，通过强类型 IPC 与前端通信，前端不承载规则逻辑。
- **DNS 服务**：内置本地 DNS server（`mhost-dns` crate）+ 独立 root 权限转发进程 `mhost-dns-proxy`，规则匹配使用 reversed-domain trie 优化热路径。
- **前端**：React 18 + TypeScript + Jotai + Vite，测试使用 Vitest。
- **工程化**：GitHub Actions CI（macOS 双架构 aarch64 / x86_64 矩阵），强制 `cargo fmt` / `clippy -D warnings` / 全量测试；release 流程自动化产出双架构安装包。

## 项目状态

- 当前版本：v0.3.3（见 [Releases](https://github.com/igevin/mHost/releases)）
- 开源协议：Apache License 2.0
- Hosts 模式已在 macOS / Windows / Linux 规划内落地推进，DNS 模式已随 v0.2 发布（详见上文平台支持表）
- 项目持续活跃开发中，采用分阶段交付（`spec/` 目录存档各阶段计划），历史问题审计与性能优化记录公开可查

## 项目愿景

mHost 希望成为一个简单、稳定、可信赖的 Hosts 与域名解析管理工具。它服务于开发者，也服务于普通用户；它既能解决多环境切换的效率问题，也能通过广告屏蔽能力改善日常浏览体验。

相比传统 Hosts 编辑器，mHost 更关注三件事：

- 更低的使用门槛。
- 更少的权限依赖。
- 更安静、更轻量的运行体验。

## Roadmap

已完成：

- Profile 增删改查、一键启用切换。
- 广告屏蔽规则，支持导入 `domains` 格式屏蔽列表、规则冲突/重叠检测。
- Profile 导入导出、快照备份与恢复。
- 安全、可回滚的 Hosts 写入流程（原子写入 + 自动备份）。
- DNS 模式（macOS）。

进行中 / 规划：

- Windows / Linux 的 DNS 模式（见 #67）。
- 广告屏蔽域名白名单。
- 屏蔽规则命中统计可视化。

## 适合谁使用

- 需要频繁切换本地、测试、预发、生产环境的开发者。
- 需要验证不同环境配置的测试人员。
- 想减少广告和追踪请求的普通用户。
- 希望使用轻量工具管理网络解析规则的桌面用户。

## License

本项目基于 [Apache License 2.0](LICENSE) 开源发布。
