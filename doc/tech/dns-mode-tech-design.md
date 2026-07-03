# mHost DNS 模式技术设计方案

> Issue: #67 — 改为支持本地 hosts 模式和本地 DNS 模式两种模式
> 版本: 1.0
> 日期: 2026-07-02

---

## 1. 需求概述

### 1.1 核心目标

将 mHost 从单一 hosts 文件管理模式，扩展为支持两种独立工作模式：

| 模式 | 规则生效方式 | Profile 激活策略 | 与系统关系 |
|------|-------------|-----------------|-----------|
| **Hosts 模式**（已有） | 写入 `/etc/hosts` | 单激活（互斥） | 直接操作 hosts 文件 |
| **DNS 模式**（新增） | 本地 DNS 服务解析 | 多激活（并集） | 启动 DNS 服务 + 修改系统 DNS 配置 |

### 1.2 关键约束

- **两种模式的 Profile 数据集互不共享**，可以并存
- **hosts 文件在 DNS 模式启用后保留**，hosts 文件内容优先级天然高于 DNS
- **DNS 模式无额外优先级处理逻辑**
- DNS 模式是后续**远程规则订阅**和**广告屏蔽（EasyList 等非 hosts 格式）**的基础设施

---

## 2. 数据模型设计

### 2.1 ProfileMode 枚举

在 `mhost-core/src/models.rs` 中新增模式标识：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProfileMode {
    #[default]
    Hosts,
    Dns,
}
```

### 2.2 Profile 结构体扩展

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
    pub protected: bool,
    pub tags: Vec<String>,
    pub rules: Vec<HostRule>,
    // 新增字段
    #[serde(default)]
    pub mode: ProfileMode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

- `mode` 字段使用 `#[serde(default)]` 确保向后兼容：旧数据反序列化时默认值为 `Hosts`
- 两种模式的 `Profile` 使用**同一个结构体**，通过 `mode` 字段区分

### 2.3 存储目录结构调整

`FileStorage` 的存储结构从：

```
{root}/
  manifest.json
  profiles/
    {profile_id}.json
  backups/
  settings.json
```

扩展为：

```
{root}/
  manifest.json          # 升级至 version = 2
  profiles/
    hosts/               # hosts 模式 Profile
      {profile_id}.json
    dns/                 # DNS 模式 Profile
      {profile_id}.json
  backups/
  settings.json
  dns/                   # DNS 服务相关配置
    config.json
    zones/               # DNS zone 文件缓存
```

### 2.4 Manifest 版本升级

```rust
impl Manifest {
    pub fn new(app_version: impl Into<String>) -> Self {
        Self {
            version: 2,  // 从 1 升级到 2
            app_version: app_version.into(),
            updated_at: Utc::now(),
        }
    }
}
```

- 版本 2 新增 `dns_enabled: bool` 字段记录 DNS 模式全局开关状态
- 启动时检测 manifest 版本，从 v1 自动迁移到 v2

---

## 3. 后端架构设计

### 3.1 新增 Crate：`mhost-dns`

```
crates/mhost-dns/src/
  lib.rs           # 公共接口导出
  server.rs        # DNS 服务核心（基于 hickory-dns 或 trust-dns）
  config.rs        # DNS 配置管理
  resolver.rs      # 请求解析与规则匹配
  zones.rs         # Zone 文件生成与管理
  platform.rs      # 平台适配（修改系统 DNS 设置）
```

#### 3.1.1 DNS 服务核心（`server.rs`）

使用 `hickory-dns` 库实现本地 DNS 服务器：

```rust
pub struct DnsServer {
    listener: UdpSocket,
    tcp_listener: TcpListener,
    rule_engine: Arc<RuleEngine>,
    upstream: Vec<SocketAddr>, // 上游 DNS 服务器（如 8.8.8.8, 1.1.1.1）
}

impl DnsServer {
    /// 启动 DNS 服务，监听 UDP/53 和 TCP/53
    pub async fn start(&self) -> Result<(), DnsError>;
    
    /// 优雅停止
    pub async fn stop(&self) -> Result<(), DnsError>;
    
    /// 重新加载规则（profile 变更时调用）
    pub async fn reload_rules(&self, profiles: &[Profile]) -> Result<(), DnsError>;
}
```

#### 3.1.2 规则引擎（`resolver.rs`）

