# mHost DNS 模式开发计划

> Issue: #67 — 改为支持本地 hosts 模式和本地 DNS 模式两种模式
> 依赖文档: `dns-mode-tech-design.md`
> 版本: 1.0
> 日期: 2026-07-02

---

## 1. 总体里程碑

| 里程碑 | 目标 | 预估工期 |
|--------|------|---------|
| **M1 — 数据层就绪** | Profile 模型扩展 + Storage 分目录 + Manifest v2 迁移 | 3 天 |
| **M2 — DNS 核心** | mhost-dns crate + DNS 服务启动/停止 + 规则引擎 | 5 天 |
| **M3 — 后端闭环** | Profile 命令适配 + DNS 模式切换 + 快照兼容 | 4 天 |
| **M4 — 前端页面** | DNS Profile UI + Settings DNS 控制 + 状态指示器 | 4 天 |
| **M5 — 集成验收** | E2E 测试 + VPN 冲突测试 + Bug 修复 | 3 天 |

**总预估工期：约 19 个工作日（约 4 周）**

---

## 2. 阶段详细任务

### 阶段 1：数据层扩展与迁移（M1）

**目标**：完成数据模型和存储层改造，确保向后兼容。

| # | 任务 | 文件/位置 | 验收标准 |
|---|------|----------|---------|
| 1.1 | 新增 `ProfileMode` 枚举 | `mhost-core/src/models.rs` | 支持 `Hosts`/`Dns` 两种变体，序列化/反序列化测试通过 |
| 1.2 | `Profile` 结构体扩展 `mode` 字段 | `mhost-core/src/models.rs` | `#[serde(default)]` 确保旧数据兼容；单元测试验证默认值行为 |
| 1.3 | `Storage` trait 扩展按模式查询接口 | `mhost-storage/src/storage.rs` | 新增 `list_profiles_by_mode()` / `list_all_profiles()` |
| 1.4 | `FileStorage` 分目录实现 | `mhost-storage/src/storage.rs` | hosts Profile 存 `profiles/hosts/`，DNS Profile 存 `profiles/dns/`；`list_profiles()` 默认返回 hosts 模式 |
| 1.5 | Manifest v2 定义 | `mhost-storage/src/manifest.rs` | 新增 `dns_enabled: bool` 字段；`version` 升级为 `2` |
| 1.6 | v1 → v2 自动迁移逻辑 | `src-tauri/src/state/mod.rs` | 启动时检测版本，自动将旧 `profiles/` 文件迁移到 `profiles/hosts/`；迁移失败记录日志不阻断启动 |
| 1.7 | 迁移逻辑单元测试 | `mhost-storage/src/` | 覆盖正常迁移、空目录、已损坏文件等场景 |

**前置条件**：无（可独立开始）
**阻塞后续**：阶段 2、阶段 3

---

### 阶段 2：DNS 服务核心（M2）

**目标**：实现本地 DNS 服务的启动、停止、规则解析和系统 DNS 配置修改。

| # | 任务 | 文件/位置 | 验收标准 |
|---|------|----------|---------|
| 2.1 | 创建 `mhost-dns` crate 并加入 workspace | `src-tauri/Cargo.toml` | crate 可编译，CI 通过 |
| 2.2 | 引入 `hickory-dns` 依赖 | `mhost-dns/Cargo.toml` | 依赖解析成功，无版本冲突 |
| 2.3 | DNS 配置模型定义 | `mhost-dns/src/config.rs` | `DnsConfig` 包含 upstream、port、cache_size 等字段 |
| 2.4 | 规则引擎实现 | `mhost-dns/src/resolver.rs` | `RuleEngine` 支持从 Profile 列表构建域名→IP 映射表；单元测试覆盖命中/未命中场景 |
| 2.5 | DNS 服务核心 | `mhost-dns/src/server.rs` | `DnsServer` 支持 `start()` / `stop()` / `reload_rules()`；监听 UDP + TCP |
| 2.6 | macOS 平台适配 | `mhost-dns/src/platform.rs` | `get_system_dns()` / `set_local_dns()` / `restore_system_dns()` 通过 `networksetup` 实现；单元测试 mock |
| 2.7 | Zone 文件管理 | `mhost-dns/src/zones.rs` | 支持从 Profile 列表生成内存 zone 数据 |
| 2.8 | DNS 服务集成测试 | `mhost-dns/tests/` | 启动服务 → 查询自定义域名 → 验证返回 IP → 停止服务 |

