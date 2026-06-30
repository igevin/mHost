# 阶段 1 收尾 + 阶段 2 开发计划

创建日期：2026-06-29

## 范围说明

阶段 1（MVP）核心功能已完成约 85%，阶段 2（编辑效率与安全感）大部分功能经评估后裁剪不做。本计划涵盖阶段 1 的收尾工作和精简后的阶段 2，共 5 项功能。

**明确不做的功能：** Windows 适配、多 Profile 同时启用、回收站、修改历史、表格化规则编辑器、增量合并导入、错误处理架构重构。

---

## 功能 1：ApplyConfirmDialog 集成 + diff 预览

**目标：** 用户点击 Apply 时能看到变更预览并确认，而不是直接执行。

**现状：**
- `ApplyConfirmDialog` 组件已存在（`src/components/ApplyConfirmDialog.tsx`），但没有任何页面引用它
- 组件当前只有 applying / success / error 三个状态，缺少确认步骤和 diff 展示
- 后端 `generate_apply_plan` 已返回 `ApplyPlan { diff, conflicts, backup_required, rules }`
- `ApplyStatus` 组件中已调用 `generateApplyPlan` 获取 pending changes 统计
- 前端实际通过 `enable_and_apply` 直接执行，跳过确认流程

**开发任务：**

### 后端
- 无新增后端任务，`generate_apply_plan` 已返回 `HostsDiff { added, removed }`

### 前端
1. **重构 ApplyConfirmDialog**，增加 pending 状态（等待用户确认）
   - pending 状态展示：diff 预览（新增行绿色、删除行红色、未变更行灰色）
   - 冲突列表提示（如有冲突，高亮警告）
   - 确认（Apply）和取消（Cancel）两个按钮
   - 保留现有 applying / success / error 状态
2. **在 ProfileView 中集成 ApplyConfirmDialog**
   - 用户点击 Apply/切换 Profile 时，先调用 `generate_apply_plan`
   - 弹出 ApplyConfirmDialog 展示 diff 和冲突
   - 用户确认后调用 `enable_and_apply` 执行
   - 存在冲突时禁用 Apply 按钮（或加警告让用户确认风险）
3. **移除 Settings 中的回滚入口**（回滚入口移至功能 2）

**涉及文件：**
- `src/components/ApplyConfirmDialog.tsx` — 重构
- `src/pages/ProfileView.tsx` — 集成对话框
- `src/stores/profiles/actions.ts` — 拆分 apply 流程（generate plan → confirm → apply）

---

## 功能 2：回滚入口前移到主界面

**目标：** 用户在主界面即可执行回滚，不需要进入 Settings 页面。

**现状：**
- `RollbackButton` 组件仅在 `src/pages/Settings.tsx` 中使用
- `ApplyStatus` 组件（主界面底部）纯展示型，无任何操作按钮
- 后端 `rollback_hosts` 命令已实现，恢复最新备份文件
- 备份文件位于 `~/Library/Application Support/mHost/backups/`，最多保留 10 份

**开发任务：**

### 前端
1. **在 ApplyStatus 的 "Apply History" 区域添加 Rollback 按钮**
   - 与 "Last Applied" 信息同行或下方
   - 点击触发确认对话框（复用 RollbackButton 的确认逻辑）
   - 确认后调用 `rollback_hosts`
   - 需要添加 loading 状态防止重复点击
2. **从 Settings 页面移除 RollbackButton**
   - 或保留为"高级选项"，但主入口在 ApplyStatus

**涉及文件：**
- `src/components/ApplyStatus.tsx` — 添加回滚按钮
- `src/pages/Settings.tsx` — 移除或弱化回滚入口

---

## 功能 3：备份管理（列表 + 选择版本回滚）

**目标：** 用户可以查看所有备份记录并选择特定版本回滚。

**现状：**
- 后端 `create_backup` 自动创建（apply 时触发），文件名 `hosts-YYYYMMDD_HHMMSS.bak`
- `prune_old_backups` 保留最近 10 份
- `rollback_hosts` 只恢复最新一份
- 没有列出备份、获取备份信息、选择版本回滚的 API
- 前端没有备份相关的 UI

**开发任务：**

### 后端
1. **新增 `BackupInfo` 结构体和 `list_backups` 函数**（`src-tauri/crates/mhost-apply/src/writer/backup.rs`）
   - 扫描 backups 目录，返回 `Vec<BackupInfo>`，每条包含：文件名、创建时间、文件大小
   - 按创建时间降序排列
2. **新增 `rollback_to_backup` 函数**（`backup.rs`）
   - 接受备份文件名参数，读取指定备份内容并写入 `/etc/hosts`
   - 验证写入后的内容与备份一致
3. **新增 Tauri commands**（`src-tauri/src/commands/apply.rs`）
   - `list_backups` → `Result<Vec<BackupInfo>, MhostError>`
   - `rollback_to_backup(filename: String)` → `Result<(), MhostError>`
   - 修改现有 `rollback_hosts` 为调用 `rollback_to_backup(最新文件名)` 的便捷封装

