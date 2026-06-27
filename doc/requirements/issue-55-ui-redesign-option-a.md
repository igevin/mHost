# Issue #55 UI 重设计方案 A

## 背景

当前 mHost 的 UI 布局存在两个核心问题：

1. **Profile 列表页占用空间过大**。展示 profile 列表及相关操作的页面占据整屏空间，而这些都是低频操作。用户更希望一打开应用就能看到当前的 hosts 规则内容。
2. **编辑页左右对称导致 Rules 区域过窄**。编辑页采用 `1fr 1fr` 的左右对称布局，左侧 Basic Info 修改频率极低却占用大量空间，右侧作为核心编辑区的 Rules 编辑器反而太窄。

本方案在不删除任何原有功能的前提下，重新分配界面空间的优先级。

## 设计目标

- **核心内容优先**：打开应用后，主要视觉空间展示当前 hosts 规则
- **管理功能收敛**：Profile 的 CRUD 操作收敛到可访问但不抢占主视觉的位置
- **编辑体验优化**：规则编辑器占据主要空间，Basic Info 收缩为可折叠的信息条
- **零功能损失**：统计面板、Profile 卡片详情（名称/描述/标签/规则数/状态）、启用/禁用切换、导出/复制/删除操作、新建 Profile 表单、导入对话框、空状态引导、StatusBar、Tools 导航、Basic Info 编辑等全部保留

## 整体布局

方案 A 采用 **左侧 Sidebar + 右侧主区域** 的三层架构：

```
+----------------------------------------------------------+
| Sidebar (240px)  |  Main Area                            |
|                  |                                       |
| - Logo           |  - Info Bar (可折叠 Basic Info)       |
| - Profile 列表    |  - Header (标题 + 操作按钮)            |
| - Tools 导航      |  - Rules Editor (主区域)              |
| - StatusBar      |                                       |
|                  |  [Management Drawer 从右侧滑出]       |
+----------------------------------------------------------+
```

## 左侧 Sidebar

Sidebar 为深色主题，高度与视口相同，包含四个区域：

### Logo 区
- mHost logo + 应用名称
- 固定在 Sidebar 顶部

### Profile 列表区
- 标题栏显示 "Profiles"，右侧有 "Manage ->" 链接
- 列表以紧凑形式展示所有 profile，每项包含：
  - 状态圆点（绿色 = Enabled，灰色 = Disabled）
  - Profile 名称
  - 简短描述（单行截断）
  - **Enable/Disable toggle 开关**（点击直接切换启用状态）
- 当前选中的 profile 有左侧蓝色指示条 + 高亮背景
- 列表底部有 "+ New Profile" 按钮

### Tools 导航区
- Ad Block（Soon，带 badge 计数）
- Remote Rules（Soon，带 badge 计数）
- Backup（Soon）
- Settings（可用）
- 不可用的工具显示 `Soon` 标记和禁用样式

### StatusBar
- 底部显示系统 hosts 状态（active/inactive）
- 已启用 profile 数量和总规则数

## 主区域

主区域分为三层：

### Info Bar（可折叠）
- 默认展开，以单行展示当前 profile 的 Basic Info：
  - Name
  - Description
  - Tags
- 右侧有 "Edit info ->" 按钮，点击后展开为可编辑表单
- 可折叠为只显示 profile 名称的一行

### Header
- 左侧：当前 profile 名称 + 状态 badges（Enabled/Protected/规则数）
- 右侧操作按钮：
  - Import
  - Export
  - Back（返回列表，或在上级导航中返回）
  - **Edit Rules**（默认显示，点击后进入编辑模式）
  - Apply to System

### Rules Editor（核心区域）

默认状态为**只读模式**：
- 标题栏显示 "Rules" + `Read-only` 灰色标签 + 规则数量
- 工具栏：Format、Validate
- 代码区域以语法高亮形式展示 hosts 规则，背景为浅灰色，无光标
- 支持滚动浏览、复制内容

点击 **Edit Rules** 后进入**编辑模式**：
- 标题栏 `Read-only` 标签变为 `Editing` 蓝色标签
- 代码区域变为可编辑 textarea，末尾显示闪烁光标
- 工具栏增加 Format、Validate
- Header 右侧按钮变为：Cancel（放弃修改）+ Save（有修改时才可用）+ Apply to System
- 点击 Save 后保存并自动回到只读模式
- 点击 Cancel 后放弃修改并回到只读模式

## Management Drawer（管理抽屉）

点击 Sidebar Profile 列表区的 "Manage ->" 后，从右侧滑出管理抽屉（宽度 520px），遮罩主区域：

### 统计面板
- Total Profiles
- Enabled
- Total Rules
- 三个卡片横向排列

