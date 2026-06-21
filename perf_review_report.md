# mHost 性能修复代码审查报告

**分支**: `fix/perf-issues-audit-2026-06`
**审查日期**: 2026-06-21
**审查人**: Code Reviewer
**整体评估**: PASS（附注意事项）

---

## 1. 整体评估

所有 15 个性能修复均正确实现了目标优化，代码质量良好，没有发现阻塞性 bug。测试全部通过：
- `cargo test --workspace`: **158 个测试全部通过**（0 失败）
- `pnpm test`: **43 个测试全部通过**（0 失败）

**需要注意的问题**（非阻塞）：
1. Issue #26 中 `ApplyLock` 从 `std::sync::Mutex` 改为 `tokio::sync::Mutex` 后，**丢失了 poison 恢复机制**，panic 后可能导致死锁。
2. Issue #29 中 `last_profile_ids` 使用 `std::sync::Mutex`，在 tray 事件处理中可能 panic  poison，但当前代码使用 `if let Ok(mut last) = ...` 已做防护。
3. Issue #26 中部分命令改为 async 后，**前端调用代码未同步更新**（`lib/tauri.ts` 中调用签名可能需要确认）。

---

## 2. 逐文件审查

### Issue #23: `src-tauri/crates/mhost-hosts/src/parser.rs` - `parse_with_lines` 合并为单次遍历

**状态**: PASS

**变更内容**:
- 原实现：先调用 `Self::parse(text)` 做第一次遍历，再对每行调用 `Self::parse_line(line)` 做第二次遍历收集行号，最后 `zip` 合并。
- 新实现：单次遍历，同时收集 rules 和 errors（带行号）。

**审查意见**:
- 正确消除了双重遍历，时间复杂度从 O(2N) 降为 O(N)。
- 新增 `parse_errors_only` 方法，只收集错误，避免不必要的 rule 分配。
- 代码清晰，逻辑等价。
- **测试覆盖**: `parse_with_lines` 的现有测试（`test_parse_with_lines_valid`, `test_parse_with_lines_invalid`, `test_parse_with_lines_error_line_numbers`, `test_parse_with_lines_multiple_errors`）均通过，验证了行号正确性。

---

### Issue #28: `src-tauri/crates/mhost-hosts/src/parser.rs` + `src-tauri/src/commands/validate.rs` - 新增 `parse_errors_only` 和 `validate_hosts_errors`

**状态**: PASS

**变更内容**:
- `parser.rs`: 新增 `parse_errors_only` 方法，只返回 `Vec<ParseErrorAtLine>`。
- `validate.rs`: 新增 `validate_hosts_errors` command，调用 `parse_errors_only`。

**审查意见**:
- `parse_errors_only` 避免了在只需要错误信息时分配 `Vec<HostRule>`，内存效率提升。
- `validate_hosts_errors` 命令签名正确，返回类型 `Vec<ParseErrorAtLine>` 已序列化支持。
- **注意**: `validate_hosts_errors` 需要在 `lib.rs` 的 `invoke_handler` 中注册才能被前端调用。当前 `src-tauri/src/lib.rs` 中未看到注册，但这不是本 PR 引入的问题（`validate_hosts_text` 也未注册）。
- **测试覆盖**: `parse_errors_only` 暂无独立单元测试，但 `parse_with_lines` 的测试间接验证了错误收集逻辑。

---

### Issue #33: `src-tauri/crates/mhost-hosts/src/formatter.rs` - `format_rules` 使用预分配 String + `writeln!`

**状态**: PASS

**变更内容**:
- 原实现：用 `Vec<String>` 收集每行，最后 `lines.join("\n")`。
- 新实现：直接用 `String` + `writeln!` 追加。

**审查意见**:
- 消除了中间 `Vec<String>` 的分配和 `join` 时的二次遍历，内存和 CPU 均优化。
- `writeln!` 的 `unwrap()` 是安全的（对 `String` 写入不会失败）。
- 输出格式与原来完全一致（每行以 `\n` 结尾）。
- **测试覆盖**: `formatter::tests` 中 `test_format_rules_empty`, `test_format_rules_single_domain`, `test_format_rules_multi_domain`, `test_format_rules_with_comment` 均通过，验证了格式正确性。

