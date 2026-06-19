# 阶段 0：产品骨架与数据模型 — 开发计划

创建日期：2026-06-19
修订日期：2026-06-19（第 1 版）

---

## 1. 阶段目标

把 mHost 的核心概念稳定下来，形成可执行的技术基础，避免后续功能不断返工。

---

## 2. 关键决策（已确认）

| 决策项 | 结论 |
|--------|------|
| 首发平台 | macOS 优先，Windows 后续扩展 |
| Profile 叠加 | 阶段 0 只支持单 Profile 启用 |
| 前端框架 | React + TypeScript + Vite |
| 前端状态管理 | Jotai（轻量，与 SwitchHosts 技术栈一致） |
| macOS 权限 | 阶段 0 采用弹窗授权，阶段 1 评估 Privileged Helper |
| 开发方式 | TDD（Red-Green-Refactor） |

---

## 3. 技术栈版本

| 组件 | 版本 | 说明 |
|------|------|------|
| Tauri | v2 | 跨平台桌面应用框架 |
| Rust | 1.78+ | 最低版本要求 |
| Node.js | 20+ LTS | 前端运行时 |
| pnpm | 9+ | 包管理器 |
| React | 18+ | UI 框架 |
| React Router | v6 | 前端路由 |
| Jotai | 2+ | 状态管理 |

---

## 4. TDD 执行原则

- Red：先写测试，确认失败
- Green：实现最小代码，让测试通过
- Refactor：重构，保持测试通过
- 所有单元测试采用表格驱动

表格驱动测试格式：

```rust
#[test]
fn test_feature() {
    let cases = vec![
        ("case_name", input, expected),
        // ...
    ];
    for (name, input, expected) in cases {
        let result = function_under_test(input);
        assert_eq!(result, expected, "case: {}", name);
    }
}
```

---

## 5. 工程目录结构

初始化后的完整目录树：

```
mHost/
  .github/
    workflows/
      ci.yml
  src-tauri/                    # Tauri Rust 工程
    Cargo.toml                  # Workspace 根配置
    src/
      main.rs                   # Tauri 入口
      commands/                 # Tauri commands（前后端接口）
        mod.rs
        profile.rs
        apply.rs
      state/                    # 应用状态管理
        mod.rs
      platform/                 # 平台适配（macOS/Windows）
        mod.rs
        macos.rs
    crates/                     # Workspace crates
      mhost-core/
        Cargo.toml
        src/
          lib.rs
          models/               # 数据模型
          error.rs              # 错误类型
      mhost-hosts/
        Cargo.toml
        src/
          lib.rs
          parser.rs             # hosts 解析
          formatter.rs          # hosts 格式化
          validator.rs          # 语法校验
      mhost-storage/
        Cargo.toml
        src/
          lib.rs
          storage.rs            # 存储 trait 实现
          manifest.rs           # manifest 管理
      mhost-apply/
        Cargo.toml
        src/
          lib.rs
          merge.rs              # 规则合并
          conflict.rs           # 冲突检测
          diff.rs               # diff 生成
          writer.rs             # 系统写入
    tauri.conf.json
  src/                          # 前端工程
    main.tsx                    # React 入口
    App.tsx                     # 根组件
    pages/                      # 页面
      ProfileList.tsx
      ProfileEdit.tsx
      Settings.tsx
    components/                 # 组件
    stores/                     # Jotai atoms
    hooks/                      # 自定义 hooks
    types/                      # TypeScript 类型（与 Rust 对应）
    lib/
      tauri.ts                  # Tauri API 封装
  index.html
  vite.config.ts
  tsconfig.json
  package.json
  pnpm-lock.yaml
  Cargo.lock
  .gitignore
```

---

## 6. 核心数据模型

定义在 `mhost-core` crate 中。

### 6.1 ID 类型

```rust
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceId(pub Uuid);
```

### 6.2 Profile

```rust
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

impl Profile {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: ProfileId(Uuid::new_v4()),
            name: name.into(),
            description: None,
            enabled: false,
            protected: false,
            tags: Vec::new(),
            rules: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}
```

### 6.3 HostRule

```rust
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostRule {
    pub id: RuleId,
    pub ip: IpAddr,
    pub domains: Vec<String>,
    pub enabled: bool,
    pub comment: Option<String>,
    pub source: RuleSource,
}

impl HostRule {
    pub fn new(ip: IpAddr, domains: Vec<String>) -> Self {
        Self {
            id: RuleId(Uuid::new_v4()),
            ip,
            domains,
            enabled: true,
            comment: None,
            source: RuleSource::Manual,
        }
    }
}
```