### 操作区
- "+ New Profile" 按钮
- "Import" 按钮

### Profile 卡片列表
以完整卡片形式展示所有 profile，每张卡片包含：
- 名称 + 状态 badges（Enabled/Disabled/Protected）
- 规则数量 + 最后修改时间
- 描述
- 标签列表
- 操作按钮：Edit / Duplicate / Export / Delete

### 空状态
- 无 profile 时展示引导提示和创建入口

## 路由调整

当前路由结构：
- `/` -> 重定向到 `/profiles`
- `/profiles` -> ProfileList（整页列表）
- `/profiles/:id` -> ProfileEdit（编辑页）
- `/settings` -> Settings

方案 A 的路由调整：
- `/` -> 默认选中上次使用的 profile，展示其规则（主区域编辑器视图）
- `/profiles/:id` -> 选中指定 profile，展示其规则（主区域编辑器视图）
- `/settings` -> Settings（保持不变）

原有的 `/profiles` 列表页不再作为独立页面存在，其功能全部迁移到 Management Drawer 中。

## 交互流程

### 打开应用
1. 加载 Sidebar（Profile 列表 + Tools + StatusBar）
2. 主区域默认展示上次选中的 profile 的 hosts 规则（只读模式）
3. 用户直接看到规则内容，无需额外点击

### 切换 Profile
1. 在 Sidebar 点击其他 profile
2. 主区域即时切换为该 profile 的规则（保持只读模式）
3. Info Bar 同步更新

### 启用/禁用 Profile
1. 在 Sidebar 点击某 profile 右侧的 toggle switch
2. 开关状态切换，StatusBar 计数同步更新

### 编辑规则
1. 点击 Header 的 "Edit Rules"
2. 编辑器进入编辑模式（Editing 标签、光标出现）
3. 修改规则内容
4. 点击 Save 保存并回到只读模式；或点击 Cancel 放弃并回到只读模式

### 管理 Profile（新建/导入/删除/查看统计）
1. 点击 Sidebar 的 "Manage ->"
2. 右侧滑出 Management Drawer
3. 在抽屉内完成所有管理操作
4. 点击关闭按钮或遮罩区域收起抽屉

### 编辑 Basic Info
1. 点击 Info Bar 的 "Edit info ->"
2. Info Bar 展开为表单（Name / Description / Tags / Status）
3. 修改后保存，Info Bar 收起为单行展示

## 功能对照

| 原有功能 | 原位置 | 方案 A 位置 |
|---|---|---|
| Profile 列表（名称/描述/标签/规则数/状态） | `/profiles` 整页卡片 | Sidebar 紧凑列表 + Management Drawer 完整卡片 |
| Enable/Disable 切换 | ProfileCard 内 toggle | Sidebar 列表项右侧 toggle |
| 统计面板（Total/Enabled/Rules） | `/profiles` 顶部 | Management Drawer 顶部 |
| 新建 Profile | `/profiles` 内联表单 | Sidebar "+ New" 按钮 + Drawer 内按钮 |
| 导入 | ImportDialog 弹窗 | Drawer 内 Import 按钮 |
| 导出 | ProfileCard 内 Export 按钮 | Drawer 卡片内 Export 按钮 |
| 复制 | ProfileCard 内 Duplicate 按钮 | Drawer 卡片内 Duplicate 按钮 |
| 删除 | ProfileCard 内 Delete 按钮 | Drawer 卡片内 Delete 按钮 |
| Basic Info 编辑 | ProfileEdit 左侧表单 | Info Bar 展开编辑 |
| Rules 编辑器 | ProfileEdit 右侧 textarea | 主区域大面积编辑器 |
| Save 按钮 | ProfileEdit Header | 编辑模式下 Header |
| Format/Validate | ProfileEdit 编辑器内 | 编辑器工具栏（只读/编辑均有）|
| Apply to System | ProfileEdit Header | 主区域 Header（常驻）|
| Tools 导航（Settings 等）| Layout Sidebar | Sidebar 底部 Tools 区 |
| StatusBar | Layout 底部 | Sidebar 底部 |
| 空状态引导 | `/profiles` 无数据时 | Drawer 内无数据时 |

## 设计原则

1. **空间分层**：高频操作（查看规则、切换 profile、启用禁用）放在一级界面；低频操作（新建、删除、导出、查看统计）放在二级抽屉
2. **默认只读**：规则编辑器默认只读，减少误操作和界面噪音
3. **即时反馈**：切换 profile 时主区域即时更新，无需页面跳转
4. **渐进展开**：Info Bar 可折叠、Management Drawer 可滑出，信息按需展示
5. **功能无损**：所有原有功能均有对应入口，不删除任何能力
