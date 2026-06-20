# PR #5 代码审查报告

**仓库**: igevin/mHost  
**PR**: fix: 启用 Profile 时直接写入 hosts 并修复授权弹窗 (#4)  
**分支**: fix/issue-4-apply-on-enable -> master  
**变更**: 18 个文件，+182/-461 行

---

## 1. 正确性

**评分**: 需改进

### 1.1 `enable_and_apply` 后端命令

**文件**: `src-tauri/src/commands/apply.rs` (第 48-90 行)

该命令将"启用/禁用 Profile"和"写入 hosts"合并为单一原子操作，整体流程正确：

- 启用时先禁用其他 Profile（Phase 0 约束），再设置目标 Profile 为 enabled
- 禁用（`enabled = false`）时直接设置目标 Profile 为 disabled
- 重新加载所有 Profile 后生成 ApplyPlan
- 调用 `state.writer.apply(&plan)` 写入系统 hosts
- 成功后再写入 `last_applied` 时间戳

**问题**：该命令与 `set_profile_enabled`（`src-tauri/src/commands/profile.rs` 第 39-61 行）存在**重复逻辑**。两者都实现了"启用时禁用其他 Profile"的相同逻辑，只是 `enable_and_apply` 多了一步写入 hosts。建议将这段逻辑提取为共享函数，避免维护两份相同的代码。

### 1.2 禁用 Profile 时清除 managed block

**评分**: 通过

当 `enabled = false` 时：
- 目标 Profile 被设为 disabled
- `generate_plan` 会合并所有 enabled Profile 的规则（此时为空）
- `format_as_hosts(&[])` 返回空字符串
- `build_hosts_content` 检测到空 managed block 后会**移除**现有的 managed block 标记（`src-tauri/crates/mhost-apply/src/writer/content.rs` 第 44-47 行）

已有测试覆盖：`test_apply_empty_rules_removes_managed_block`（`src-tauri/crates/mhost-apply/src/writer/tests.rs` 第 226-245 行）。

### 1.3 `atomic_write` 的临时文件方案

**文件**: `src-tauri/crates/mhost-apply/src/writer/mod.rs` (第 268-281 行)

**评分**: 通过

PR 将临时文件创建位置从"目标文件所在目录"改为"系统临时目录"：

```rust
let mut temp_file = tempfile::NamedTempFile::new()?;
```

这是正确的修复。原方案在 `/etc` 目录下创建临时文件会因权限不足而失败（非 root 用户）。`tempfile`  crate 会在系统临时目录（如 `/tmp`）创建文件，该目录对所有用户可写。

`tempfile::NamedTempFile` 在 drop 时会自动清理，即使 `elevated_move` 失败也不会泄漏临时文件。

### 1.4 `elevated_move` 的 `cp + rm` 方案

**文件**: `src-tauri/crates/mhost-apply/src/platform/macos.rs` (第 20-40 行)

**评分**: 通过

PR 将 `mv` 改为 `cp + rm`：

```rust
"do shell script \"cp {} {} && rm {}\" with administrator privileges"
```

这是正确的修复。因为临时文件现在创建在 `/tmp`（或其他临时目录），与目标 `/etc/hosts` 可能位于不同文件系统，`mv` 会失败。`cp + rm` 可以正确处理跨设备移动。

潜在问题：`cp` 不保留原文件的精确权限位（如特殊权限位），但 `/etc/hosts` 通常只需要标准权限，影响有限。建议后续考虑使用 `cp -p` 保留权限，或显式 `chmod` 为目标权限。

---

## 2. 错误处理

**评分**: 需改进

### 2.1 后端命令错误处理

**文件**: `src-tauri/src/commands/apply.rs` (第 71-80 行)

`enable_and_apply` 中读取 `/etc/hosts` 的错误处理是正确的：

```rust
let current_hosts = match std::fs::read_to_string("/etc/hosts") {
    Ok(content) => content,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
    Err(e) => { return Err(MhostError::Io { ... }); }
};
```

但存在一个问题：**如果 `write_last_applied` 失败，错误被静默忽略**（仅打印 warning）。虽然注释说明这是"non-fatal"，但如果写入失败，用户界面上的"last applied"时间戳会显示旧值，造成状态不一致。建议至少将 warning 提升到 error 级别，或考虑将 `last_applied` 写入失败作为非阻塞错误返回给前端。

### 2.2 前端 Optimistic Update 回滚逻辑

**文件**: `src/stores/profiles/actions.ts` (第 110-151 行)

```typescript
// Optimistic UI update
if (newEnabled) {
  set(profilesAtom, (prev) =>
    prev.map((p) => (p.id === id ? { ...p, enabled: true } : { ...p, enabled: false })),
  );
} else {
  set(profilesAtom, (prev) =>
    prev.map((p) => (p.id === id ? { ...p, enabled: false } : p)),
  );
}

// ...

catch (err) {
  // Revert optimistic update
  set(profilesAtom, (prev) =>
    prev.map((p) => (p.id === id ? target : p)),
  );
  throw err;
}
```

**问题**：回滚逻辑只恢复了目标 Profile 的状态，但**没有恢复其他 Profile 的状态**。如果启用 Profile A 时 optimistically 禁用了 Profile B，当 `enableAndApply` 失败时，Profile B 仍然显示为 disabled，而实际后端状态并未改变。

**建议**：保存完整的 `profiles` 快照用于回滚，或至少保存所有被修改的 Profile 的原始状态。

### 2.3 `extractErrorMessage` 覆盖范围

**文件**: `src/lib/error.ts`

**评分**: 需改进

```typescript
if (typeof obj.Parse === "string") return obj.Parse;
if (typeof obj.Apply === "string") return obj.Apply;
if (typeof obj.Storage === "string") return obj.Storage;
if (typeof obj.InvalidInput === "string") return obj.InvalidInput;
```

**问题 1**：`MhostError::Io` 的序列化格式是 `{ "Io": { "kind": "...", "message": "..." } }`（externally tagged enum），但 `extractErrorMessage` 只检查了 `obj.message`，没有处理 `obj.Io` 这种嵌套结构。如果 Tauri 返回的 error 对象中 `message` 字段不存在于顶层，而是嵌套在 `Io` 对象内，该函数会 fallback 到 `JSON.stringify`，用户体验不佳。

**问题 2**：`MhostError` 还有 `Parse`、`Apply`、`Storage` 等变体，它们的序列化格式也是 externally tagged（如 `{ "Parse": "..." }`），这部分处理是正确的。

**建议**：增加对 `obj.Io?.message` 的检查：

```typescript
if (typeof obj.Io === "object" && obj.Io !== null && typeof (obj.Io as Record<string, unknown>).message === "string") {
  return (obj.Io as Record<string, unknown>).message as string;
}
```

---

## 3. 安全性

**评分**: 通过

### 3.1 `osascript` 命令注入风险

**文件**: `src-tauri/crates/mhost-apply/src/platform/macos.rs`

路径经过 `escape_applescript_path` 处理：
- 先通过 `validate_path_characters` 限制允许的字符集（`[a-zA-Z0-9/._-\\"]`）
- 再将反斜杠和双引号进行转义

字符白名单已排除了常见的 shell 注入字符（`;`, `|`, `$`, 空格等）。即使攻击者能控制临时文件路径（`tempfile` 生成的路径是安全的），也无法注入恶意命令。

### 3.2 临时文件安全

**文件**: `src-tauri/crates/mhost-apply/src/writer/mod.rs`

`tempfile::NamedTempFile::new()` 使用：
- 随机文件名（避免预测）
- 创建时设置 0o600 权限（仅所有者可读写）
- 自动清理（drop 时删除）

安全方面没有问题。

---

## 4. 代码质量

**评分**: 需改进

### 4.1 死代码 / 未使用的 import

**文件**: `src/stores/profiles/state.ts` (第 8 行)

```typescript
export const applyPlanAtom = atom<ApplyPlan | null>(null);
```

`applyPlanAtom` 在 PR 后**不再被任何业务逻辑使用**（`generateApplyPlanActionAtom` 和 `applyHostsActionAtom` 已被删除），但：
- 仍然在 `state.ts` 中定义
- 仍然在 `src/stores/profiles/index.ts` 中导出
- 仍然在 `src/stores/__tests__/profiles.test.ts` 中测试
- `ApplyPlan` 类型在 `state.ts` 的 import 中仍然被引用

**建议**：清理 `applyPlanAtom` 及其相关引用，或至少标记为 deprecated。

**文件**: `src-tauri/crates/mhost-apply/src/writer/mod.rs`

`ElevatedMover` trait 和 `OsascriptMover` 结构体被标记为 `#[deprecated]`，但仍然保留在代码中。虽然这是为了向后兼容，但 `OsascriptMover` 的 `elevated_move` 使用的是旧的 `mv` 命令（而非修复后的 `cp + rm`），如果还有测试或代码在使用它，可能会遇到跨设备移动的问题。建议确认是否还有使用方，如果没有则考虑移除。

### 4.2 重复逻辑

**文件**: 
- `src-tauri/src/commands/apply.rs` (第 55-67 行)
- `src-tauri/src/commands/profile.rs` (第 39-61 行)

`enable_and_apply` 和 `set_profile_enabled` 共享完全相同的"启用时禁用其他 Profile"逻辑。建议提取为 `storage` 模块的辅助函数，如 `Storage::set_profile_enabled_exclusive(id, enabled)`。

### 4.3 命名

**评分**: 通过

- `enable_and_apply` 命名清晰，表达了"启用并应用"的语义
- `extractErrorMessage` 准确地描述了函数职责
- `elevated_move` 虽然实际做的是 `cp + rm`，但命名仍合理（从调用方视角看是"移动到目标位置"）

---

## 5. 测试覆盖

**评分**: 有问题

### 5.1 `enable_and_apply` 命令测试

**严重缺失**：PR 新增的核心命令 `enable_and_apply` **没有任何单元测试或集成测试**。

该命令包含以下复杂逻辑，都需要测试覆盖：
1. 启用 Profile A 时，是否正确禁用了其他已启用的 Profile
2. 禁用 Profile 时，是否正确清除了 managed block
3. 命令失败时，storage 状态是否保持一致（不应出现"Profile 已启用但 hosts 未写入"的中间状态）
4. `last_applied` 时间戳是否正确写入

**建议**：在 `src-tauri/src/commands/` 下新增 `apply_tests.rs` 或在现有集成测试中增加 `enable_and_apply` 的测试用例。

### 5.2 组件测试

**文件**: `src/components/__tests__/ApplyConfirmDialog.test.tsx`

PR 删除了大量与 diff preview 相关的测试（从 156 行减至 77 行），这是合理的因为 UI 已经移除了这些功能。但剩余的测试仅覆盖了：
- 显示 applying 进度
- 显示成功状态
- 显示失败状态和回滚按钮

**缺失**：没有测试 `ApplyConfirmDialog` 在 `open` 为 `false` 时不渲染的情况（虽然简单，但属于基础覆盖）。

**文件**: `src/components/__tests__/StatusBar.test.tsx`

删除了与 `applyPlanAtom` 和 `onApply` 相关的测试，新增了一个简单的"不显示 Applying"测试。覆盖度基本足够，因为 StatusBar 现在的职责已经简化。

### 5.3 `extractErrorMessage` 测试

**文件**: `src/lib/error.ts`

该函数**没有任何测试**。虽然逻辑简单，但它是所有错误显示的入口，建议至少测试：
- `Error` 实例
- 字符串错误
- `MhostError::Io` 格式
- `MhostError::Parse` 格式
- 未知对象 fallback

---

## 修改建议汇总

| 优先级 | 文件 | 建议 |
|--------|------|------|
| 高 | `src/stores/profiles/actions.ts` | 修复 optimistic update 回滚：保存完整 profiles 快照，失败时恢复所有 Profile 状态 |
| 高 | `src-tauri/src/commands/apply.rs` | 为 `enable_and_apply` 添加单元/集成测试 |
| 高 | `src/lib/error.ts` | 增加对 `obj.Io` 嵌套结构的处理；添加单元测试 |
| 中 | `src-tauri/src/commands/apply.rs` + `profile.rs` | 提取共享的"启用时禁用其他 Profile"逻辑 |
| 中 | `src/stores/profiles/state.ts` | 清理不再使用的 `applyPlanAtom` 及相关引用 |
| 低 | `src-tauri/src/commands/apply.rs` | 考虑将 `write_last_applied` 失败提升为可观察的错误 |
| 低 | `src-tauri/crates/mhost-apply/src/platform/macos.rs` | 考虑使用 `cp -p` 保留文件权限 |

---

## 总结

**结论**: **Request Changes**

PR 的核心目标（启用 Profile 时直接写入 hosts、修复授权弹窗）已经正确实现，`atomic_write` 和 `elevated_move` 的修复也是合理的。但存在以下**必须修复**的问题：

1. **Optimistic update 回滚不完整**：启用 Profile A 时禁用了 Profile B，失败后 B 的状态无法恢复
2. **`extractErrorMessage` 未覆盖 `Io` 错误格式**：用户可能看到不友好的 JSON 错误
3. **`enable_and_apply` 零测试**：这是 PR 新增的核心命令，必须有测试覆盖
4. **`applyPlanAtom` 死代码未清理**：增加了维护负担

建议在修复上述问题后重新提交审查。