**关键决策**：一行多域名（`127.0.0.1 a.com b.com`）解析时，在 `HostRule` 中保留为一条规则（`domains: vec!["a.com", "b.com"]`），格式化输出时再展开为多行。这样保留原始语义，便于编辑。

### 6.4 RuleSource

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuleSource {
    Manual,
    Remote { source_id: SourceId, source_name: String },
    AdBlock { source_id: SourceId, source_name: String },
}
```

阶段 0 只使用 `Manual` 变体，`Remote` 和 `AdBlock` 为后续阶段预留。

### 6.5 ApplyPlan

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyPlan {
    pub rules: Vec<ResolvedRule>,
    pub conflicts: Vec<RuleConflict>,
    pub diff: HostsDiff,
    pub backup_required: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRule {
    pub ip: IpAddr,
    pub domain: String,
    pub source_profile_id: ProfileId,
    pub source_profile_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuleConflict {
    pub domain: String,
    pub rules: Vec<ResolvedRule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostsDiff {
    pub added: Vec<String>,    // 新增的行（hosts 文本格式）
    pub removed: Vec<String>,  // 删除的行
    pub unchanged: Vec<String>, // 未变更的行
}
```

### 6.6 错误类型

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MhostError {
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
    
    #[error("apply error: {0}")]
    Apply(#[from] ApplyError),
    
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum ParseError {
    #[error("invalid IP address: {0}")]
    InvalidIp(String),
    
    #[error("invalid domain: {0}")]
    InvalidDomain(String),
    
    #[error("malformed line: {0}")]
    MalformedLine(String),
}

#[derive(Error, Debug, PartialEq)]
pub enum ApplyError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    
    #[error("hosts file not found")]
    HostsFileNotFound,
    
    #[error("backup failed: {0}")]
    BackupFailed(String),
    
    #[error("external modification detected")]
    ExternalModification,
}

#[derive(Error, Debug, PartialEq)]
pub enum StorageError {
    #[error("profile not found: {0}")]
    ProfileNotFound(ProfileId),
    
    #[error("manifest corrupted: {0}")]
    ManifestCorrupted(String),
    
    #[error("version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: u32, found: u32 },
}
```

---

## 7. 前后端接口契约

Tauri command 定义在 `src-tauri/src/commands/` 中。

### 7.1 Profile 相关

```rust
// src-tauri/src/commands/profile.rs

use tauri::State;
use mhost_core::{Profile, ProfileId, HostRule, MhostError};
use crate::state::AppState;

#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> Result<Vec<Profile>, MhostError> {
    // 返回所有 Profile 列表
}

#[tauri::command]
pub async fn get_profile(
    id: String,
    state: State<'_, AppState>
) -> Result<Profile, MhostError> {
    // 根据 ID 返回单个 Profile
}

#[tauri::command]
pub async fn create_profile(
    name: String,
    state: State<'_, AppState>
) -> Result<Profile, MhostError> {
    // 创建新 Profile
}

#[tauri::command]
pub async fn update_profile(
    profile: Profile,
    state: State<'_, AppState>
) -> Result<Profile, MhostError> {
    // 更新 Profile
}

#[tauri::command]
pub async fn delete_profile(
    id: String,
    state: State<'_, AppState>
) -> Result<(), MhostError> {
    // 删除 Profile
}

#[tauri::command]
pub async fn set_profile_enabled(
    id: String,
    enabled: bool,
    state: State<'_, AppState>
) -> Result<Profile, MhostError> {
    // 启用/禁用 Profile（阶段 0 只允许一个启用）
}
```

### 7.2 Apply 相关

```rust
// src-tauri/src/commands/apply.rs

use mhost_apply::ApplyPlan;
use mhost_core::MhostError;

#[tauri::command]
pub async fn generate_apply_plan(
    state: State<'_, AppState>
) -> Result<ApplyPlan, MhostError> {
    // 生成应用计划（含 diff、冲突检测）
}

#[tauri::command]
pub async fn apply_hosts(
    state: State<'_, AppState>
) -> Result<(), MhostError> {
    // 应用 hosts（需要弹窗授权）
}

#[tauri::command]
pub async fn rollback_hosts(
    state: State<'_, AppState>
) -> Result<(), MhostError> {
    // 回滚到上一版
}