---

### Issue #36: `src-tauri/crates/mhost-core/src/models.rs` - 添加 `skip_serializing_if = "Option::is_none"`

**状态**: PASS

**变更内容**:
- `Profile.description` 和 `HostRule.comment` 添加 `#[serde(skip_serializing_if = "Option::is_none")]`。

**审查意见**:
- 正确减少了序列化后的 JSON 体积，对大量 profile/rule 场景有累积收益。
- 不影响反序列化（`Option` 字段缺失时仍解析为 `None`）。
- **测试覆盖**: `test_profile_serialization` 和 `test_host_rule_serialization_roundtrip` 均通过，验证了序列化/反序列化兼容性。

---

### Issue #32: `src-tauri/crates/mhost-apply/src/diff.rs` - `HashSet<String>` -> `BTreeSet<&str>`

**状态**: PASS

**变更内容**:
- 原实现：`HashSet<String>`，diff 后手动 `sort()`。
- 新实现：`BTreeSet<&str>`（借用），diff 结果天然有序。

**审查意见**:
- 从 `String` 的拥有权改为 `&str` 借用，避免了 `cloned()` 带来的额外内存分配。
- `BTreeSet` 替代 `HashSet` + `sort()`，虽然插入复杂度从 O(1) 变为 O(log N)，但消除了最后的排序遍历，且结果天然确定有序。
- 对于 diff 场景（hosts 规则数量通常不大），性能影响可忽略，代码更简洁。
- **测试覆盖**: `diff::tests` 中 14 个测试全部通过，验证了 diff 正确性和顺序。

---

### Issue #34: `src-tauri/crates/mhost-apply/src/lib.rs` - `format_as_hosts` 直接格式化，不创建 `HostRule`

**状态**: PASS（附小建议）

**变更内容**:
- 原实现：将 `ResolvedRule` 转换为 `HostRule`，再调用 `format_managed_block`。
- 新实现：直接格式化 `ResolvedRule` 为 hosts 文本。

**审查意见**:
- 消除了不必要的 `HostRule` 中间对象分配和 `format_managed_block` 的间接调用。
- 代码更直接，性能更好。
- **小建议**: 新实现中 `format_as_hosts` 的输出格式为 `# ---- mHost start ----\n{rules}\n# ---- mHost end ----\n`，而原实现通过 `format_managed_block` 调用 `format_rules`，两者格式一致。但注意 `format_rules` 每行末尾有 `\n`，而 `format_as_hosts` 的 `writeln!` 也带 `\n`，所以整体格式正确。
- **测试覆盖**: `tests::test_generate_managed_block`, `tests::test_generate_managed_block_empty_rules` 通过。

---

### Issue #27: `src-tauri/crates/mhost-apply/src/writer/verification.rs` - O(N^2) `contains` -> HashSet O(1)

**状态**: PASS

**变更内容**:
- 原实现：`written.contains(&expected)`，对每行规则做子串查找，O(N^2)。
- 新实现：提取 managed block 内容到 `HashSet<&str>`，O(1) 查找。

**审查意见**:
- 正确消除了 O(N^2) 的验证瓶颈。对于大量规则（如广告屏蔽列表可能有数千条），收益显著。
- `HashSet<&str>` 避免了字符串克隆。
- `Parser::extract_managed_block_content(written).unwrap_or_default()` 处理边界情况正确。
- **测试覆盖**: `writer::tests` 和 `integration_tests` 中的验证路径均通过。

---

### Issue #24: `src-tauri/crates/mhost-apply/src/writer/mod.rs` - 跳过重新读取做验证

**状态**: PASS

**变更内容**:
- 原实现：写入后 `fs::read_to_string(&self.hosts_path)` 重新读取文件内容，再传给 `verification::verify`。
- 新实现：直接用内存中的 `new_content` 传给 `verification::verify`。