```rust
pub struct RuleEngine {
    /// 域名 -> IP 的映射表（来自所有启用的 DNS 模式 profile 的并集）
    rules: RwLock<HashMap<String, IpAddr>>,
    /// 缓存上游解析结果
    cache: LruCache<String, DnsRecord>,
}

impl RuleEngine {
    /// 根据域名查询对应 IP，优先匹配自定义规则，未命中则转发上游
    pub async fn resolve(&self, domain: &str) -> Option<IpAddr>;
    
    /// 从 profiles 重建规则表
    pub fn rebuild(&self, profiles: &[Profile]);
}
```

#### 3.1.3 平台适配（`platform.rs`）

macOS 下修改系统 DNS 配置：

```rust
/// 获取当前系统 DNS 配置
pub fn get_system_dns() -> Result<Vec<String>, PlatformError>;

/// 设置系统 DNS 为本地服务（127.0.0.1）
pub fn set_local_dns() -> Result<(), PlatformError>;

/// 恢复系统 DNS 为原始配置
pub fn restore_system_dns(original: &[String]) -> Result<(), PlatformError>;
```

实现方式：通过 `networksetup` 命令修改系统 DNS 设置。

### 3.2 Storage 层扩展

#### 3.2.1 Storage Trait 扩展

```rust
pub trait Storage {
    // 原有接口不变...
    
    /// 按模式列出 Profile
    fn list_profiles_by_mode(&self, mode: ProfileMode) -> Result<Vec<Profile>, StorageError>;
    
    /// 列出所有 Profile（跨模式）
    fn list_all_profiles(&self) -> Result<Vec<Profile>, StorageError>;
}
```

#### 3.2.2 FileStorage 实现调整

- `profiles_dir()` 改为按模式返回子目录：`profiles/hosts/` 或 `profiles/dns/`
- `list_profiles()` 保持向后兼容：默认返回 hosts 模式 Profile（与前端现有行为一致）
- 新增 `list_profiles_by_mode()` 和 `list_all_profiles()`

### 3.3 AppState 扩展

```rust
pub struct AppState {
    pub storage: Arc<dyn Storage + Send + Sync>,
    pub writer: Arc<HostsWriter>,
    pub apply_lock: ApplyLock,
    pub snapshot_lock: ApplyLock,
    pub last_profile_ids: Mutex<Vec<String>>,
    // 新增
    pub dns_server: Arc<Mutex<Option<DnsServer>>>,
    pub dns_enabled: AtomicBool,
    pub original_dns: Mutex<Vec<String>>, // 保存原始 DNS 配置以便恢复
}
```

### 3.4 命令层扩展（`src-tauri/src/commands/`）

新增 `dns.rs` 模块：

```rust
/// 启动/停止 DNS 模式
#[tauri::command]
pub async fn set_dns_mode(enabled: bool, state: State<'_, AppState>) -> Result<(), MhostError>;

/// 获取 DNS 模式状态
#[tauri::command]
pub async fn get_dns_mode(state: State<'_, AppState>) -> Result<bool, MhostError>;

/// 重新加载 DNS 规则（profile 变更后调用）
#[tauri::command]
pub async fn reload_dns_rules(state: State<'_, AppState>) -> Result<(), MhostError>;

/// 获取 DNS 服务运行状态
#[tauri::command]
pub async fn get_dns_status(state: State<'_, AppState>) -> Result<DnsStatus, MhostError>;
```

### 3.5 Profile 命令调整

- `create_profile`：新增 `mode` 参数，决定 Profile 存储到哪个子目录
- `list_profiles`：默认返回 hosts 模式 Profile（保持前端兼容）；新增可选 `mode` 参数
- `set_profile_enabled`：根据 Profile 的 `mode` 字段，hosts 模式保持单激活互斥，DNS 模式允许多激活
- `apply_hosts`：仅处理 hosts 模式 Profile，DNS 模式 Profile 不参与 hosts 文件写入

---

## 4. 前端架构设计

### 4.1 路由扩展

```tsx
// App.tsx
<Routes>
  <Route element={<Layout />}>
    <Route path="/" element={<Navigate to="/profiles" replace />} />
    {/* Hosts 模式 Profile */}
    <Route path="/profiles" element={<ProfileView mode="hosts" />} />
    <Route path="/profiles/:id" element={<ProfileView mode="hosts" />} />
    {/* DNS 模式 Profile（新增） */}
    <Route path="/dns-profiles" element={<ProfileView mode="dns" />} />
    <Route path="/dns-profiles/:id" element={<ProfileView mode="dns" />} />
    <Route path="/settings" element={<Settings />} />
    <Route path="/snapshot" element={<SnapshotPage />} />
    <Route path="/hosts" element={<SystemHosts />} />
  </Route>
</Routes>
```