#[tauri::command]
pub async fn read_system_hosts() -> Result<String, MhostError> {
    // 读取当前系统 hosts 内容（用于展示）
}
```

### 7.3 前端调用封装

```typescript
// src/lib/tauri.ts

import { invoke } from '@tauri-apps/api/core';
import type { Profile, ApplyPlan } from '../types';

export async function listProfiles(): Promise<Profile[]> {
    return invoke('list_profiles');
}

export async function getProfile(id: string): Promise<Profile> {
    return invoke('get_profile', { id });
}

export async function createProfile(name: string): Promise<Profile> {
    return invoke('create_profile', { name });
}

export async function updateProfile(profile: Profile): Promise<Profile> {
    return invoke('update_profile', { profile });
}

export async function deleteProfile(id: string): Promise<void> {
    return invoke('delete_profile', { id });
}

export async function setProfileEnabled(id: string, enabled: boolean): Promise<Profile> {
    return invoke('set_profile_enabled', { id, enabled });
}

export async function generateApplyPlan(): Promise<ApplyPlan> {
    return invoke('generate_apply_plan');
}

export async function applyHosts(): Promise<void> {
    return invoke('apply_hosts');
}

export async function rollbackHosts(): Promise<void> {
    return invoke('rollback_hosts');
}

export async function readSystemHosts(): Promise<string> {
    return invoke('read_system_hosts');
}
```

### 7.4 TypeScript 类型定义

```typescript
// src/types/index.ts

export interface Profile {
    id: string;
    name: string;
    description: string | null;
    enabled: boolean;
    protected: boolean;
    tags: string[];
    rules: HostRule[];
    created_at: string;  // ISO 8601
    updated_at: string;
}

export interface HostRule {
    id: string;
    ip: string;
    domains: string[];
    enabled: boolean;
    comment: string | null;
    source: RuleSource;
}

export type RuleSource =
    | { type: 'Manual' }
    | { type: 'Remote'; source_id: string; source_name: string }
    | { type: 'AdBlock'; source_id: string; source_name: string };

export interface ApplyPlan {
    rules: ResolvedRule[];
    conflicts: RuleConflict[];
    diff: HostsDiff;
    backup_required: boolean;
}

export interface ResolvedRule {
    ip: string;
    domain: string;
    source_profile_id: string;
    source_profile_name: string;
}

export interface RuleConflict {
    domain: string;
    rules: ResolvedRule[];
}

export interface HostsDiff {
    added: string[];
    removed: string[];
    unchanged: string[];
}
```

---

## 8. 任务拆分

### T1：初始化 Tauri 工程骨架

**类型**：基础设施，非 TDD

**依赖**：无

**具体步骤**：

1. 确认环境：
   ```bash
   rustc --version  # >= 1.78
   node --version   # >= 20
   pnpm --version   # >= 9
   ```

2. 初始化 Tauri v2 项目：
   ```bash
   npm create tauri-app@latest . -- --template react-ts
   # 或
   cargo install create-tauri-app
   cargo create-tauri-app --template react-ts
   ```
   选择：
   - 前端目录：`src`
   - 包管理器：pnpm
   - UI 模板：React + TypeScript

3. 配置 Rust workspace（`src-tauri/Cargo.toml`）：
   ```toml
   [workspace]
   members = [".", "crates/*"]
   resolver = "2"

   [workspace.dependencies]
   serde = { version = "1.0", features = ["derive"] }
   serde_json = "1.0"
   chrono = { version = "0.4", features = ["serde"] }
   uuid = { version = "1.8", features = ["v4", "serde"] }
   thiserror = "1.0"
   tokio = { version = "1.37", features = ["full"] }
   ```

4. 创建 crate 目录结构：
   ```bash
   mkdir -p src-tauri/crates/mhost-core/src
   mkdir -p src-tauri/crates/mhost-hosts/src
   mkdir -p src-tauri/crates/mhost-storage/src
   mkdir -p src-tauri/crates/mhost-apply/src
   mkdir -p src-tauri/src/commands
   mkdir -p src-tauri/src/state
   mkdir -p src-tauri/src/platform
   ```

5. 配置前端：
   - 安装依赖：`pnpm add react-router-dom jotai`
   - 配置 `vite.config.ts`（确保 `base: './'`）
   - 配置 `tsconfig.json` 路径别名

6. 配置 `.gitignore`：
   ```
   target/
   dist/
   node_modules/
   .DS_Store
   *.log
   ```

7. 配置基础 CI（`.github/workflows/ci.yml`）：
   - Rust format（`cargo fmt --check`）
   - Rust clippy（`cargo clippy --all-targets --all-features`）
   - Rust 测试（`cargo test`）
   - 前端 build（`pnpm build`）
   - Tauri build smoke test

**预估**：0.5 天

---

### T2：定义核心数据模型 + 测试

**类型**：TDD

**依赖**：T1

**内容**：

在 `mhost-core` 中实现第 6 节定义的所有数据模型。

**TDD 测试用例**：

```rust
#[test]
fn test_profile_serialization() {
    let cases = vec![
        ("minimal", Profile::new("test"), /* 预期 JSON */),
        ("with_rules", /* ... */),
    ];
    // 验证序列化 / 反序列化往返正确
}