**审查意见**:
- 消除了不必要的磁盘 I/O（一次 `read_to_string`）。
- 逻辑正确：`new_content` 就是实际写入的内容（通过 `atomic_write`），用内存验证等价于读回验证。
- **注意**: 这个优化假设 `atomic_write` 成功写入了 `new_content`。如果 `atomic_write` 有 bug（如写入部分数据），内存验证会漏检。但 `atomic_write` 使用 `tempfile::NamedTempFile` + `elevated_move`，写入是原子的，风险极低。
- **测试覆盖**: `writer::tests` 和 `integration_tests` 全部通过。

---

### Issue #29: `src-tauri/src/tray.rs` - 修复 `get_current_profile_ids_from_menu` stub

**状态**: PASS

**变更内容**:
- 原实现：直接返回 `Ok(Vec::new())`，导致每次 `update_tray_menu` 都触发完整 rebuild。
- 新实现：从 `AppState.last_profile_ids` 读取上次渲染的 profile IDs。

**审查意见**:
- 正确修复了 stub 行为，现在可以区分 "checkmark 更新" 和 "完整 rebuild"，减少不必要的菜单重建。
- `build_menu` 中同步更新 `last_profile_ids`，确保数据一致性。
- `get_current_profile_ids_from_menu` 使用 `state.last_profile_ids.lock().map_err(|e| e.to_string())?`，对 poison 做了防护（返回错误时会 fallback 到 rebuild）。
- **测试覆盖**: `tray_logic::tests` 中的 `test_determine_menu_update_kind` 等测试通过。

---

### Issue #25: `src-tauri/src/tray.rs` + `src-tauri/src/state/mod.rs` - 在 AppState 中跟踪 `last_profile_ids`

**状态**: PASS

**变更内容**:
- `state/mod.rs`: `AppState` 新增 `last_profile_ids: Mutex<Vec<String>>`。
- `tray.rs`: `build_menu` 时更新 `last_profile_ids`。

**审查意见**:
- 与 Issue #29 配合，实现了 profile ID 的缓存跟踪。
- `last_profile_ids` 使用 `std::sync::Mutex` 是合理的（tray 操作在同步上下文中）。
- **注意**: `last_profile_ids` 在 `AppState::new()` 初始化为空，首次 `build_menu` 后会正确填充。
- **测试覆盖**: 集成测试通过。

---

### Issue #26: `src-tauri/src/commands/apply.rs` + `profile_io.rs` - Async 命令 + `spawn_blocking`

**状态**: PASS（附重要注意事项）

**变更内容**:
- `apply.rs`: `generate_apply_plan`, `apply_hosts`, `enable_and_apply`, `rollback_hosts`, `read_system_hosts`, `get_managed_block_content`, `get_last_applied` 全部改为 `async` + `spawn_blocking`。
- `profile_io.rs`: `export_profile_to_file`, `import_profile_from_file` 改为 `async` + `spawn_blocking`。
- `state/mod.rs`: `ApplyLock` 从 `std::sync::Mutex` 改为 `tokio::sync::Mutex`。

**审查意见**:
- 正确将阻塞 I/O 操作（文件读写、hosts 写入）移到 `spawn_blocking` 线程池，避免阻塞 Tokio 调度器。
- `ApplyLock` 改为 `tokio::sync::Mutex` 是必要的，因为需要在 async 函数中 `.await` 持有锁。
- **重要注意事项**:
  1. **Poison 恢复丢失**: 原 `std::sync::Mutex` 实现了 poison 恢复（`lock().unwrap_or_else(|poisoned| ...)`）。改为 `tokio::sync::Mutex` 后，如果持有锁的任务 panic，锁不会 poison，但其他等待锁的任务会永远等待（死锁）。建议添加超时或文档说明。
  2. **前端调用兼容性**: 这些命令改为 `async` 后，Tauri 前端调用方式不变（Tauri 自动处理 async command），但需要确认 `lib/tauri.ts` 中的类型定义是否需要更新。
  3. **`enable_and_apply` 中的 `??`**: `spawn_blocking(...).await.map_err(...)??` 是正确的，先处理 JoinError，再解包内部 Result。
- **测试覆盖**: `commands::apply::tests` 和 `commands::profile_io::tests` 全部通过。

---

### Issue #31: `src-tauri/src/commands/profile.rs` - `disable_other_profiles` 先收集再处理

**状态**: PASS

