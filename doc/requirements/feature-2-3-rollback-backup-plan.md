# 功能2 + 功能3：回滚入口前移 + 备份管理 — 开发计划

创建日期：2026-06-30

---

## 功能2：回滚入口前移到主界面

### 目标
用户在主界面即可执行回滚，不需要进入 Settings 页面。

### 现状
- `RollbackButton` 组件已存在（带确认弹窗），仅在 `Settings.tsx` 中使用
- `ApplyStatus` 组件位于主界面底部，显示 "Apply History"（上次应用时间）
- `rollbackHostsActionAtom` Jotai atom 已存在，调用 `rollbackHosts()` 后端命令
- 后端 `rollback_hosts` 命令已实现（`HostsWriter::rollback()` 恢复最新备份）

### 开发任务（纯前端）

#### 1. 修改 ApplyStatus 组件

**文件：** `src/components/ApplyStatus.tsx`

- 引入 `RollbackButton` 和 `rollbackHostsActionAtom`
- 在 "Apply History" 区域右侧添加 Rollback 按钮
- 回滚成功后刷新 `lastApplied`（设为 null 或重新获取）

```tsx
// 新增 import
import RollbackButton from "./RollbackButton";
import { rollbackHostsActionAtom } from "../stores/profiles";

// 在 ApplyStatus 内部
const rollback = useSetAtom(rollbackHostsActionAtom);

// 在 "Apply History" 区域
<div className={styles.statusItem}>
  <span className={styles.statusLabel}>Apply History</span>
  <span className={styles.statusValue}>
    {lastApplied ? formatDistanceToNow(...) : "Not applied yet"}
  </span>
  <RollbackButton onRollback={rollback} />
</div>
```

**样式调整：** `ApplyStatus.module.css` 中 `.statusItem` 改为 flex 布局，容纳标签、值、按钮。

#### 2. 从 Settings 页面移除 RollbackButton（可选）

**文件：** `src/pages/Settings.tsx`

- 移除 `RollbackButton` 的 import 和 `<RollbackButton onRollback={rollback} />` 使用
- 移除 `rollbackHostsActionAtom` 的 import 和 `const rollback = useSetAtom(...)`
- 保留 "Hosts Management" card 但只保留说明文字，或完全移除该 card

> **决策：** 如果 Settings 页面不再需要回滚功能，可以移除该 card，使 Settings 更简洁。如果希望保留两个入口，也可以不移除。

#### 3. RollbackButton 样式适配

`RollbackButton` 当前在 Settings card 中使用，移到 ApplyStatus 后可能需要调整大小（改为更紧凑的版本）。

- 选项 A：给 `RollbackButton` 添加 `size` prop（`"default" | "small"`）
- 选项 B：直接调整样式使其更紧凑

推荐选项 A，保持组件复用性。

---

## 功能3：备份管理（列表 + 选择版本回滚）

### 目标
用户可以查看所有备份记录并选择特定版本回滚。

### 现状
- 后端 `backup.rs` 中：
  - `create_backup` 创建带时间戳的备份（格式：`hosts-{timestamp}.bak`）
  - `prune_old_backups` 保留最近 10 份
  - `HostsWriter::rollback()` 只恢复最新备份
- 没有列出备份的 API
- 没有选择版本回滚的 API
- 前端没有备份相关 UI

### 开发任务

#### 后端任务

##### 1. 新增 BackupInfo 结构体