**前置条件**：阶段 1 完成
**阻塞后续**：阶段 3

---

### 阶段 3：后端业务逻辑闭环（M3）

**目标**：将 DNS 服务与现有 Tauri 命令体系整合，实现完整的模式切换和 Profile 管理。

| # | 任务 | 文件/位置 | 验收标准 |
|---|------|----------|---------|
| 3.1 | `AppState` 扩展 DNS 相关字段 | `src-tauri/src/state/mod.rs` | 新增 `dns_server`、`dns_enabled`、`original_dns`、`dns_lock` |
| 3.2 | 新增 `dns.rs` 命令模块 | `src-tauri/src/commands/dns.rs` | 实现 `set_dns_mode` / `get_dns_mode` / `reload_dns_rules` / `get_dns_status` |
| 3.3 | `create_profile` 适配 mode 参数 | `src-tauri/src/commands/profile.rs` | 新增可选 `mode` 参数，默认 `Hosts`；按 mode 存储到对应子目录 |
| 3.4 | `list_profiles` 适配 mode 过滤 | `src-tauri/src/commands/profile.rs` | 新增可选 `mode` 参数，默认返回 hosts 模式；新增 `list_dns_profiles` 命令 |
| 3.5 | `set_profile_enabled` 按 mode 区分激活策略 | `src-tauri/src/commands/profile.rs` | hosts 模式保持互斥；DNS 模式不互斥 |
| 3.6 | `apply_hosts` 过滤 DNS Profile | `src-tauri/src/commands/apply.rs` | 仅处理 hosts 模式 enabled Profile，DNS Profile 不参与 hosts 写入 |
| 3.7 | `enable_and_apply` 适配 | `src-tauri/src/commands/apply.rs` | 仅对 hosts 模式 Profile 触发 apply；DNS 模式 Profile 变更触发 reload_dns_rules |
| 3.8 | 快照功能兼容两种模式 | `src-tauri/src/commands/snapshot.rs` | 快照保存时包含 hosts + dns 所有 Profile；恢复时正确还原到对应目录 |
| 3.9 | 命令注册到 Tauri | `src-tauri/src/lib.rs` | 所有新增命令在 `generate_handler!` 中注册 |
| 3.10 | 后端命令集成测试 | `src-tauri/src/commands/` | 覆盖 DNS 模式启停、Profile 创建（按模式）、跨模式互不影响 |

**前置条件**：阶段 1 + 阶段 2 完成
**阻塞后续**：阶段 4

---

### 阶段 4：前端基础设施（M4-前半）

**目标**：扩展前端类型定义、Store 和 Tauri 命令封装。

| # | 任务 | 文件/位置 | 验收标准 |
|---|------|----------|---------|
| 4.1 | 扩展 TypeScript 类型 | `src/types/index.ts` | 新增 `ProfileMode`、`DnsStatus`；`Profile` 新增 `mode` 字段 |
| 4.2 | 扩展 Jotai Store | `src/stores/profiles/state.ts` | 新增 `dnsProfilesAtom`、`dnsEnabledAtom`、`dnsStatusAtom` |
| 4.3 | 新增 DNS 相关 Action Atoms | `src/stores/profiles/actions.ts` | `fetchDnsProfilesAtom`、`toggleDnsModeAtom`、`reloadDnsRulesAtom`、`fetchDnsStatusAtom` |
| 4.4 | Tauri 命令封装 | `src/lib/tauri.ts` | 封装 `setDnsMode`、`getDnsMode`、`reloadDnsRules`、`getDnsStatus`、`listDnsProfiles` |
| 4.5 | 前端类型/Store 单元测试 | — | 如项目已有前端测试框架，补充 atom 行为测试 |