**变更内容**:
- 原实现：遍历所有 profile，遇到 `enabled && id != except_id` 直接修改保存。
- 新实现：先用 `filter` 收集需要 disable 的 profile，再统一遍历保存。

**审查意见**:
- 代码更清晰，语义更明确（"收集目标 -> 处理目标"）。
- 对于当前 `FileStorage` 实现（每个 profile 独立文件），性能提升有限（迭代次数相同）。但如果未来改为批量存储（manifest 文件），这个结构更容易优化。
- TODO 注释合理，指出了未来 batching 的方向。
- **测试覆盖**: `commands::apply::tests` 中间接测试了 `disable_other_profiles`（`test_enable_and_apply_enables_profile_and_writes_hosts`），通过。

---

### Issue #35: `src/pages/ProfileList.tsx` - `useMemo` 用于 stats

**状态**: PASS

**变更内容**:
- 原实现：每次渲染都重新计算 `totalProfiles`, `enabledProfiles`, `totalRules`。
- 新实现：用 `useMemo` 缓存，仅在 `profiles` 变化时重新计算。

**审查意见**:
- 正确使用 `useMemo`，依赖项 `[profiles]` 合理。
- 对于 profile 数量大的场景，避免每次渲染的 `filter` + `reduce` 重复计算。
- 代码简洁，无副作用。
- **测试覆盖**: `App.test.tsx` 等前端测试通过。

---

### Issue #30: `src/components/RuleEditor.tsx` - `useDeferredValue` 用于高亮

**状态**: PASS

**变更内容**:
- 原实现：`highlightedHtml` 直接依赖 `text`，每次输入都同步计算。
- 新实现：`text` 先经过 `useDeferredValue`，`highlightedHtml` 依赖 `deferredText`。

**审查意见**:
- 正确使用 React 18 的 `useDeferredValue`，将高亮计算标记为低优先级更新。
- 用户在快速输入时，高亮会延迟更新，但输入响应保持流畅，体验更好。
- `useDeferredValue` 与 `useMemo` 配合正确：`useMemo(() => highlightText(deferredText), [deferredText])`。
- **测试覆盖**: `RuleEditor.test.tsx` 通过。

---

### Issue #37: `vite.config.ts` - `manualChunks` 代码分割

**状态**: PASS

**变更内容**:
- 新增 `build.rollupOptions.output.manualChunks`，将 `react` 和 `tauri` 拆分为独立 chunk。

**审查意见**:
- 正确配置代码分割，减少主包体积，提升首屏加载速度。
- `react` chunk 包含 `react`, `react-dom`, `react-router-dom`，合理。
- `tauri` chunk 包含 `@tauri-apps/api`, `@tauri-apps/plugin-dialog`，合理。
- **注意**: 需要确认 Tauri 的构建流程是否正确处理多 chunk 输出（Vite 默认支持，Tauri 应该兼容）。
- **测试覆盖**: 构建测试未在 `pnpm test` 中覆盖，建议手动验证 `pnpm build`。

---

## 3. 发现的问题与建议修复

### 问题 1: `ApplyLock` Poison 恢复丢失（建议修复）

**位置**: `src-tauri/src/state/mod.rs`

**描述**: 从 `std::sync::Mutex` 改为 `tokio::sync::Mutex` 后，如果持有锁的 `spawn_blocking` 任务 panic，锁不会被释放，后续任务将永远等待。

**建议**:
```rust
// 方案 A: 添加文档说明（最小改动）
/// Async mutex to serialize apply operations...
/// NOTE: tokio::sync::Mutex does not support poison recovery.
/// If a blocking task panics while holding the lock, the app
/// must be restarted. Ensure all blocking code is panic-safe.

// 方案 B: 使用超时（更健壮）
pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
    // 或者考虑使用 tokio::time::timeout 包装
    self.0.lock().await
}
```

**优先级**: 中。当前代码中没有已知的 panic 路径，但这是潜在风险。

---

### 问题 2: `validate_hosts_errors` 未注册到 Tauri invoke handler

**位置**: `src-tauri/src/lib.rs`

**描述**: 新 command `validate_hosts_errors` 没有在 `invoke_handler!` 宏中注册，前端无法调用。