#[test]
fn test_profile_default_values() {
    let p = Profile::new("dev");
    assert!(!p.enabled);
    assert!(!p.protected);
    assert!(p.tags.is_empty());
    assert!(p.rules.is_empty());
    assert!(p.description.is_none());
}

#[test]
fn test_host_rule_multi_domain() {
    let rule = HostRule::new(
        "127.0.0.1".parse().unwrap(),
        vec!["a.com".to_string(), "b.com".to_string()],
    );
    assert_eq!(rule.domains.len(), 2);
}

#[test]
fn test_error_display() {
    let err = MhostError::Parse(ParseError::InvalidIp("bad".to_string()));
    assert!(err.to_string().contains("invalid IP"));
}
```

**预估**：1 天

---

### T3：实现存储层 + 测试

**类型**：TDD

**依赖**：T2

**内容**：

在 `mhost-storage` 中实现本地文件存储。

**存储路径**：

使用 `dirs` crate 获取系统数据目录：

```rust
use dirs::data_dir;

fn storage_root() -> PathBuf {
    data_dir().unwrap().join("mHost")
}
```

macOS 下路径：`~/Library/Application Support/mHost/`

**存储目录结构**：

```
mHost/
  manifest.json              # { "version": 1, "app_version": "0.1.0" }
  profiles/
    {profile_id}.json
  backups/
    hosts-{timestamp}.bak
  settings.json
```

阶段 0 不创建 `remote_sources/` 和 `cache/remote/`（阶段 3 范围）。

**Storage trait**：

```rust
pub trait Storage {
    fn load_profile(&self, id: &ProfileId) -> Result<Profile, StorageError>;
    fn save_profile(&self, profile: &Profile) -> Result<(), StorageError>;
    fn delete_profile(&self, id: &ProfileId) -> Result<(), StorageError>;
    fn list_profiles(&self) -> Result<Vec<Profile>, StorageError>;
    fn load_manifest(&self) -> Result<Manifest, StorageError>;
    fn save_manifest(&self, manifest: &Manifest) -> Result<(), StorageError>;
}
```

**原子写入实现**：

```rust
fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, content)?;
    fs::rename(&temp, path)?;
    Ok(())
}
```

**数据版本迁移**：

阶段 0 只有 v1，预留迁移 trait：

```rust
pub trait Migration {
    fn from_version(&self) -> u32;
    fn to_version(&self) -> u32;
    fn migrate(&self, data: Value) -> Result<Value, StorageError>;
}
```

**TDD 测试用例**：

```rust
#[test]
fn test_save_and_load_profile() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStorage::new(temp_dir.path());
    let profile = Profile::new("test");
    
    storage.save_profile(&profile).unwrap();
    let loaded = storage.load_profile(&profile.id).unwrap();
    
    assert_eq!(profile, loaded);
}

#[test]
fn test_list_profiles() {
    // 创建多个 Profile，验证 list_profiles 返回全部
}

#[test]
fn test_delete_profile() {
    // 保存后删除，验证 load 返回 ProfileNotFound
}

#[test]
fn test_atomic_write() {
    // 验证写入过程中断不会留下损坏文件
}

#[test]
fn test_manifest_version() {
    // 验证 manifest 版本为 1
}
```

**预估**：1 天

---

### T4：实现 hosts 解析器 + 测试

**类型**：TDD

**依赖**：T2

**内容**：

在 `mhost-hosts` 中实现标准 hosts 语法解析。

**解析器接口**：

```rust
pub struct Parser;

impl Parser {
    pub fn parse(input: &str) -> ParseResult {
        // 解析 hosts 文本，返回规则列表和错误列表
    }
    