### 前端
1. **新增 BackupPanel 组件**（`src/components/BackupPanel.tsx`）
   - 展示备份列表：时间、文件大小
   - 每行一个 "Restore" 按钮
   - 点击 Restore 弹出确认对话框
   - 确认后调用 `rollback_to_backup`
2. **在 ApplyStatus 或 Settings 中添加入口**
   - ApplyStatus 的回滚按钮旁增加"查看所有备份"链接
   - 或在 Settings 的 "Hosts Management" 卡片中嵌入 BackupPanel
   - 使用 `createPortal` 渲染（遵循项目对话框规范）

**涉及文件：**
- `src-tauri/crates/mhost-apply/src/writer/backup.rs` — 新增 list、selective rollback
- `src-tauri/src/commands/apply.rs` — 新增 commands
- `src/components/BackupPanel.tsx` — 新建
- `src/components/ApplyStatus.tsx` 或 `src/pages/Settings.tsx` — 添加入口
- `src/lib/tauri.ts` — 新增 Tauri 函数封装

---

## 功能 4：查找和替换

**目标：** 用户可以在 RuleEditor 中搜索规则并批量替换 IP 或域名。

**现状：**
- `RuleEditor` 是 textarea + 语法高亮 overlay 的叠加方案
- 无内置查找替换功能
- 浏览器原生 Ctrl+F/Cmd+F 可搜索但不能替换，且搜索结果不在高亮层显示
- `highlightText()` 函数生成高亮 HTML，可注入搜索匹配的 span

**开发任务：**

### 前端
1. **新增 SearchBar 组件**（`src/components/SearchBar.tsx`）
   - 输入框（搜索关键词）+ 替换输入框（可折叠）
   - 搜索结果计数（如 "3/12"）
   - 上一个 / 下一个导航按钮
   - Replace / Replace All 按钮
   - 大小写敏感 / 正则表达式 开关（可选，MVP 可不做）
   - 快捷键：Cmd+F 打开、Esc 关闭、Enter 下一个、Shift+Enter 上一个
2. **在 RuleEditor 中集成 SearchBar**
   - 搜索栏定位在编辑器顶部工具栏区域
   - 搜索匹配时在高亮 overlay 中标记匹配项（黄色背景）
   - 当前匹配项高亮（橙色背景）
   - Replace 操作修改 textarea 内容并触发 `onChange`
   - Replace All 一次性替换所有匹配项
3. **搜索匹配的滚动定位**
   - 切换匹配项时，textarea 滚动到对应行

**涉及文件：**
- `src/components/SearchBar.tsx` — 新建
- `src/components/RuleEditor.tsx` — 集成搜索栏和匹配高亮

### 后端
- 无新增后端任务

---

## 功能 5：重复规则检测

**目标：** 同一 Profile 内存在重复域名时给出提示，帮助用户发现配置错误。

**现状：**
- 后端 `merge.rs` 已实现跨 Profile 的 (domain, ip) 去重
- 但不检测同一 Profile 内的重复域名（同一域名出现多次，无论 IP 是否相同）
- 前端 `validate_hosts_text` 已有语法校验（无效 IP、非法域名、格式错误），无重复检测
- `RuleEditor` 的错误列表只展示语法校验结果

**开发任务：**

### 后端
1. **新增重复检测逻辑**（`src-tauri/crates/mhost-hosts/src/validator.rs`）
   - 在现有 `validate` 流程中增加重复域名检测
   - 输出：重复域名列表，每条包含：行号、域名、重复出现行号
   - 区分两种情况：同域名同 IP（冗余规则）和同域名不同 IP（可能冲突）
2. **扩展 `ValidationError` 类型**（如需）
   - 新增 `DuplicateDomain` 变体，或复用现有 warning 类型

### 前端
1. **在 RuleEditor 错误列表中展示重复域名**
   - 复用现有错误列表 UI
   - 冗余规则用 warning 样式（黄色），可能冲突用 error 样式（红色）
   - 点击错误项跳转到对应行（如支持）

**涉及文件：**
- `src-tauri/crates/mhost-hosts/src/validator.rs` — 新增重复检测
- `src-tauri/src/commands/validate.rs` — 如需调整返回类型
- `src/components/RuleEditor.tsx` — 展示重复检测结果

---

## 开发顺序建议

建议按依赖关系和用户价值排序：

| 顺序 | 功能 | 预估工作量 | 依赖 |
|------|------|-----------|------|
| 1 | ApplyConfirmDialog 集成 + diff 预览 | 中 | 无 |
| 2 | 回滚入口前移到主界面 | 小 | 无 |
| 3 | 备份管理（列表 + 选择版本回滚） | 中 | 功能 2（回滚入口统一设计） |
| 4 | 查找和替换 | 中 | 无 |
| 5 | 重复规则检测 | 小 | 无 |

功能 1、2、5 可以并行开发，功能 3 依赖功能 2 的 UI 设计决策，功能 4 独立。