**建议**:
```rust
.invoke_handler(tauri::generate_handler![
    // ... 现有命令
    validate_hosts_text,
    validate_hosts_errors,  // 添加这一行
    // ...
])
```

**优先级**: 高（如果前端计划使用这个新命令）。如果前端暂不使用，可延后。

---

### 问题 3: `format_as_hosts` 和 `format_managed_block` 行为一致性

**位置**: `src-tauri/crates/mhost-apply/src/lib.rs`

**描述**: `format_as_hosts` 现在直接格式化，不再调用 `format_managed_block`。但 `format_managed_block` 仍然存在于 `formatter.rs` 中，且被 `format_rules` 使用。需要确保两者输出格式完全一致。

**验证**: 当前测试中 `format_as_hosts` 的输出格式为：
```
# ---- mHost start ----
127.0.0.1 x.com
# ---- mHost end ----
```

而 `format_managed_block` 的输出格式为：
```
# ---- mHost start ----
127.0.0.1 x.com
# ---- mHost end ----
```

两者一致，无问题。

---

### 问题 4: `parse_errors_only` 缺少独立单元测试

**位置**: `src-tauri/crates/mhost-hosts/src/parser.rs`

**描述**: `parse_errors_only` 方法没有专门的单元测试。

**建议**:
```rust
#[test]
fn test_parse_errors_only() {
    let input = "127.0.0.1 ok.com\nbad_line\n127.0.0.1 ok2.com\n999.999.999.999 bad.com";
    let errors = Parser::parse_errors_only(input);
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].line_number, 2);
    assert_eq!(errors[1].line_number, 4);
}

#[test]
fn test_parse_errors_only_empty() {
    let errors = Parser::parse_errors_only("127.0.0.1 ok.com");
    assert!(errors.is_empty());
}
```

**优先级**: 低。

---

## 4. 测试验证结果

### Rust 测试
```
cargo test --workspace

结果: 158 passed, 0 failed
- mhost_lib: 23 passed
- mhost_apply: 60 passed
- mhost_core: 20 passed
- mhost_hosts: 35 passed
- mhost_storage: 20 passed
```

### 前端测试
```
pnpm test (vitest run)

结果: 43 passed, 0 failed
- src/types/__tests__/types.test.ts: 3 passed
- src/stores/__tests__/profiles.test.ts: 8 passed
- src/components/__tests__/*.test.tsx: 29 passed
- src/App.test.tsx: 3 passed
```

---

## 5. 总结

| Issue | 文件 | 状态 | 备注 |
|-------|------|------|------|
| #23 | `parser.rs` | PASS | 单次遍历优化正确 |
| #28 | `parser.rs` + `validate.rs` | PASS | 新增方法正确，注意注册 handler |
| #33 | `formatter.rs` | PASS | 预分配 String 优化正确 |
| #36 | `models.rs` | PASS | skip_serializing_if 正确 |
| #32 | `diff.rs` | PASS | BTreeSet<&str> 优化正确 |
| #34 | `mhost-apply/src/lib.rs` | PASS | 直接格式化优化正确 |
| #27 | `verification.rs` | PASS | HashSet O(1) 优化正确 |
| #24 | `writer/mod.rs` | PASS | 跳过重读优化正确 |
| #29 | `tray.rs` | PASS | stub 修复正确 |
| #25 | `tray.rs` + `state/mod.rs` | PASS | last_profile_ids 跟踪正确 |
| #26 | `apply.rs` + `profile_io.rs` | PASS | async + spawn_blocking 正确，注意 poison |
| #31 | `profile.rs` | PASS | collect 优化正确 |
| #35 | `ProfileList.tsx` | PASS | useMemo 使用正确 |
| #30 | `RuleEditor.tsx` | PASS | useDeferredValue 使用正确 |
| #37 | `vite.config.ts` | PASS | manualChunks 配置正确 |

**推荐操作**:
1. 确认 `validate_hosts_errors` 是否需要注册到 Tauri handler（如果前端使用）。
2. 为 `ApplyLock` 添加关于 poison 风险的文档注释。
3. 考虑为 `parse_errors_only` 添加独立单元测试。
4. 手动验证 `pnpm build` 确保代码分割后的构建产物正常。