    pub fn format(rules: &[HostRule]) -> String {
        // 将规则格式化为 hosts 文本
    }
    
    pub fn extract_managed_block(input: &str) -> Option<(usize, usize)> {
        // 返回托管区块的起止行号（含标记行）
    }
}

pub struct ParseResult {
    pub rules: Vec<HostRule>,
    pub errors: Vec<ParseError>,
}
```

**支持能力**：

- 标准 hosts 语法解析（IP + 域名 + 注释）
- IPv4 / IPv6 支持
- 多域名同行（`127.0.0.1 a.com b.com`）→ 解析为一条 `HostRule`，`domains` 包含多个
- 空行、注释行处理
- 语法错误标记（无效 IP、非法域名、格式错误）
- 反向格式化（`Vec<HostRule>` → hosts 文本）
- 托管区块识别（`# ---- mHost start ----` / `# ---- mHost end ----`）

**TDD 测试用例（表格驱动）**：

```rust
#[test]
fn test_parse_standard_line() {
    let cases = vec![
        ("ipv4_single", "127.0.0.1 example.com", vec![("127.0.0.1", vec!["example.com"])]),
        ("ipv6_single", "::1 localhost", vec![("::1", vec!["localhost"])]),
        ("ipv6_full", "2001:db8::1 example.com", vec![("2001:db8::1", vec!["example.com"])]),
        ("multi_domain", "127.0.0.1 a.com b.com", vec![("127.0.0.1", vec!["a.com", "b.com"])]),
        ("with_comment", "127.0.0.1 example.com # dev", vec![("127.0.0.1", vec!["example.com"])]),
    ];
    // 验证解析结果
}

#[test]
fn test_parse_errors() {
    let cases = vec![
        ("invalid_ip", "999.999.999.999 x.com", ParseError::InvalidIp("999.999.999.999".to_string())),
        ("invalid_domain", "127.0.0.1 -bad.com", ParseError::InvalidDomain("-bad.com".to_string())),
        ("malformed", "example.com 127.0.0.1", ParseError::MalformedLine("example.com 127.0.0.1".to_string())),
    ];
    // 验证错误类型
}

#[test]
fn test_parse_comment_and_empty() {
    let cases = vec![
        ("comment", "# this is a comment", 0),
        ("empty", "", 0),
        ("whitespace", "   ", 0),
    ];
    // 验证不产生规则
}

#[test]
fn test_extract_managed_block() {
    let input = "# line 1\n# ---- mHost start ----\n127.0.0.1 x.com\n# ---- mHost end ----\n# line 5";
    assert_eq!(extract_managed_block(input), Some((1, 3))); // 行号 0-based
}

#[test]
fn test_format_roundtrip() {
    let input = "127.0.0.1 example.com\n::1 localhost\n";
    let result = Parser::parse(input);
    let formatted = Parser::format(&result.rules);
    let reparsed = Parser::parse(&formatted);
    assert_eq!(result.rules, reparsed.rules);
}
```

**预估**：1.5 天

---

### T5：实现规则合并与冲突检测 + 测试

**类型**：TDD

**依赖**：T2, T4

**内容**：

在 `mhost-apply` 中实现规则合并引擎。

**阶段 0 简化**：

- 只支持单 Profile 启用（已确认决策）
- 不涉及远程规则和广告屏蔽（阶段 3 范围）
- 白名单作为预留概念，阶段 0 不实现

**合并逻辑**：

```rust
pub struct Merger;

impl Merger {
    pub fn merge(profiles: &[Profile]) -> MergeResult {
        // 合并所有启用的 Profile 的规则
    }
}

pub struct MergeResult {
    pub rules: Vec<ResolvedRule>,
    pub conflicts: Vec<RuleConflict>,
}
```

**冲突检测策略**：

- 同一域名映射到相同 IP：合并为一条（无冲突）
- 同一域名映射到不同 IP：标记冲突
- 冲突时保留所有变体，由用户决定

**生成 ApplyPlan**：

```rust
pub fn generate_plan(
    profiles: &[Profile],
    current_hosts: &str,
) -> Result<ApplyPlan, MhostError> {
    let merge_result = Merger::merge(profiles);
    let diff = calculate_diff(current_hosts, &merge_result.rules);
    
    Ok(ApplyPlan {
        rules: merge_result.rules,
        conflicts: merge_result.conflicts,
        diff,
        backup_required: true,
    })
}
```