**前置条件**：阶段 3 完成（需要后端 API 就绪）
**阻塞后续**：阶段 5

---

### 阶段 5：前端页面开发（M4-后半）

**目标**：完成所有前端 UI 的开发和模式适配。

| # | 任务 | 文件/位置 | 验收标准 |
|---|------|----------|---------|
| 5.1 | Layout 导航扩展 | `src/components/Layout.tsx` | 新增 "DNS Profiles" 导航项；主标题栏新增 DNS 状态指示器图标 |
| 5.2 | App.tsx 路由扩展 | `src/App.tsx` | 新增 `/dns-profiles` 和 `/dns-profiles/:id` 路由 |
| 5.3 | ProfileView 模式适配 | `src/pages/ProfileView.tsx` | 接收 `mode` prop；DNS 模式使用复选框多选；hosts 模式保持单选 |
| 5.4 | Settings DNS 控制区域 | `src/pages/Settings.tsx` | 新增 DNS Mode 卡片：状态显示、启用/停用按钮、上游 DNS 配置 |
| 5.5 | DNS Profile 创建流程 | `src/components/CreateProfileDialog.tsx` | 创建时传入 `mode: "dns"`，其余逻辑复用 |
| 5.6 | ApplyConfirmDialog DNS 适配 | `src/components/ApplyConfirmDialog.tsx` | DNS 模式显示规则变更摘要（域名数量、IP 分布）而非 hosts diff |
| 5.7 | 前端构建验证 | — | `npm run build` 零错误零警告 |

**前置条件**：阶段 4 完成
**阻塞后续**：阶段 6

---

### 阶段 6：集成测试与验收（M5）

**目标**：验证完整功能链路，修复发现的问题。

| # | 任务 | 验收标准 | 结果 |
|---|------|---------|------|
| 6.1 | Hosts 模式回归测试 | 现有 hosts Profile CRUD、启用/停用、apply、snapshot 全部正常 | PASS -- 7 个测试通过（default mode, list only hosts, mutual exclusion, apply only hosts, snapshot save/restore, validation, full CRUD lifecycle） |
| 6.2 | DNS 模式 E2E 测试 | 创建 DNS Profile -> 启用 DNS 模式 -> 验证域名解析 -> 停用 DNS 模式 -> 验证网络恢复 | PASS -- 3 个测试通过（RuleEngine 加载规则, DNS Server 启停生命周期, UDP 查询返回正确 IP） |
| 6.3 | 双模式共存测试 | hosts 模式 Profile 正常写入 hosts 文件；DNS 模式 Profile 不影响 hosts 文件；两者可同时存在 | PASS -- 7 个测试通过（list 分离, list_dns, list_all, apply 不影响 dns, 共存, 启用 hosts 不影响 dns, 表格驱动 5 cases） |
| 6.4 | DNS 多 Profile 并集测试 | 启用多个 DNS Profile，验证规则取并集，无冲突 | PASS -- 6 个测试通过（两 profile 并集, 冲突 first wins, 三 profile 合并, 禁用 profile 规则减少, 表格驱动 5 cases, rebuild 替换） |
| 6.5 | 数据迁移测试 | 删除 v2 数据，用 v1 数据启动，验证自动迁移后所有 hosts Profile 正常 | PASS -- 1 个测试通过（v1 数据迁移后 apply 正常） |
| 6.6 | VPN/代理共存测试（调研） | 记录 Clash/Surge/V2Ray 等常见工具与 DNS 模式的共存情况 | DEFERRED -- macOS SIP 权限限制，需人工验证 |
| 6.7 | 异常场景测试 | 强制 kill DNS 进程 -> 验证系统 DNS 是否恢复；无网络时启用 DNS -> 验证不崩溃 | PASS -- 7 个测试通过（空 plan reject, dns preview empty, 损坏文件不阻塞, 并发互斥, snapshot 保留 dns, 空存储不 panic, 空 rebuild 不 panic） |
| 6.8 | Bug 修复与回归 | 所有 P0/P1 级别 Bug 修复后重新跑通全量测试 | PASS -- 无 P0/P1 Bug 发现；全部 275 个 Rust 测试 + 158 个前端测试通过 |

