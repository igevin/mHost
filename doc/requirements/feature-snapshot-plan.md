# Feature: Configuration Snapshot（配置快照）

## 背景

原有 Backup/Rollback 机制备份的是系统 `/etc/hosts` 文件内容，回滚时只恢复 hosts 文件，不恢复各 Profile 的 enabled 状态和 rules 配置。这会导致 Profile 配置与系统 hosts 状态不一致。

方案 B 将 Backup 重新设计为**配置快照机制**：快照保存的是所有 Profile 的完整配置（rules + enabled 状态），回滚时先恢复配置再自动 Apply。

## 设计目标

1. **配置级快照**：快照包含所有 Profile 的完整 JSON 序列化
2. **一键回滚**：恢复快照后自动 apply，确保 hosts 文件与配置一致
3. **轻量列表**：快照列表只展示元数据（id/name/count/created_at），不加载完整 profiles
4. **最大数量限制**：最多保留 20 个快照，自动清理最旧的

## 数据模型

### Rust — `mhost-core/src/models.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub profiles: Vec<Profile>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotMeta {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub profile_count: usize,
    pub created_at: DateTime<Utc>,
}
```

### TypeScript — `src/types/index.ts`

```typescript
export interface Snapshot {
  id: string;
  name: string;
  description?: string;
  profiles: Profile[];
  created_at: string;
}

export interface SnapshotMeta {
  id: string;
  name: string;
  description?: string;
  profile_count: number;
  created_at: string;
}
```

## 存储位置

```
~/Library/Application Support/mHost/
  snapshots/
    {uuid}.json
```

每个快照一个 JSON 文件，使用 `atomic_write` 写入。

## 后端 API

新增 `src-tauri/src/commands/snapshot.rs`：

| Command | 参数 | 返回 | 说明 |
|---------|------|------|------|
| `save_snapshot` | name, description? | SnapshotMeta | 读取当前所有 profiles，序列化为 Snapshot 保存 |
| `list_snapshots` | — | Vec<SnapshotMeta> | 读取 snapshots 目录，解析元数据（不加载 profiles） |
| `load_snapshot` | id | () | 读取快照 → 删除当前所有 profiles → 保存快照 profiles → apply_current_plan |
| `delete_snapshot` | id | () | 删除快照文件 |

`load_snapshot` 实现要点：
1. 使用 `apply_lock` 防止并发
2. 读取快照文件，获取 `profiles` 列表
3. 删除当前 storage 中所有 profiles（通过 `list_profiles` + `delete_profile`）
4. 保存快照中的所有 profiles（通过 `save_profile`）
5. 调用 `apply_current_plan_logic(storage, writer)` 自动 apply
6. 返回成功后前端刷新 profile 列表

`save_snapshot` 实现要点：
1. 读取当前所有 profiles
2. 生成 UUID 作为快照 id
3. 写入 `{root}/snapshots/{id}.json`
4. 如果总数超过 `MAX_SNAPSHOTS = 20`，删除最旧的
5. 返回 SnapshotMeta

`list_snapshots` 实现要点：
1. 遍历 `snapshots/` 目录下的 `.json` 文件
2. 对每个文件只解析元数据（id, name, description, profile_count, created_at）
3. 按 created_at 降序排列
4. 损坏的文件静默跳过

## 前端设计

### 路由
- App.tsx 添加 `<Route path="/snapshot" element={<SnapshotPage />} />`

### 导航
- Layout.tsx 中 `toolNavItems` 的 Backup 项改为：
  - `to: "/snapshot"`
  - `label: "Snapshots"`
  - `disabled: false`
  - 使用 Camera/Archive 图标（现有 BackupIcon 可复用）

### 页面 — `src/pages/Snapshot.tsx`
- 标题栏 + 创建快照按钮
- SnapshotPanel 组件

### 组件 — `src/components/SnapshotPanel.tsx`
- 快照列表（名称、描述、Profile 数量、创建时间）
- 每个快照的操作：回滚、删除
- 回滚前需要确认对话框（"回滚到快照 \"{name}\"? 当前配置将被覆盖。"）
- 创建快照对话框（输入名称、可选描述）
- 空状态提示

### Store — `src/stores/profiles.ts` 或新建 `src/stores/snapshots.ts`
- 考虑到复用现有 patterns，在 profiles store 中新增：
  - `snapshotsAtom`
  - `fetchSnapshotsAtom`
  - `saveSnapshotAtom`
  - `loadSnapshotAtom`
  - `deleteSnapshotAtom`

### Tauri 桥接 — `src/lib/tauri.ts`
- `saveSnapshot(name, description?)`
- `listSnapshots()`
- `loadSnapshot(id)`
- `deleteSnapshot(id)`

## 测试要求

### 后端测试
- `test_save_snapshot_creates_file`
- `test_save_snapshot_prunes_old`
- `test_list_snapshots_returns_meta_only`
- `test_list_snapshots_sorted_by_date`
- `test_load_snapshot_restores_profiles`
- `test_load_snapshot_applies_hosts`
- `test_delete_snapshot_removes_file`
- `test_load_snapshot_respects_apply_lock`

### 前端测试
- SnapshotPanel 渲染空状态
- 创建快照对话框提交
- 回滚确认对话框显示/取消/确认
- 删除快照按钮点击

## 文件变更清单

### 新增
- `src-tauri/src/commands/snapshot.rs`
- `src/pages/Snapshot.tsx`
- `src/pages/Snapshot.module.css`
- `src/components/SnapshotPanel.tsx`
- `src/components/SnapshotPanel.module.css`

### 修改
- `src-tauri/crates/mhost-core/src/models.rs` — 添加 Snapshot, SnapshotMeta
- `src-tauri/src/commands/mod.rs` — 添加 snapshot 模块
- `src-tauri/src/lib.rs` — 注册新命令
- `src/types/index.ts` — 添加类型
- `src/lib/tauri.ts` — 添加桥接函数
- `src/App.tsx` — 添加路由
- `src/components/Layout.tsx` — 启用 Snapshot 导航
- `src/stores/profiles.ts` — 添加快照相关 atoms

### 废弃（不做修改，保留现状）
- `src-tauri/crates/mhost-apply/src/writer/backup.rs` — 系统 hosts 文件备份机制保留，与快照机制独立