**TDD 测试用例（表格驱动）**：

```rust
#[test]
fn test_merge_single_profile() {
    let profile = Profile::new("dev");
    // 添加规则...
    let result = Merger::merge(&[profile]);
    assert_eq!(result.rules.len(), /* 预期数量 */);
    assert!(result.conflicts.is_empty());
}

#[test]
fn test_merge_no_conflict() {
    let p1 = profile_with_rules("p1", vec![("127.0.0.1", "a.com")]);
    let p2 = profile_with_rules("p2", vec![("127.0.0.1", "b.com")]);
    let result = Merger::merge(&[p1, p2]);
    assert_eq!(result.rules.len(), 2);
    assert!(result.conflicts.is_empty());
}

#[test]
fn test_merge_same_domain_same_ip() {
    let p1 = profile_with_rules("p1", vec![("127.0.0.1", "x.com")]);
    let p2 = profile_with_rules("p2", vec![("127.0.0.1", "x.com")]);
    let result = Merger::merge(&[p1, p2]);
    assert_eq!(result.rules.len(), 1); // 合并为一条
    assert!(result.conflicts.is_empty());
}

#[test]
fn test_merge_same_domain_different_ip() {
    let p1 = profile_with_rules("p1", vec![("127.0.0.1", "x.com")]);
    let p2 = profile_with_rules("p2", vec![("192.168.1.1", "x.com")]);
    let result = Merger::merge(&[p1, p2]);
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.conflicts[0].domain, "x.com");
    assert_eq!(result.conflicts[0].rules.len(), 2);
}

#[test]
fn test_generate_managed_block() {
    let plan = generate_plan(/* ... */);
    let hosts_text = format_as_hosts(&plan.rules);
    assert!(hosts_text.contains("# ---- mHost start ----"));
    assert!(hosts_text.contains("# ---- mHost end ----"));
}
```

**预估**：1.5 天

---

### T6：实现系统 hosts 写入原型 + 测试

**类型**：TDD

**依赖**：T4, T5

**内容**：

在 `mhost-apply` 的 `writer` 模块中实现 macOS 系统 hosts 安全写入。

**模块归属**：`mhost-apply/src/writer.rs`，`platform` 作为子模块处理平台差异。

**macOS hosts 路径**：`/etc/hosts`

**写入流程**：

```rust
pub struct HostsWriter {
    hosts_path: PathBuf,
    backup_dir: PathBuf,
}

impl HostsWriter {
    pub fn new() -> Self {
        Self {
            hosts_path: PathBuf::from("/etc/hosts"),
            backup_dir: storage_root().join("backups"),
        }
    }
    
    pub fn apply(&self, plan: &ApplyPlan) -> Result<(), MhostError> {
        // 1. 读取当前系统 hosts
        let current = fs::read_to_string(&self.hosts_path)?;
        
        // 2. 检测 mHost 托管区块
        let has_managed = extract_managed_block(&current).is_some();
        
        // 3. 检测外部变更（简化：阶段 0 只检测托管区块外内容变化）
        
        // 4-7. 已由 ApplyPlan 完成（合并、校验、冲突检测、diff）
        
        // 8. 用户确认（UI 层，此处只接收已确认的计划）
        
        // 9. 创建备份
        let backup_path = self.create_backup(&current)?;
        
        // 10-12. 写入临时文件 → 校验 → 替换
        let new_content = self.build_hosts_content(&current, plan);
        self.atomic_write(&new_content)?;
        
        // 13. 刷新 DNS 缓存
        self.flush_dns_cache()?;
        
        // 14. 验证写入结果
        let written = fs::read_to_string(&self.hosts_path)?;
        self.verify(&written, plan)?;
        
        Ok(())
    }
    
    fn build_hosts_content(&self, current: &str, plan: &ApplyPlan) -> String {
        // 保留托管区块外的内容，替换托管区块
    }
    
    fn create_backup(&self, content: &str) -> Result<PathBuf, MhostError> {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let path = self.backup_dir.join(format!("hosts-{}.bak", timestamp));
        fs::write(&path, content)?;
        Ok(path)
    }
    
    fn atomic_write(&self, content: &str) -> Result<(), MhostError> {
        let temp = self.hosts_path.with_extension("tmp");
        fs::write(&temp, content)?;
        // 使用 osascript 弹窗授权执行 mv
        self.elevated_move(&temp, &self.hosts_path)?;
        Ok(())
    }
    
    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError> {
        let script = format!(
            "do shell script \"mv {} {}\" with administrator privileges",
            from.display(),
            to.display()
        );
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()?;
        
        if !output.status.success() {
            return Err(ApplyError::PermissionDenied(
                String::from_utf8_lossy(&output.stderr).to_string()
            ).into());
        }
        Ok(())
    }
    
    fn flush_dns_cache(&self) -> Result<(), MhostError> {
        std::process::Command::new("dscacheutil")
            .args(["-flushcache"])
            .output()?;
        Ok(())
    }
}
```