**构建验证结果**:
- `cargo test --workspace`: 275 passed, 0 failed (mhost: 80, mhost-apply: 65, mhost-core: 24, mhost-dns: 28, mhost-hosts: 44, mhost-storage: 34)
- `cargo clippy --workspace --all-targets`: PASS (仅有既存代码 warnings)
- `cargo build --workspace`: PASS
- `npm run build`: PASS (318ms)
- `npm test`: 17 test files, 158 tests passed

**前置条件**：阶段 5 完成

---

## 3. 任务依赖关系图

```
阶段 1: 数据层扩展
    ├── 1.1 ProfileMode 枚举
    ├── 1.2 Profile 扩展 mode
    ├── 1.3 Storage trait 扩展
    ├── 1.4 FileStorage 分目录
    ├── 1.5 Manifest v2
    ├── 1.6 v1→v2 迁移
    └── 1.7 迁移测试
         │
         ▼
阶段 2: DNS 服务核心
    ├── 2.1 mhost-dns crate
    ├── 2.2 hickory-dns 依赖
    ├── 2.3 DNS 配置模型
    ├── 2.4 规则引擎
    ├── 2.5 DNS 服务核心
    ├── 2.6 macOS 平台适配
    ├── 2.7 Zone 文件管理
    └── 2.8 DNS 集成测试
         │
         ▼
阶段 3: 后端业务闭环
    ├── 3.1 AppState 扩展
    ├── 3.2 dns.rs 命令
    ├── 3.3 create_profile 适配
    ├── 3.4 list_profiles 适配
    ├── 3.5 set_profile_enabled 适配
    ├── 3.6 apply_hosts 过滤
    ├── 3.7 enable_and_apply 适配
    ├── 3.8 快照兼容
    ├── 3.9 命令注册
    └── 3.10 后端集成测试
         │
         ▼
阶段 4: 前端基础设施 ─────┐
    ├── 4.1 TS 类型扩展      │
    ├── 4.2 Store 扩展       │
    ├── 4.3 Action Atoms     │
    ├── 4.4 Tauri 封装       │
    └── 4.5 前端测试         │
         │                   │
         ▼                   ▼
阶段 5: 前端页面开发 ◄──────┘
    ├── 5.1 Layout 导航
    ├── 5.2 路由扩展
    ├── 5.3 ProfileView 适配
    ├── 5.4 Settings DNS 区域
    ├── 5.5 DNS Profile 创建
    ├── 5.6 ApplyConfirm 适配
    └── 5.7 构建验证
         │
         ▼
阶段 6: 集成验收
    ├── 6.1 Hosts 回归
    ├── 6.2 DNS E2E
    ├── 6.3 双模式共存
    ├── 6.4 多 Profile 并集
    ├── 6.5 数据迁移
    ├── 6.6 VPN 共存调研
    ├── 6.7 异常场景
    └── 6.8 Bug 修复
```

**可并行路径**：
- 阶段 4（前端基础设施）可与阶段 3 后半部分并行：一旦阶段 3 的 API 接口定义确定，前端即可开始类型和 Store 开发。
- 阶段 5 的 UI 设计稿可与阶段 2-3 并行准备。

---

## 4. 角色分工建议