**文件：** `src-tauri/crates/mhost-core/src/models.rs`（或新建 `backup.rs`）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    pub id: String,           // 文件名（作为唯一标识）
    pub filename: String,     // hosts-YYYYMMDD_HHMMSS.bak
    pub timestamp: String,    // ISO 8601
    pub size: u64,            // 字节数
    pub path: String,         // 绝对路径
}
```

##### 2. 新增 list_backups 函数

**文件：** `src-tauri/crates/mhost-apply/src/writer/backup.rs`

```rust
pub fn list_backups(backup_dir: &Path) -> Result<Vec<BackupInfo>, MhostError> {
    let mut backups: Vec<BackupInfo> = Vec::new();
    
    for entry in fs::read_dir(backup_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("hosts-") || !name.ends_with(".bak") {
            continue;
        }
        
        let metadata = entry.metadata()?;
        let modified = metadata.modified()?;
        let timestamp = chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339();
        
        backups.push(BackupInfo {
            id: name.clone(),
            filename: name,
            timestamp,
            size: metadata.len(),
            path: entry.path().to_string_lossy().to_string(),
        });
    }
    
    // Sort by timestamp descending (newest first)
    backups.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    
    Ok(backups)
}
```

##### 3. 新增 rollback_to_backup 函数

**文件：** `src-tauri/crates/mhost-apply/src/writer/mod.rs`

在 `HostsWriter` impl 中新增：

```rust
/// Rollback to a specific backup file.
pub fn rollback_to_backup(&self, backup_path: &Path) -> Result<(), MhostError> {
    ensure_regular_file(&self.hosts_path)?;
    
    if !backup_path.exists() {
        return Err(ApplyError::BackupFailed(
            format!("backup not found: {}", backup_path.display())
        ).into());
    }
    
    let backup_content = fs::read_to_string(backup_path)?;
    self.atomic_write(&backup_content)?;
    
    // Verify
    let rolled_back = fs::read_to_string(&self.hosts_path)?;
    if rolled_back != backup_content {
        return Err(ApplyError::BackupFailed(
            "rollback content mismatch".to_string()
        ).into());
    }
    
    Ok(())
}
```

##### 4. 新增 Tauri Commands

**文件：** `src-tauri/src/commands/apply.rs`

```rust
#[tauri::command]
pub async fn list_backups(state: State<'_, AppState>) -> Result<Vec<BackupInfo>, MhostError> {
    let backup_dir = state.writer.backup_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        backup::list_backups(&backup_dir).map_err(Into::into)
    }).await.map_err(|e| MhostError::InvalidInput(e.to_string()))?
}

#[tauri::command]
pub async fn rollback_to_backup(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), MhostError> {
    let _guard = state.apply_lock.lock().await;
    let writer = state.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let backup_path = writer.backup_dir().join(&id);
        writer.rollback_to_backup(&backup_path).map_err(Into::into)
    }).await.map_err(|e| MhostError::InvalidInput(e.to_string()))?
}
```

> 注意：`HostsWriter` 目前没有 `backup_dir()` 方法，需要添加：
> ```rust
> pub fn backup_dir(&self) -> &Path { &self.backup_dir }
> ```

##### 5. 注册新命令

**文件：** `src-tauri/src/lib.rs`

在 `invoke_handler` 中添加：
```rust
list_backups,
rollback_to_backup,
```

##### 6. 后端测试

- `list_backups`：空目录、多个备份文件、非备份文件过滤、排序正确
- `rollback_to_backup`：正常回滚、备份不存在、内容校验失败

#### 前端任务

##### 1. 新增 BackupInfo 类型

**文件：** `src/types/index.ts`

```typescript
export interface BackupInfo {
  id: string;
  filename: string;
  timestamp: string; // ISO 8601
  size: number;
  path: string;
}
```

##### 2. 新增 Tauri 调用函数

**文件：** `src/lib/tauri.ts`

```typescript
export async function listBackups(): Promise<BackupInfo[]> {
  return invoke("list_backups");
}

export async function rollbackToBackup(id: string): Promise<void> {
  return invoke("rollback_to_backup", { id });
}
```

##### 3. 新增 BackupPanel 组件

**文件：** `src/components/BackupPanel.tsx`

- 显示备份列表（表格或卡片）
- 每行显示：时间戳、文件大小、回滚按钮
- 空状态："No backups yet"
- 加载状态
- 回滚确认弹窗（类似于 RollbackButton 的确认）
- 回滚成功后刷新列表

```tsx
interface BackupPanelProps {
  onRollback?: () => void;
}