**TDD 测试用例（使用临时目录 mock）**：

```rust
#[test]
fn test_first_write_creates_managed_block() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "# original content\n").unwrap();
    
    let writer = TestHostsWriter::new(&hosts_path);
    let plan = /* 创建 ApplyPlan */;
    writer.apply(&plan).unwrap();
    
    let content = fs::read_to_string(&hosts_path).unwrap();
    assert!(content.contains("# original content")); // 保留原有内容
    assert!(content.contains("# ---- mHost start ----"));
    assert!(content.contains("# ---- mHost end ----"));
}

#[test]
fn test_update_replaces_managed_block() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "# before\n# ---- mHost start ----\n127.0.0.1 old.com\n# ---- mHost end ----\n# after\n").unwrap();
    
    let writer = TestHostsWriter::new(&hosts_path);
    let plan = /* 新 ApplyPlan */;
    writer.apply(&plan).unwrap();
    
    let content = fs::read_to_string(&hosts_path).unwrap();
    assert!(content.contains("# before"));
    assert!(content.contains("# after"));
    assert!(!content.contains("old.com")); // 旧规则被替换
}

#[test]
fn test_backup_created() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    let backup_dir = temp_dir.path().join("backups");
    fs::write(&hosts_path, "original").unwrap();
    
    let writer = TestHostsWriter::new_with_backup_dir(&hosts_path, &backup_dir);
    let plan = /* ... */;
    writer.apply(&plan).unwrap();
    
    let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().collect();
    assert_eq!(backups.len(), 1);
}

#[test]
fn test_rollback_restores_backup() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "original").unwrap();
    
    let writer = TestHostsWriter::new(&hosts_path);
    // 先应用
    let plan = /* ... */;
    writer.apply(&plan).unwrap();
    
    // 再回滚
    writer.rollback().unwrap();
    
    let content = fs::read_to_string(&hosts_path).unwrap();
    assert_eq!(content, "original");
}

#[test]
fn test_write_failure_preserves_original() {
    let temp_dir = TempDir::new().unwrap();
    let hosts_path = temp_dir.path().join("hosts");
    fs::write(&hosts_path, "original").unwrap();
    
    // 模拟权限不足（只读目录）
    let mut perms = fs::metadata(&hosts_path).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&hosts_path, perms).unwrap();
    
    let writer = TestHostsWriter::new(&hosts_path);
    let plan = /* ... */;
    let result = writer.apply(&plan);
    
    assert!(result.is_err());
    let content = fs::read_to_string(&hosts_path).unwrap();
    assert_eq!(content, "original"); // 原文件未被破坏
}
```

**预估**：1.5 天

---

### T7：搭建前端应用壳

**类型**：UI 初始化，非 TDD

**依赖**：T1

**内容**：

- 初始化 React + TypeScript + Vite（T1 已完成）
- 安装额外依赖：`pnpm add react-router-dom jotai`
- 配置 Tauri API 调用（见第 7.3 节）
- 实现基础页面布局：

```
App
├── Layout（侧边栏 + 主内容区）
│   ├── Sidebar
│   │   ├── Logo
│   │   ├── NavLink: Profiles
│   │   ├── NavLink: Settings
│   │   └── ApplyStatus
│   └── Main
│       ├── Routes
│       │   ├── /profiles → ProfileList
│       │   ├── /profiles/:id → ProfileEdit
│       │   └── /settings → Settings
│       └── Toast/Modal 容器
```

- Profile 列表页：展示所有 Profile，支持启用/禁用切换
- Profile 编辑页：展示 Profile 基本信息和规则列表（阶段 0 只读或简单编辑）
- 设置页：展示应用信息、存储路径等

- 状态管理（Jotai）：