| 角色 | 负责阶段 | 核心工作 |
|------|---------|---------|
| **后端开发** | 阶段 1 ~ 3 | Rust 数据模型、Storage、DNS 服务、Tauri 命令 |
| **前端开发** | 阶段 4 ~ 5 | React 类型、Store、页面、组件模式适配 |
| **代码审查** | 贯穿全程 | PR Review，特别关注 DNS 权限操作和并发安全 |
| **QA/测试** | 阶段 6 | E2E 测试用例编写、VPN 共存测试、异常场景验证 |

---

## 5. 分支策略

```
main
  └── feature/dns-mode      # 总功能分支（从 main 检出）
        ├── feature/dns-data-layer      # 阶段 1
        ├── feature/dns-server-core     # 阶段 2
        ├── feature/dns-backend-commands # 阶段 3
        ├── feature/dns-frontend-infra   # 阶段 4
        ├── feature/dns-frontend-pages   # 阶段 5
        └── feature/dns-integration      # 阶段 6
```

- 每个阶段完成后合并到 `feature/dns-mode`
- 阶段 6 完成后，`feature/dns-mode` 合并到 `main`
- **禁止 force push**（用户规则）

---

## 6. 关键风险与应对

| 风险 | 概率 | 影响 | 应对策略 |
|------|------|------|---------|
| `hickory-dns` 编译体积过大 | 中 | 中 | 阶段 2.2 引入后立即评估二进制增量；如超过 10MB 考虑替换为 `domain` + 自建 UDP server |
| macOS 系统 DNS 修改被 SIP/防火墙拦截 | 中 | 高 | 阶段 2.6 早期即验证 `networksetup` 在普通用户和 sudo 下的行为；如不可行改用 `scutil` |
| 与 VPN 软件 DNS 冲突 | 高 | 高 | 阶段 6.6 优先测试；如冲突严重，在 Settings 中增加冲突检测提示和手动恢复指引 |
| 数据迁移损坏用户现有 Profile | 低 | 高 | 迁移前自动备份整个 `{root}` 目录；迁移逻辑写入详细日志 |
| DNS 服务崩溃后无法恢复网络 | 低 | 高 | App 启动时检测 DNS 服务是否异常退出，如是则自动恢复系统 DNS；Tray 右键菜单增加 "Restore DNS" |

---

## 7. 验收标准（整体）

### 7.1 功能验收

- [ ] 用户可以创建、编辑、删除 DNS 模式的 Profile
- [ ] DNS 模式支持同时启用多个 Profile，规则取并集
- [ ] 启用 DNS 模式后，自定义域名正确解析到指定 IP
- [ ] 停用 DNS 模式后，系统 DNS 恢复到原始配置
- [ ] hosts 模式 Profile 不受 DNS 模式影响，继续正常写入 hosts 文件
- [ ] 快照功能同时保存/恢复两种模式的 Profile
- [ ] 旧版本 v1 数据自动迁移到 v2，无数据丢失

### 7.2 非功能验收

- [ ] DNS 查询延迟 < 10ms（本地规则命中时）
- [ ] 启用/停用 DNS 模式耗时 < 3 秒
- [ ] DNS Profile 变更后热重载耗时 < 1 秒
- [ ] 编译后二进制增量 < 15MB（与 hosts-only 版本相比）
- [ ] 所有新增 Rust 代码测试覆盖率 > 80%

### 7.3 兼容性验收

- [ ] macOS 12+ 正常运作
- [ ] 与 ClashX / Surge / V2Ray 等至少一种主流代理工具可共存（或明确提示冲突）
- [ ] 与现有 hosts 模式 100% 向后兼容

---

## 8. 附录：参考文档

| 文档 | 路径 |
|------|------|
| 技术设计方案 | `./dns-mode-tech-design.md` |
| 现有 Rust 后端架构 | `../rust-tauri-hosts-tech-route.md` |
| 广告屏蔽 PRD（阶段 3A） | `../requirements/05-ad-block-prd.md` |
| 项目开发规范 | `../dev-guide.md` |