### 4.2 Store 扩展

```ts
// stores/profiles/state.ts

// 新增 atoms
export const dnsProfilesAtom = atom<Profile[]>([]);
export const dnsEnabledAtom = atom(false);
export const dnsStatusAtom = atom<DnsStatus | null>(null);

// hosts 模式相关 atoms 保持不变
export const profilesAtom = atom<Profile[]>([]); // hosts 模式
export const selectedProfileIdAtom = atom<string | null>(null);
```

### 4.3 Layout 导航调整

侧边栏新增导航项：

```
Profiles (Hosts)    → /profiles
DNS Profiles        → /dns-profiles  (新增)
System Hosts        → /hosts
Snapshot            → /snapshot
Settings            → /settings
```

主界面标题栏区域增加 **DNS 模式状态指示器**：
- 图标：WiFi/信号图标 + DNS 标记
- 状态：绿色（运行中）/ 灰色（未启用）
- 点击可快速跳转 Settings 页面

### 4.4 Settings 页面扩展

新增 **DNS 模式控制区域**：

```
┌─ DNS Mode ─────────────────────────────┐
│  Status: [Running ●] / [Stopped ○]     │
│                                        │
│  [Enable DNS Mode]  [Disable DNS Mode] │
│                                        │
│  Upstream DNS: [8.8.8.8] [1.1.1.1]    │
│  Port: 53 (default)                    │
└────────────────────────────────────────┘
```

### 4.5 ProfileView 模式适配

`ProfileView` 接收 `mode: "hosts" | "dns"` prop：

| 行为 | Hosts 模式 | DNS 模式 |
|------|-----------|---------|
| Profile 激活 | 单选（互斥） | 多选（复选框） |
| 启用按钮行为 | enable_and_apply（自动禁用其他） | toggle_enabled（保持其他启用状态） |
| Apply 确认弹窗 | 显示 hosts diff | 显示 DNS 规则变更摘要 |
| 规则编辑器 | 同现有 | 同现有（hosts 规则语法兼容） |

---

## 5. 模式切换流程

### 5.1 启用 DNS 模式

```
用户点击 "Enable DNS Mode"
    ↓
[前端] 调用 set_dns_mode(true)
    ↓
[后端] 1. 保存当前系统 DNS 配置到 original_dns
       2. 启动 DnsServer（监听 127.0.0.1:53）
       3. 调用 platform::set_local_dns() 修改系统 DNS
       4. 加载所有 enabled 的 DNS 模式 Profile，构建规则表
       5. 更新 manifest.dns_enabled = true
       6. 返回成功
    ↓
[前端] 更新 dnsEnabledAtom = true，显示运行状态
```

### 5.2 停用 DNS 模式

```
用户点击 "Disable DNS Mode"
    ↓
[前端] 调用 set_dns_mode(false)
    ↓
[后端] 1. 调用 platform::restore_system_dns() 恢复原始 DNS
       2. 优雅停止 DnsServer
       3. 更新 manifest.dns_enabled = false
       4. 返回成功
    ↓
[前端] 更新 dnsEnabledAtom = false
```

### 5.3 Profile 变更时自动同步

```
用户编辑/启用/禁用 DNS 模式 Profile
    ↓
[后端] 保存 Profile 后
    ↓
[后端] 如果 dns_enabled == true：
       调用 reload_dns_rules() 重建规则表
       （无需重启 DNS 服务）
    ↓
[前端] 刷新 DNS Profile 列表
```

---

## 6. 数据迁移策略

### 6.1 v1 → v2 自动迁移

启动时 `AppState::new()` 中检测 manifest 版本：

```rust
fn migrate_v1_to_v2(storage: &FileStorage) -> Result<(), StorageError> {
    // 1. 将所有现有 profiles/ 下的 .json 文件移动到 profiles/hosts/
    // 2. 更新 manifest.version = 2
    // 3. 设置 dns_enabled = false
    // 4. 保存新 manifest
}
```

- 迁移是一次性的，完成后旧目录结构不再使用
- 迁移失败不应阻止应用启动，记录错误日志

### 6.2 向后兼容性

- `Profile` 的 `mode` 字段使用 `#[serde(default = "ProfileMode::Hosts")]`
- 前端 API 调用保持现有接口不变，新增参数为可选
- 未提供 `mode` 时默认使用 hosts 模式

---

## 7. 安全与权限考量