```typescript
// src/stores/profiles.ts
import { atom } from 'jotai';
import type { Profile, ApplyPlan } from '../types';

export const profilesAtom = atom<Profile[]>([]);
export const selectedProfileIdAtom = atom<string | null>(null);
export const applyPlanAtom = atom<ApplyPlan | null>(null);
export const isApplyingAtom = atom(false);
```

**预估**：1 天

---

### T8：集成验收

**类型**：集成测试

**依赖**：T3, T4, T5, T6, T7

**内容**：

端到端验证完整流程：

1. 创建 Profile（通过前端或 Rust API）
2. 添加规则（通过 Rust API）
3. 启用 Profile
4. 生成 ApplyPlan（验证 diff 正确）
5. 应用规则（mock 文件系统，不操作真实 `/etc/hosts`）
6. 验证 mock hosts 内容包含托管区块
7. 验证备份存在
8. 执行回滚
9. 验证 mock hosts 恢复原状

**预估**：0.5 天

---

## 9. 任务依赖图

```
T1（工程初始化）
  ├── T2（数据模型）
  │     ├── T3（存储层）
  │     ├── T4（解析器）
  │     │     └── T5（合并引擎）
  │     │           └── T6（系统写入）
  │     └── T8（集成验收）
  └── T7（前端壳）
```

T8 依赖 T3（存储）、T4（解析）、T5（合并）、T6（写入）、T7（前端）。

---

## 10. 团队组建

采用方案 A：1 个 Backend Developer 串行开发，Frontend Developer 并行开发前端壳。

| 角色 | 数量 | 负责内容 |
|------|------|----------|
| Backend Developer | 1 | T2 → T3 → T4 → T5 → T6 串行开发 |
| Frontend Developer | 1 | T7 前端应用壳（与 Backend 并行） |
| Code Reviewer | 1 | 审查 TDD 测试完整性、代码质量 |

并行点：

- Frontend Developer 在 T1 完成后即可开始 T7，与 Backend 的 T2-T6 并行
- Code Reviewer 在每个任务完成后即时 Review，不阻塞后续任务

---

## 11. 阶段产出物

| 产出物 | 说明 |
|--------|------|
| `src-tauri/` 工程结构 | 完整的 Rust + Tauri v2 项目 |
| `src/` 前端工程 | React + Vite + Jotai 基础应用壳 |
| `mhost-core` crate | 核心数据模型与错误类型 |
| `mhost-hosts` crate | hosts 解析与格式化 |
| `mhost-storage` crate | 本地持久化存储 |
| `mhost-apply` crate | 规则合并、diff、系统写入 |
| `platform` 子模块 | macOS 系统适配 |
| 单元测试覆盖 | 核心逻辑表格驱动测试 |
| `.github/workflows/ci.yml` | 基础 CI 配置 |

---

## 12. 阶段 0 不做的事

| 排除项 | 原因 |
|--------|------|
| 广告屏蔽模块 | 阶段 3 范围 |
| 远程规则订阅 | 阶段 3 范围 |
| 白名单功能 | 阶段 3 范围（与广告屏蔽配套） |
| 本地 DNS 模式 | 技术决策已排除 |
| Windows / Linux 平台适配 | 阶段 1 扩展 |
| 语法高亮编辑器 | 阶段 2 范围 |
| 系统托盘 | 阶段 2 范围 |
| 导入导出 | 阶段 1/2 范围 |
| 诊断工具 | 阶段 4 范围 |
| 签名与发布 | 阶段 1 后期 |
| Privileged Helper | 阶段 1 评估 |

---

## 13. 验收标准

- [ ] Tauri 工程可正常编译运行
- [ ] `cargo test` 全部通过
- [ ] CI 检查（format、clippy、test、build）全部通过
- [ ] hosts 解析器支持标准语法、IPv6、错误标记
- [ ] 规则合并支持冲突检测（同域名不同 IP）
- [ ] 系统写入支持托管区块、备份、回滚
- [ ] 前端应用壳可展示 Profile 列表和编辑页
- [ ] 集成测试验证完整应用流程

---

## 14. 风险与应对

| 风险 | 影响 | 应对 |
|------|------|------|
| macOS 权限写入不稳定 | 高 | 阶段 0 先用弹窗授权，测试使用 mock 文件系统 |
| Tauri + Rust 构建耗时 | 中 | CI 配置 Cargo 缓存，本地开发增量编译 |
| 前端与 Rust 接口不匹配 | 低 | 接口契约已在计划中完整定义 |
| 大规则文件性能 | 低 | 阶段 0 先保证正确性，阶段 2 再优化 |