function BackupPanel({ onRollback }: BackupPanelProps) {
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [confirmBackup, setConfirmBackup] = useState<BackupInfo | null>(null);
  // ...
}
```

##### 4. 新增样式

**文件：** `src/components/BackupPanel.module.css`

- 表格样式
- 行 hover 效果
- 回滚按钮样式
- 空状态样式

##### 5. 在 Settings 页面添加入口

**文件：** `src/pages/Settings.tsx`

在 "Hosts Management" card 下方或替换它，添加 BackupPanel：

```tsx
<div className="card">
  <h3 className="card-title">Backup Management</h3>
  <p className={styles.sectionDesc}>
    View and restore previous versions of your hosts file.
  </p>
  <BackupPanel onRollback={rollback} />
</div>
```

##### 6. 前端测试

- BackupPanel 渲染空状态
- 备份列表渲染正确
- 回滚按钮触发确认弹窗
- 确认后调用 `rollbackToBackup`
- 回滚成功后刷新列表

---

## 涉及文件清单

### 后端

| 文件 | 操作 | 说明 |
|------|------|------|
| `src-tauri/crates/mhost-core/src/models.rs` | 修改 | 新增 `BackupInfo` 结构体 |
| `src-tauri/crates/mhost-apply/src/writer/backup.rs` | 修改 | 新增 `list_backups` 函数 |
| `src-tauri/crates/mhost-apply/src/writer/mod.rs` | 修改 | 新增 `rollback_to_backup` 方法，新增 `backup_dir()` getter |
| `src-tauri/src/commands/apply.rs` | 修改 | 新增 `list_backups` 和 `rollback_to_backup` Tauri commands |
| `src-tauri/src/lib.rs` | 修改 | 注册新 commands |

### 前端

| 文件 | 操作 | 说明 |
|------|------|------|
| `src/types/index.ts` | 修改 | 新增 `BackupInfo` 接口 |
| `src/lib/tauri.ts` | 修改 | 新增 `listBackups` 和 `rollbackToBackup` 函数 |
| `src/components/BackupPanel.tsx` | 新建 | 备份列表 + 回滚 UI |
| `src/components/BackupPanel.module.css` | 新建 | 备份面板样式 |
| `src/components/ApplyStatus.tsx` | 修改 | 添加 RollbackButton |
| `src/components/ApplyStatus.module.css` | 修改 | 调整布局 |
| `src/components/RollbackButton.tsx` | 修改 | 可选：添加 `size` prop |
| `src/pages/Settings.tsx` | 修改 | 移除/替换 RollbackButton 为 BackupPanel |
| `src/components/__tests__/BackupPanel.test.tsx` | 新建 | 备份面板测试 |
| `src/components/__tests__/ApplyStatus.test.tsx` | 修改 | 新增回滚按钮测试 |

---

## 工作量评估

| 任务 | 预估 |
|------|------|
| 后端：BackupInfo + list_backups + rollback_to_backup | 1.5h |
| 后端：Tauri commands + 注册 | 0.5h |
| 后端：测试 | 1h |
| 前端：BackupPanel 组件 + 样式 | 2h |
| 前端：ApplyStatus 集成 RollbackButton | 0.5h |
| 前端：Settings 页面调整 | 0.5h |
| 前端：测试 | 1h |
| **总计** | **约 7h（1 天）** |

---

## 风险与注意事项

1. **backup_dir 路径暴露：** `BackupInfo.path` 返回绝对路径，前端可以查看但不应允许用户通过前端直接修改备份文件。
2. **回滚权限：** `rollback_to_backup` 同样需要 macOS 授权（写入 `/etc/hosts`），与现有 `rollback_hosts` 一致。
3. **并发安全：** `rollback_to_backup` 使用 `apply_lock`，与 apply/rollback 互斥。
4. **Settings 页面 card 调整：** 移除 RollbackButton 后，Settings 页面布局可能需要调整。