| 风险点 | 缓解措施 |
|--------|---------|
| DNS 服务监听 53 端口需要 root 权限 | macOS 通过 Tauri 的 `sudo` 提权机制（与 hosts 写入共用同一授权流程） |
| 修改系统 DNS 配置可能导致断网 | 停用 DNS 模式时**必须**恢复原始 DNS；异常退出时通过 tray 守护恢复 |
| DNS 服务崩溃导致系统无法解析 | 设置 upstream fallback，服务崩溃时自动恢复系统 DNS |
| 并发模式切换导致配置混乱 | 使用 `dns_lock`（类似 `apply_lock`）串行化 DNS 模式切换操作 |

---

## 8. 关键技术选型

| 组件 | 选型 | 理由 |
|------|------|------|
| DNS 库 | `hickory-dns` (原 trust-dns) | Rust 原生，成熟稳定，支持自定义 resolver |
| 系统 DNS 修改 | `networksetup` (macOS) | 无需额外依赖，标准系统命令 |
| 规则缓存 | `lru` crate | 轻量，避免上游查询性能瓶颈 |
| 并发控制 | `tokio::sync::RwLock` | 与现有 Tauri async runtime 一致 |

---

## 9. 测试策略

### 9.1 单元测试

- `mhost-dns` crate：resolver 规则匹配逻辑、zone 文件生成、配置序列化
- `mhost-storage`：按模式分目录的 CRUD 操作、v1→v2 迁移逻辑
- `mhost-core`：ProfileMode 序列化/反序列化

### 9.2 集成测试

- DNS 服务启动/停止生命周期
- 模式切换后系统 DNS 配置正确性（macOS 沙盒测试）
- hosts 模式与 DNS 模式 Profile 互不干扰
- 多 DNS Profile 并集规则正确性

### 9.3 端到端测试

- 启用 DNS 模式后域名解析正确性
- 停用 DNS 模式后网络恢复
- Profile 切换时 DNS 规则热更新

---

## 10. 风险与待决策项

| # | 风险/待决策项 | 影响 | 建议 |
|---|-------------|------|------|
| 1 | DNS 服务需要 root 权限启动 53 端口 | 高 | 调研是否可使用高位端口 + 系统端口转发，降低权限要求 |
| 2 | `hickory-dns` 引入的依赖体积 | 中 | 评估编译后二进制增量，必要时考虑更轻量方案 |
| 3 | 与 VPN/代理软件的 DNS 冲突 | 高 | 需测试常见 VPN 软件（Clash、Surge、V2Ray）共存场景 |
| 4 | DNS 模式下 IPv6 支持 | 中 | 初期仅支持 A 记录（IPv4），AAAA 记录后续迭代 |
| 5 | 缓存策略（TTL、过期清理） | 低 | 初期使用简单 LRU，后续根据实际性能数据优化 |

---

## 附录 A：接口变更清单

### A.1 新增 Tauri 命令

| 命令 | 输入 | 输出 | 说明 |
|------|------|------|------|
| `set_dns_mode` | `enabled: bool` | `Result<(), MhostError>` | 启用/停用 DNS 模式 |
| `get_dns_mode` | — | `Result<bool, MhostError>` | 获取 DNS 模式状态 |
| `reload_dns_rules` | — | `Result<(), MhostError>` | 热重载 DNS 规则 |
| `get_dns_status` | — | `Result<DnsStatus, MhostError>` | 获取 DNS 服务状态 |
| `list_dns_profiles` | — | `Result<Vec<Profile>, MhostError>` | 列出 DNS 模式 Profile |

### A.2 变更的 Tauri 命令

| 命令 | 变更 | 说明 |
|------|------|------|
| `create_profile` | 新增可选参数 `mode: Option<ProfileMode>` | 默认 Hosts |
| `list_profiles` | 新增可选参数 `mode: Option<ProfileMode>` | 默认返回 Hosts 模式 |
| `set_profile_enabled` | 根据 Profile.mode 决定互斥/多选逻辑 | DNS 模式不互斥 |
| `apply_hosts` | 仅处理 Hosts 模式 Profile | DNS Profile 不参与 |

### A.3 新增前端类型

```ts
// types/index.ts

export type ProfileMode = "hosts" | "dns";

export interface Profile {
  id: string;
  name: string;
  description?: string;
  enabled: boolean;
  protected: boolean;
  tags: string[];
  rules: HostRule[];
  mode: ProfileMode;  // 新增
  created_at: string;
  updated_at: string;
}

export interface DnsStatus {
  running: boolean;
  port: number;
  upstream: string[];
  rule_count: number;
  cache_size: number;
}
```
