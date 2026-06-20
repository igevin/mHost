# 阶段 1：MVP，可用的 Profile 切换 — 开发计划

创建日期：2026-06-20
更新日期：2026-06-20（v2：移除签名/Privileged Helper，v3：二次 Review 修订）

---

## 1. 阶段目标

在 Phase 0 产品骨架之上，让开发者能用 mHost **管理多环境 hosts 并完成可靠切换**。Phase 0 是"能跑"，Phase 1 是"能用"。

---

## 2. 当前代码现状分析

### 2.1 Phase 0 已有的能力

| 模块 | 已有 | 状态 |
|------|------|------|
| Profile CRUD（创建/读取/更新/删除） | Rust commands + 前端页面 | 完成 |
| 单 Profile 启用/禁用（互斥） | `set_profile_enabled` + 乐观更新 | 完成 |
| hosts 解析/格式化/校验 | `mhost-hosts` crate + 122 测试 | 完成 |
| 规则合并/冲突检测/diff | `mhost-apply` crate | 完成 |
| 系统 hosts 写入（备份/回滚） | `mhost-apply::writer` | 完成 |
| 前端应用壳（3 页面 + 侧边栏） | React + Jotai | 完成 |
| 托管区块（`# ---- mHost start/end ----`） | 后端 | 完成 |
| macOS 权限写入（osascript） | 后端 | 完成 |
| 读取系统 hosts | `read_system_hosts` command + 前端 `readSystemHosts()` | 完成 |
| Apply Plan 生成 | `generate_apply_plan` command + 前端 store | 完成 |
| 应用状态管理 | `applyPlanAtom`、`isApplyingAtom` 等 Jotai atom | 完成 |

### 2.2 Phase 1 需要新增的能力

| 能力 | 描述 |
|------|------|
| **hosts 文本编辑器** | 当前 ProfileEdit 只展示规则为只读，Phase 1 需要可编辑的文本域 |
| **基础语法检查（UI 层）** | 编辑时实时校验，错误规则不允许应用到系统 |
| **当前生效状态展示** | 展示当前 `/etc/hosts` 中 mHost 托管区块的实际内容 |
| **Profile 导入/导出** | 从外部 hosts 文件导入规则；导出 Profile 为 hosts 文件 |
| **Profile 复制** | 基于已有 Profile 创建副本 |
| **应用确认流程** | Apply Plan 预览 + 用户确认 + 结果反馈 |
| **回滚 UI** | 前端暴露回滚操作入口 |
| **Windows 基础适配** | 平台层（hosts 路径、权限提升、DNS 刷新） |

### 2.3 关于"代码文件代理量超过 1 万行"的分析

经排查，**当前所有源文件均未超过 1000 行**。最大源文件为 `writer.rs`（844 行）和 `App.css`（679 行）。

但以下文件存在**未来膨胀风险**，建议在 Phase 1 中预拆分：

| 文件 | 当前行数 | 风险 | 拆分方案 |
|------|--------|------|------|
| `src/App.css` | 679 | 随组件增多会长到 2000+ | CSS Modules（通用样式保留 `global.css`，组件样式用 `*.module.css`） |
| `src/stores/profiles.ts` | 205 | 随 action 增多会膨胀 | 拆为 `state.ts`（atoms + derived）+ `actions.ts` |
| `src-tauri/crates/mhost-apply/src/writer.rs` | 844 | 测试代码占一半，后续加 Windows 适配会超 1000 | 拆为 `backup.rs`、`content.rs`、`verification.rs`；`OsascriptMover` 移入 `platform/macos.rs` |
| `src/pages/ProfileList.tsx` | 210 | 会随功能增多膨胀 | 提取 `ProfileCard`、`CreateProfileForm` 为独立组件 |
| `src/pages/ProfileEdit.tsx` | 198 | 加入文本编辑器后会膨胀 | 提取 `RuleEditor`、`BasicInfoForm` 为独立组件 |

---

## 3. 技术决策

沿用 Phase 0 的所有决策，Phase 1 新增决策：

| 决策项 | 结论 |
|--------|------|
| 规则编辑器形态 | 先做**文本编辑器**（textarea），表格编辑留到 Phase 2 |
| 导入格式 | 支持标准 hosts 文本格式（与 `/etc/hosts` 兼容），方式：粘贴文本 + 选择文件 |
| 导出格式 | 标准 hosts 文本文件、JSON 两种格式 |
| 导入名称冲突 | 同名 Profile 追加数字后缀（如 "Production (2)"），不覆盖已有 Profile |
| 导入策略 | 仅支持新建 Profile，不支持增量合并到已有 Profile |
| 导出 JSON schema | 与 Profile 存储格式一致（可直接用于导入） |
| Windows 权限 | 使用 `runas` / PowerShell `Start-Process -Verb RunAs`（与 macOS osascript 对等） |
| CSS 拆分策略 | 混合策略：通用样式（`.btn`、`.card`、`.input` 等）保留 `global.css`，组件特定样式用 CSS Modules |
| 测试策略 | 严格 TDD：Red → Green → Refactor |
| 校验接口 | 复用已有 `mhost-hosts::Parser::parse()`，在其上薄封装 `validate_hosts_text`，不创建并行数据结构 |
| 错误处理 | Phase 1 各组件使用局部错误状态，不依赖全局 `errorAtom`；错误处理架构重构留到 Phase 2 |

---

## 4. TDD 执行原则（继承 Phase 0）

- **Red**：先写测试，确认失败
- **Green**：实现最小代码，让测试通过
- **Refactor**：重构，保持测试通过
- 所有 Rust 单元测试采用表格驱动
- 前端测试：组件测试用 Vitest + React Testing Library，store 测试用纯逻辑测试
- 测试断言必须验证具体内容，不能仅验证数量（如 `rules.len()`）

---

## 5. 任务拆分

### 总览

```
                     ┌──────────────────────────────┐
                     │  T0：工程结构与代码拆分（重构） │
                     └──────────────┬───────────────┘
                                    │
          ┌─────────────────────────┼─────────────────────────┐
          │                         │                         │
          ▼                         ▼                         ▼
┌─────────────────┐    ┌─────────────────────┐    ┌──────────────────┐
│ T1：hosts 文本   │    │ T2：Profile 导入导出  │    │ T4：Windows       │
│ 编辑器 + 语法检查 │    │ 与复制               │    │ 基础适配          │
└────────┬────────┘    └──────────┬──────────┘    └────────┬─────────┘
         │                        │                        │
         └──────────┬─────────────┘                        │
                    │                                      │
         ┌──────────┼──────────┐                           │
         │          │          │                           │
         ▼          ▼          ▼                           │
┌────────────┐ ┌──────────────────┐                        │
│ T3：当前    │ │ T5：应用确认流程  │                        │
│ 生效状态展示│ │ + 回滚 UI         │                        │
└─────┬──────┘ └────────┬─────────┘                        │
      │                 │                                  │
      └────────┬────────┘                                  │
               │                                           │
               └───────────────┬───────────────────────────┘
                               │
                               ▼
                    ┌─────────────────────┐
                    │ T7：集成验收          │
                    └─────────────────────┘
```

---

### T0：工程结构与代码拆分（重构）

**类型**：重构（非 TDD，但需保证已有测试通过）

**依赖**：无

**目标**：在开始新功能开发前，将潜在膨胀的文件合理拆分，降低后续开发复杂度。

**执行策略**：每个子任务单独提交，便于独立回滚。

**交付物**：拆分后的工程结构，每个子任务一个 commit，`cargo test` + `pnpm test` + `pnpm build` 全部通过。

---

#### T0.1 CSS 拆分（`src/App.css` → CSS Modules + global.css）

**分层策略**：混合策略，不是全部迁移到 CSS Modules。

- **`src/styles/global.css`**：CSS 变量、reset、通用组件样式（`.btn`、`.card`、`.input`、`.form-group`、`.alert`、`.badge`、`.toggle` 等）
- **`src/components/*.module.css`**：组件特定样式（如 `.profile-card`、`.rule-item` 等）
- **`src/pages/*.module.css`**：页面布局样式

**关键注意**：迁移过程中，`.tsx` 文件中的 `className` 需要从全局字符串改为 `styles.xxx` 引用。逐组件迁移，每个组件迁移后 visually 验证。

**验收**：
- `pnpm build` 通过
- 逐个组件 visually 验证无样式回归（屏幕截图对比）
- 无未使用的 CSS 变量/类名残留

---

#### T0.2 Store 拆分（`src/stores/profiles.ts`）

当前只有 2 个派生 atom（`selectedProfileAtom`、`enabledProfileAtom`），每个仅 3-5 行，不需要独立文件。

```
src/stores/
  profiles/
    state.ts        # 基础 atom + 派生 atom（放在一起，派生紧跟源 atom）
    actions.ts      # 异步 action atom：fetchProfilesAtom, createProfileAtom, etc.
                    # 新增 rollbackHostsActionAtom（T5 需要，当前 stores 中缺失）
    index.ts        # 统一 re-export
```

**验收**：`pnpm vitest run` 通过，无类型错误。

---

#### T0.3 Rust writer 拆分（`mhost-apply/src/writer.rs`）

**重要**：`OsascriptMover` 和 `escape_applescript_path` 是 macOS 特定实现，应放入 `mhost-apply/src/platform/macos.rs`。注意区分两个 `platform/` 目录：
- `mhost-apply/src/platform/`（本次拆分目标）：平台抽象层，`PlatformAdapter` trait + macOS/Windows 实现
- `src-tauri/src/platform/macos.rs`（现有，仅 1 行注释）：T4 完成后将改为引用 `mhost-apply` 的 platform 层

```
mhost-apply/src/
  writer/
    mod.rs           # HostsWriter 结构体 + apply/rollback 入口
    backup.rs        # create_backup, prune_old_backups, rollback
    content.rs       # build_hosts_content
    verification.rs  # verify
  platform/
    mod.rs           # 平台抽象（T4 在此定义 PlatformAdapter trait）
    macos.rs         # OsascriptMover, escape_applescript_path（从 writer.rs 移入）
    windows.rs       # Windows 占位（T4 实现）
```

**验收**：`cargo test -p mhost-apply` 全部通过。

---

#### T0.4 前端组件拆分

```
src/components/
  ProfileCard.tsx          # 从 ProfileList 提取
  CreateProfileForm.tsx    # 从 ProfileList 提取
  BasicInfoForm.tsx        # 从 ProfileEdit 提取（name/description/tags）
  RuleList.tsx             # 从 ProfileEdit 提取（规则只读展示）
  RuleEditor.tsx           # 新增（文本编辑器，T1 实现）
  StatusBar.tsx            # 从 Layout 提取（侧边栏底部状态）
```

**验收**：`pnpm vitest run` 通过，`pnpm build` 通过。

---

#### T0.5 前置依赖验证

在开始 T1-T5 开发前，验证关键依赖：

1. 验证 `@tauri-apps/plugin-dialog` 与当前 Tauri v2 版本的兼容性（`pnpm add @tauri-apps/plugin-dialog`，确认 API 能力）
2. 确认 Rust 端 `tauri::api::dialog` 或 `tauri-plugin-dialog` 的可用性

**验收**：依赖安装成功，无版本冲突，基本 API 调用通过。

---

**预估**：1.5 天

---

### T1：hosts 文本编辑器 + 实时语法检查

**类型**：TDD（前端组件 + Rust 后端）

**依赖**：T0

**交付物**：`RuleEditor.tsx` 组件 + `validate_hosts_text` command + 测试文件

**内容**：

#### T1.1 Rust 后端：实时校验接口

**复用已有能力**：`mhost-hosts/src/parser.rs` 已有 `Parser::parse()` 返回 `ParseResult`（含 `Vec<HostRule>` 和 `Vec<ParseError>`）。Phase 1 只需在其上薄封装，让 `ParseError` 携带 `line_number`。

**方案**：在 `mhost-core::error` 中扩展 `ParseError` 增加 `line_number: Option<usize>` 字段，同步修改 `mhost-hosts::parser` 中所有 `ParseError` 构造调用以传入行号。然后新增 Tauri command：

```rust
// src-tauri/src/commands/validate.rs（新增文件）
#[tauri::command]
pub fn validate_hosts_text(text: String) -> ParseResult {
    // 直接调用 mhost_hosts::parser::Parser::parse(&text)
    // 返回已有的 ParseResult 结构
}
```

**不清除**：不创建新的 `ValidationResult`/`ValidatedRule`/`ParseErrorWithLine` 数据结构，直接复用 `ParseResult`/`HostRule`/`ParseError`。

**TDD 测试用例（表格驱动，验证具体内容）**：

```rust
#[test]
fn test_validate_valid_hosts() {
    let cases = vec![
        ("single_ipv4", "127.0.0.1 example.com",
         vec![("127.0.0.1", vec!["example.com"])], vec![]),
        ("ipv6", "::1 localhost",
         vec![("::1", vec!["localhost"])], vec![]),
        ("multi_domain", "127.0.0.1 a.com b.com",
         vec![("127.0.0.1", vec!["a.com", "b.com"])], vec![]),
        ("with_comment", "127.0.0.1 x.com # dev",
         vec![("127.0.0.1", vec!["x.com"])], vec![]),
        ("empty_lines", "\n\n127.0.0.1 x.com\n\n",
         vec![("127.0.0.1", vec!["x.com"])], vec![]),
        ("comment_only", "# this is a comment", vec![], vec![]),
    ];
    for (name, input, expected_rules, expected_errors) in cases {
        let result = Parser::parse(input);
        assert_eq!(result.rules.len(), expected_rules.len(), "case: {}", name);
        for (i, (expected_ip, expected_domains)) in expected_rules.iter().enumerate() {
            assert_eq!(result.rules[i].ip, *expected_ip, "case: {} rule {} ip", name, i);
            assert_eq!(result.rules[i].domains, *expected_domains, "case: {} rule {} domains", name, i);
        }
        assert_eq!(result.errors.len(), expected_errors.len(), "case: {}", name);
    }
}

#[test]
fn test_validate_invalid_hosts() {
    let cases = vec![
        ("bad_ip", "999.999.999.999 x.com", "invalid IP address"),
        ("bad_domain", "127.0.0.1 -bad", "invalid domain"),
        ("no_ip", "example.com 127.0.0.1", "invalid format"),
    ];
    for (name, input, expected_msg_contains) in cases {
        let result = Parser::parse(input);
        assert!(!result.errors.is_empty(), "case: {} should have errors", name);
        let msg = result.errors[0].message.to_lowercase();
        assert!(msg.contains(expected_msg_contains),
            "case: {} error '{}' should contain '{}'", name, msg, expected_msg_contains);
    }
}

#[test]
fn test_parse_error_includes_line_number() {
    let result = Parser::parse("127.0.0.1 valid.com\n999.999.999.999 bad.com");
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].line_number, Some(2));
}

#[test]
fn test_validate_hosts_text_command_roundtrip() {
    // 集成测试：直接调用 validate_hosts_text 函数
    // 验证输入 hosts 文本 → 解析结果可序列化/反序列化（MhostError 通过 Tauri 返回）
    let result = validate_hosts_text("127.0.0.1 example.com".to_string());
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ParseResult = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.rules.len(), result.rules.len());
}
```

#### T1.2 前端：RuleEditor 文本编辑器组件

**组件规格**：

```typescript
// src/components/RuleEditor.tsx
interface RuleEditorProps {
  rules: HostRule[];            // 当前 Profile 的规则
  onChange: (rules: HostRule[]) => void;  // 规则变更回调（仅校验通过时调用）
  readOnly?: boolean;
}
```

**功能**：
- 将 `HostRule[]` 渲染为 hosts 文本格式的 textarea
- 用户编辑文本后，debounce 300ms，然后调用 `validate_hosts_text` 进行语法检查
- 错误行高亮显示（红色背景或边框）
- 错误信息在行尾或悬浮 tooltip 展示
- 只有验证通过（零错误）的规则才调用 `onChange`

**TDD 测试用例**：

```typescript
describe('RuleEditor', () => {
  it('renders rules as hosts text', () => {
    // 渲染规则 → 验证 textarea 内容正确
  });

  it('shows validation errors inline with line highlight', () => {
    // 输入无效 IP → 验证错误行高亮显示 + 错误信息可见
  });

  it('emits parsed rules on valid change', () => {
    // 编辑文本 → 等待 debounce → 验证 onChange 回调收到正确规则
  });

  it('does not emit onChange on invalid input', () => {
    // 输入无效内容 → 验证 onChange 未被调用
    // 但错误信息应该显示
  });

  it('handles empty input', () => {
    // 清空 textarea → 验证 onChange 收到空规则数组
  });

  it('debounces validation calls on rapid input', () => {
    // 快速连续输入 5 个字符 → 验证 invoke 只被调用 1 次
  });

  it('handles large input (1000+ lines) without blocking UI', () => {
    // 生成 1000 行规则 → 验证渲染和校验不阻塞 UI
  });
});
```

#### T1.3 前端：ProfileEdit 集成 RuleEditor

- 替换当前 ProfileEdit 中 Rules 只读展示区域为 RuleEditor
- 保存时校验规则无错误
- 有错误时阻止保存并提示用户（局部错误提示，不使用全局 `errorAtom`）

**预估**：3 天

---

### T2：Profile 导入导出与复制

**类型**：TDD（Rust 后端 + 前端）

**依赖**：T0

**交付物**：`import_profile`、`export_profile`、`duplicate_profile` 三个 command + 前端 ImportDialog/ExportButton 组件 + 测试文件

**内容**：

#### T2.1 Rust 后端：导入命令

```rust
#[tauri::command]
pub fn import_profile(
    name: String,
    hosts_text: String,
    state: State<'_, AppState>,
) -> Result<Profile, MhostError> {
    // 1. 复用 mhost_hosts::parser::Parser::parse(&hosts_text) 解析
    // 2. 如果解析有错误，返回错误（不创建 Profile）
    // 3. 检查名称是否冲突，冲突则追加数字后缀（如 "Production (2)"）
    // 4. 创建新 Profile（disabled 状态）
    // 5. 保存
}
```

**注意**：仅支持新建 Profile，不支持增量合并到已有 Profile。

**额外**：同时实现 `read_file_text` command（T2.4 需要），用于读取用户选择的文件内容。

**TDD 测试用例**：

```rust
#[test]
fn test_import_from_hosts_text() {
    let cases = vec![
        ("simple", "127.0.0.1 example.com", 1),
        ("multiple", "127.0.0.1 a.com\n192.168.1.1 b.com", 2),
        ("with_comments", "# header\n127.0.0.1 x.com # inline", 1),
        ("empty", "", 0),
    ];
    // ...
}

#[test]
fn test_import_rejects_invalid() {
    // 包含无效 IP 的 hosts 文本 → 返回错误
}

#[test]
fn test_import_name_conflict() {
    // 导入与已有 Profile 同名的 → 自动追加后缀
}

#[test]
fn test_import_persisted_to_storage() {
    // 导入后 → 调用 list_profiles → 验证新 Profile 存在且字段正确
}
```

#### T2.2 Rust 后端：导出命令

```rust
#[tauri::command]
pub fn export_profile(
    id: String,
    format: ExportFormat,  // "hosts" | "json"
    state: State<'_, AppState>,
) -> Result<String, MhostError> {
    // hosts 格式：复用 mhost_hosts::formatter::format_rules
    // json 格式：serde_json::to_string_pretty
}

/// 写入文件内容（T2.4 导出使用）
#[tauri::command]
pub fn write_file_text(path: String, content: String) -> Result<(), MhostError> {
    // 将内容写入指定路径
}
```

**TDD 测试用例**：

```rust
#[test]
fn test_export_as_hosts() {
    // 规则 → hosts 文本格式，验证内容正确
}

#[test]
fn test_export_as_json() {
    // 规则 → JSON 格式，验证 schema 与 Profile 存储一致
}

#[test]
fn test_export_roundtrip() {
    // 导出 → 导入 → 规则一致（名称、IP、domains、comment）
}
```

#### T2.3 Rust 后端：复制命令

```rust
#[tauri::command]
pub fn duplicate_profile(
    id: String,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<Profile, MhostError> {
    // 加载 Profile → 创建副本（新 UUID、新名称、disabled、新时间戳）→ 保存
}
```

#### T2.4 前端：导入导出 UI

**依赖**：需先验证 `@tauri-apps/plugin-dialog` 与 Tauri v2 的兼容性（T0 前置验证）。

**注意**：`@tauri-apps/plugin-dialog` 只返回文件路径，不读写文件内容。因此需要额外新增两个 Rust command：
- `read_file_text(path: String) -> Result<String, MhostError>`：读取文件内容（用于导入）
- `write_file_text(path: String, content: String) -> Result<(), MhostError>`：写入文件内容（用于导出）

这两个 command 在 T2.1/T2.2 中一并实现和测试。

- ProfileList 页面添加 "Import" 按钮
- 导入弹窗：输入 Profile 名称 + 粘贴 hosts 文本 / 选择文件（通过 Tauri file dialog，读取后调用 `read_file_text`，内容加载到文本框中预览）
- ProfileCard 添加 "Export" 和 "Duplicate" 按钮
- 导出：通过 Tauri save dialog 选择保存路径，调用 `write_file_text` 写入 hosts 或 JSON 文件

**TDD 测试用例**：

```typescript
describe('ImportDialog', () => {
  it('validates pasted hosts text before import');
  it('shows errors for invalid hosts text in the dialog');
  it('creates profile on valid import and closes dialog');
  it('handles name conflict by appending suffix');
});

describe('ExportButton', () => {
  it('exports as hosts format via save dialog');
  it('exports as JSON format via save dialog');
});
```

**预估**：2 天

---

### T3：当前生效状态展示

**类型**：TDD（前端 + 后端）

**依赖**：T0（T1 的 RuleEditor 不是硬依赖，T3 先做只读展示，等 T1 完成后再增强为可编辑）

**交付物**：`ApplyStatus` 组件 + `get_managed_block_content` command + 测试文件

**内容**：

#### T3.1 Rust 后端：增加 `get_managed_block_content` command

**复用已有能力**：Phase 0 已有 `read_system_hosts` command（`src-tauri/src/commands/apply.rs`），`mhost-hosts::parser` 已有 `Parser::extract_managed_block()` 返回托管区块的行号范围。`get_managed_block_content` 内部组合调用两者：读取系统 hosts → 提取行号范围 → 字符串切片返回区块内容。

**`last_applied` 时间戳存储方案**：当前 Profile 模型无此字段。在 `apply_hosts` command 成功后将时间戳写入 `~/.local/share/mHost/last_applied`（JSON 文件），前端通过新增的 `get_last_applied` command 读取。不修改 Profile 模型。

```rust
#[tauri::command]
pub fn get_managed_block_content(
    state: State<'_, AppState>,
) -> Result<Option<String>, MhostError> {
    // 1. 读取系统 hosts 文件
    // 2. 提取 mHost 托管区块（# ---- mHost start ---- 到 # ---- mHost end ----）
    // 3. 返回区块内容，如果不存在则返回 None
}
```

**TDD 测试用例**：

```rust
#[test]
fn test_get_managed_block_content_with_block() {
    // 模拟有托管区块的 hosts 文件 → 验证返回正确内容
}

#[test]
fn test_get_managed_block_content_without_block() {
    // 模拟无托管区块的 hosts 文件 → 验证返回 None
}

#[test]
fn test_get_managed_block_content_empty_block() {
    // 模拟空托管区块 → 验证返回空字符串
}

#[test]
fn test_get_last_applied_timestamp() {
    // 模拟 last_applied 文件存在 → 验证返回正确时间戳
    // 模拟文件不存在 → 验证返回 None
}
```

#### T3.2 前端：ApplyStatus 组件

**组件规格**：

```typescript
// src/components/ApplyStatus.tsx
// 组件内部调用已有的 readSystemHosts() 和新增的 getManagedBlockContent()
// 使用已有 store 中的 applyPlanAtom、enabledProfileAtom 等
```

**功能**：
- 展示 "当前生效的 Profile" 名称和规则列表（使用 `enabledProfileAtom`）
- 展示托管区块在系统 hosts 中的内容（使用 `getManagedBlockContent`）
- 展示 "Last Applied" 时间戳
- 展示未应用的变更提示（dirty diff，对比 `applyPlanAtom` 与当前托管区块）
- 冲突提示（复用 `mhost-apply::conflict` 逻辑）

**TDD 测试用例**：

```typescript
describe('ApplyStatus', () => {
  it('shows active profile name and rules');
  it('shows managed block content from system hosts');
  it('shows "no active profile" when none enabled');
  it('shows conflict warnings when conflicts present');
  it('shows "pending changes" indicator when plan differs from current');
  it('shows "Last Applied" timestamp');
});
```

#### T3.3 前端：侧边栏状态增强

- 侧边栏 StatusBar 增加 "Last Applied" 时间戳
- 增加 "Pending Changes" 提示（有未应用的变更时）
- 增加 "Apply" 快捷入口

**预估**：1.5 天

---

### T4：Windows 基础适配

**类型**：TDD（Rust 后端）

**依赖**：T0

**交付物**：`platform/windows.rs` + 平台抽象层完善 + 构建配置 + 测试文件

**内容**：

#### T4.1 Rust 后端：完善平台抽象层

**注意**：当前 `src-tauri/src/commands/apply.rs` 中 `read_system_hosts` 和 `generate_apply_plan` 硬编码 `/etc/hosts`（第 10、34 行），`writer.rs` 中 `HostsWriter::new()` 同样硬编码（第 94 行）。T4 需要将所有硬编码路径统一改为通过 `PlatformAdapter::hosts_path()` 获取。

当前 `flush_dns_cache()` 使用 `type_id()` 判断 mover 类型（`writer.rs` 第 391 行），这是脆弱设计。T4 将 `flush_dns_cache()` 纳入 `PlatformAdapter` trait，消除 `type_id` hack。

```rust
// mhost-apply/src/platform/mod.rs
pub trait PlatformAdapter {
    fn hosts_path() -> PathBuf;
    fn elevated_move(from: &Path, to: &Path) -> Result<(), MhostError>;
    fn flush_dns_cache() -> Result<(), MhostError>;
    fn platform_name() -> &'static str;
}
```

**T4 需要修改的文件清单**：
- `mhost-apply/src/writer/content.rs`：`HostsWriter::new()` 中的 `hosts_path` 改为从 `PlatformAdapter` 获取
- `mhost-apply/src/writer/mod.rs`：`flush_dns_cache()` 改为调用 `PlatformAdapter::flush_dns_cache()`，移除 `type_id` hack
- `src-tauri/src/commands/apply.rs`：`generate_apply_plan`、`read_system_hosts` 中的硬编码 `/etc/hosts` 改为通过 `AppState` 或 `PlatformAdapter` 获取

#### T4.2 Windows 实现

```rust
// mhost-apply/src/platform/windows.rs
impl PlatformAdapter for WindowsAdapter {
    fn hosts_path() -> PathBuf {
        // C:\Windows\System32\drivers\etc\hosts
    }
    
    fn elevated_move(from: &Path, to: &Path) -> Result<(), MhostError> {
        // 使用 PowerShell Start-Process -Verb RunAs 执行提权复制
        // 格式：powershell -Command "Start-Process cmd -ArgumentList '/c copy /Y ...' -Verb RunAs -Wait"
    }
    
    fn flush_dns_cache() -> Result<(), MhostError> {
        // ipconfig /flushdns
    }
}
```

**TDD 测试用例**：

```rust
#[test]
fn test_windows_hosts_path() {
    // 验证返回正确路径 C:\Windows\System32\drivers\etc\hosts
}

#[test]
fn test_windows_elevated_move_command_format() {
    // 验证生成的 PowerShell 命令格式正确
    // 不需要实际执行，只验证命令字符串
}

#[test]
fn test_windows_flush_dns_command_format() {
    // 验证生成的 ipconfig /flushdns 命令格式正确
}
```

#### T4.3 构建配置

- `tauri.conf.json` 添加 Windows 构建目标
- CI 添加 Windows 构建步骤

**环境要求**：需要 Windows 物理机或虚拟机进行实际测试。

**预估**：2.5 天

---

### T5：应用确认流程 + 回滚 UI

**类型**：TDD（前端）

**依赖**：T1, T3

**交付物**：`ApplyConfirmDialog` 组件 + `RollbackButton` 组件 + 测试文件

**复用已有能力**：store 中已有 `applyPlanAtom`、`isApplyingAtom`、`generateApplyPlanActionAtom`、`applyHostsActionAtom`。T5 直接使用这些 atom。`rollbackHostsActionAtom` 在 T0.2 中新增（当前 stores 中缺失，T5 需要它）。不创建新的后端 command。

**内容**：

#### T5.1 前端：ApplyConfirmDialog 组件

**功能**：
- 展示 ApplyPlan 预览（diff：新增行、删除行、未变更行），复用 `mhost-apply::diff` 的已有能力
- 冲突检测展示，复用 `mhost-apply::conflict` 的已有能力
- 确认/取消按钮
- 应用进度指示（使用 `isApplyingAtom`）
- 应用结果反馈（成功/失败）
- 失败时提供回滚入口

**TDD 测试用例**：

```typescript
describe('ApplyConfirmDialog', () => {
  it('shows diff preview before applying');
  it('shows added/removed/unchanged lines with color coding');
  it('shows conflict warnings when conflicts exist');
  it('blocks apply when conflicts exist');
  it('shows progress indicator during apply');
  it('shows success message after apply');
  it('shows error details and rollback button on failure');
});
```

#### T5.2 前端：Rollback 功能

- Settings 页面或 ApplyStatus 页面添加 "Rollback" 按钮
- 回滚确认弹窗
- 回滚结果反馈

**TDD 测试用例**：

```typescript
describe('RollbackButton', () => {
  it('shows confirmation dialog before rollback');
  it('rolls back to previous backup on confirm');
  it('shows success message after rollback');
  it('shows error when no backup exists');
});
```

**预估**：1.5 天

---

### T7：集成验收

**类型**：集成测试

**依赖**：T1-T5

**交付物**：集成测试用例 + 验收报告

**内容**：

端到端验收流程：

1. 创建 Profile "Development"（含规则 `127.0.0.1 api.example.com`）
2. 创建 Profile "Testing"（含规则 `192.168.10.12 api.example.com`）
3. 切换到 "Development" → 验证 Generate Plan 正确
4. 应用 → 验证 `/etc/hosts` 托管区块内容
5. 验证备份文件存在
6. 切换到 "Testing" → 验证旧规则被替换
7. 导入外部 hosts 文件为新 Profile
8. 导出 Profile 为 hosts 文件 → 验证内容正确
9. 复制 "Development" 为 "Development Copy" → 验证内容一致（新 ID、disabled）
10. 回滚 → 验证恢复
11. 编辑规则时输入无效 IP → 验证错误高亮提示
12. 保存时验证无错误规则才能保存
13. 导入同名 Profile → 验证名称自动追加后缀
14. 导出 JSON → 验证 schema 与导入兼容

**Gating 规则**：
- 规则格式错误不可写入生效配置
- 应用失败不破坏原系统 hosts
- 切换后用户可判断当前启用了哪个 Profile

**预估**：1.5 天

---

## 6. 任务依赖图

```
T0（工程拆分）
├── T1（文本编辑器 + 语法检查）──┐
├── T2（导入导出 + 复制）────────┤
│                                ├── T5（应用确认 + 回滚 UI）
├── T3（生效状态展示）───────────┤
│                                │
└── T4（Windows 适配）───────────┤
                                 │
                                 └── T7（集成验收）
```

**并行策略**：
- T0 完成后，T1/T2/T3/T4 可并行开发
- T3 对 T1 的依赖是弱依赖（T3 先做只读展示，T1 完成后再增强为可编辑）
- T5 必须在 T1 和 T3 都完成后才能开始

---

## 7. 团队组建

| 角色 | 数量 | 负责内容 |
|------|------|----------|
| Backend Developer A | 1 | T0.3（Rust 拆分）+ T1.1（Rust 校验）+ T4（Windows 适配） |
| Backend Developer B | 1 | T2.1-T2.3（Rust 导入导出/复制）+ T3.1（托管区块提取） |
| Frontend Developer | 1 | T0.1/T0.2/T0.4（CSS/Store/组件拆分）+ T1.2/T1.3（编辑器）+ T2.4（导入导出 UI）+ T3.2/T3.3（状态展示）+ T5（应用确认） |
| QA | 1 | T7（集成验收） |
| Code Reviewer | 1 | 审查 TDD 测试完整性、代码质量、拆分合理性 |

**并行点**：
- T0 完成后，Backend A/B 和 Frontend Developer 可并行开发
- QA 在 T5 完成后执行 T7

---

## 8. 阶段产出物

| 产出物 | 说明 |
|--------|------|
| 重构后的前端工程 | CSS Modules 混合拆分、Store 拆分、组件拆分 |
| 重构后的 Rust 工程 | `mhost-apply` writer 子模块拆分，`OsascriptMover` 移入 `platform/macos.rs` |
| `RuleEditor` 组件 | hosts 文本编辑器 + 实时语法校验（复用 `Parser::parse()`） |
| `validate_hosts_text` command | 薄封装 `Parser::parse()`，返回 `ParseResult` |
| `get_managed_block_content` command | 提取系统 hosts 托管区块内容 |
| `get_last_applied` command | 读取上次应用时间戳 |
| 导入导出命令 | `import_profile`（含名称冲突处理）、`export_profile`、`duplicate_profile`、`read_file_text`、`write_file_text` |
| `ApplyStatus` 组件 | 当前生效状态展示（使用已有 `readSystemHosts` + 新增 `getManagedBlockContent`） |
| `ApplyConfirmDialog` 组件 | 应用确认流程（复用已有 `applyPlanAtom`、`isApplyingAtom` 等） |
| Rollback UI | 前端回滚入口（复用已有 `rollbackHostsActionAtom`） |
| Windows 平台适配 | `platform/windows.rs` + 构建配置 |
| 单元测试覆盖 | 前后端 TDD 测试 |
| 集成测试 | 端到端验收流程（14 个验收步骤） |

---

## 9. 阶段 1 不做的事

| 排除项 | 原因 |
|--------|------|
| 广告屏蔽模块 | 阶段 3 范围 |
| 远程规则订阅 | 阶段 3 范围 |
| 白名单功能 | 阶段 3 范围 |
| 本地 DNS 模式 | 技术决策已排除 |
| 语法高亮编辑器 | 阶段 2 范围（先做纯文本，再做高亮） |
| 系统托盘 | 阶段 2 范围 |
| 查找替换 | 阶段 2 范围 |
| 回收站 | 阶段 2 范围 |
| 备份管理面板 | 阶段 2 范围 |
| 诊断工具 | 阶段 5 范围 |
| 表格化规则编辑器 | 阶段 2 范围（Phase 1 只做文本编辑器） |
| 多 Profile 同时启用 | 阶段 2 评估 |
| 错误处理架构重构 | 阶段 2 范围（Phase 1 各组件使用局部错误状态） |
| 增量合并导入 | 阶段 2 范围（Phase 1 只支持新建导入） |
| 应用签名 + 代码签名 | 正式发布前（Phase 1 开发阶段使用自签名） |
| Privileged Helper / SMJobBless | 正式发布前评估（Phase 1 继续使用 osascript 弹窗授权） |

---

## 10. 验收标准

- [ ] `cargo test` 全部通过（新增测试覆盖 T1-T4 所有后端功能）
- [ ] `pnpm vitest run` 全部通过（新增测试覆盖 T1-T5 所有前端组件）
- [ ] CI 检查（format、clippy、test、build）全部通过
- [ ] hosts 文本编辑器支持实时语法校验（300ms debounce），错误行高亮
- [ ] 规则格式错误不允许保存到 Profile
- [ ] Profile 导入（hosts 文本）→ 导出（hosts 文件）→ 再导入，规则一致
- [ ] 导入同名 Profile 自动追加数字后缀，不覆盖已有 Profile
- [ ] Profile 复制功能正确（新 UUID、新名称、disabled 状态、新时间戳）
- [ ] 应用前展示 diff 预览（新增行/删除行/未变更行），用户确认后才写入
- [ ] 应用失败不破坏原系统 hosts，可回滚
- [ ] 前端可查看当前生效的托管区块内容
- [ ] 前端可查看 "Last Applied" 时间戳和 "Pending Changes" 提示
- [ ] Windows 构建通过（hosts 路径、权限提升命令格式、DNS 刷新）
- [ ] 集成测试验证完整端到端流程（14 个验收步骤）

---

## 11. 风险与应对

| 优先级 | 风险 | 影响 | 应对 |
|--------|------|------|------|
| **高** | Windows 权限提升不稳定 | 中 | 先调研 runas/PowerShell 方案，在 Windows 测试环境验证 |
| **高** | `read_system_hosts`/`generate_apply_plan` 硬编码 `/etc/hosts`，T4 适配后可能遗漏 | 中 | T4 中明确扫描所有硬编码路径，统一改为 `PlatformAdapter::hosts_path()`；T4 包含修改文件清单 |
| **中** | CSS Modules 迁移引入样式回归 | 中 | 逐组件迁移 + visually 验证；混合策略降低风险（通用样式保留 global.css） |
| **中** | `@tauri-apps/plugin-dialog` 只返回路径不读写文件，导入导出链路断裂 | 中 | T0.5 前置验证 dialog 插件能力；T2 补充 `read_file_text`/`write_file_text` command |
| **中** | 大文件导入性能瓶颈 | 低 | 校验在 Rust 端执行，性能足够；导入时设置最大行数限制（如 10000 行） |
| **低** | 实时校验性能（大规则文件） | 低 | 校验在 Rust 端执行，性能足够；前端做 debounce（300ms） |
| **低** | Rust 代码拆分导致测试重写 | 低 | 拆分时保持模块公开接口不变，测试只改 import 路径 |
| **低** | CSS Modules 与 Vite HMR 兼容性 | 低 | 影响开发体验但不影响构建产物；如遇到问题，手动刷新 |

---

## 12. 预估总工期

| 任务 | 预估 |
|------|------|
| T0：工程拆分 | 1.5 天 |
| T1：文本编辑器 + 语法检查 | 3 天 |
| T2：导入导出 + 复制 | 2 天 |
| T3：生效状态展示 | 1.5 天 |
| T4：Windows 适配 | 2.5 天 |
| T5：应用确认 + 回滚 UI | 1.5 天 |
| T7：集成验收 | 1.5 天 |
| **合计** | **约 13.5 天** |

**并行后实际工期**：T0(1.5) + max(T1,T2,T3,T4)(3) + T5(1.5) + T7(1.5) ≈ **7.5 个工作日**。

---

## 附录 A：技术决策补充（新开发者必读）

本附录记录了二次 Review 中发现的需要确认的技术决策，供新开发者直接参考，无需额外询问。

---

### A.1 `ParseError` 行号扩展方案（阻塞级）

**问题**：当前 `ParseError` 是枚举（`InvalidIp(String)`、`InvalidDomain(String)`、`MalformedLine(String)`），直接给枚举加 `line_number` 字段需要修改所有变体签名，影响 122 个已有测试。

**决策**：采用**包装类型方案**，不修改 `ParseError` 枚举本身。

```rust
// mhost-hosts/src/parser.rs

/// 带行号的解析错误
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseErrorAtLine {
    pub line_number: usize,       // 1-based
    pub error: ParseError,       // 原始 ParseError，不修改
}

/// 带 Tauri 序列化的解析结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateResult {
    pub rules: Vec<HostRule>,
    pub errors: Vec<ParseErrorAtLine>,
}
```

**实现要点**：
- `ParseError` 枚举**不变**，已有 122 个测试**不受影响**
- `Parser::parse()` 返回 `ParseResult`（不变），新增 `Parser::parse_with_lines()` 返回 `ValidateResult`
- `validate_hosts_text` command 返回 `ValidateResult`（不是 `ParseResult`）
- `ParseErrorAtLine` 从 `ParseResult` 转换：遍历 `input.lines()`，对每个错误行记录行号

**对已有测试的影响**：零。`Parser::parse()` 和 `ParseResult` 完全不变。新增的 `parse_with_lines()` 和 `ValidateResult` 是纯新增代码。

---

### A.2 `PlatformAdapter` trait 修正设计（阻塞级）

**问题**：文档原设计使用静态方法（`fn hosts_path() -> PathBuf`），但 `HostsWriter` 需要运行时多态（`Box<dyn Trait>`），静态方法无法用于 trait object。

**决策**：改为**实例方法 + 工厂函数**模式。

```rust
// mhost-apply/src/platform/mod.rs

/// 平台适配器 trait（实例方法，支持 Box<dyn PlatformAdapter>）
pub trait PlatformAdapter: Send + Sync {
    fn hosts_path(&self) -> PathBuf;
    fn elevated_move(&self, from: &Path, to: &Path) -> Result<(), MhostError>;
    fn flush_dns_cache(&self) -> Result<(), MhostError>;
    fn platform_name(&self) -> &'static str;
}

/// 工厂函数：根据编译目标返回对应平台适配器
pub fn create_platform_adapter() -> Box<dyn PlatformAdapter> {
    #[cfg(target_os = "macos")]
    {
        Box::new(MacOsAdapter)
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(WindowsAdapter)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        panic!("Unsupported platform")
    }
}
```

**`HostsWriter` 迁移方案**：

```rust
pub struct HostsWriter {
    hosts_path: PathBuf,
    backup_dir: PathBuf,
    platform: Box<dyn PlatformAdapter>,  // 替换原来的 mover: Box<dyn ElevatedMover>
}
```

- `ElevatedMover` trait **保留**但标记为 `#[deprecated]`，`OsascriptMover` 改为实现 `PlatformAdapter`
- `HostsWriter::new()` 调用 `create_platform_adapter()` 获取平台适配器
- `flush_dns_cache()` 直接调用 `self.platform.flush_dns_cache()`，**消除 `type_id` hack**
- `atomic_write()` 调用 `self.platform.elevated_move()`，替代 `self.mover.elevated_move()`

**对已有测试的影响**：`HostsWriter::with_paths()` 和 `HostsWriter::with_mover()` 保留（`with_mover` 内部包装为 `PlatformAdapter`），已有 17 个 writer 测试**不需要修改**。

---

### A.3 `rollbackHostsActionAtom` 接口定义（阻塞级）

**问题**：当前 `src/stores/profiles.ts` 中没有 `rollbackHostsActionAtom`，但 `src/lib/tauri.ts` 中也没有 `rollbackHosts()` 的 JS 包装函数。后端 `rollback_hosts` command 已存在。

**决策**：在 T0.2 中同时新增以下内容：

1. `src/lib/tauri.ts` 新增：
```typescript
export async function rollbackHosts(): Promise<void> {
  return invoke('rollback_hosts');
}
```

2. `src/stores/profiles/actions.ts` 新增：
```typescript
export const rollbackHostsActionAtom = atom(null, async (get, set) => {
  try {
    await rollbackHosts();
    // 回滚成功后刷新状态
    await get(fetchProfilesAtom);
  } catch (e) {
    console.error('Rollback failed:', e);
    throw e;
  }
});
```

---

### A.4 `ExportFormat` 定义（阻塞级）

**问题**：`export_profile` command 使用 `ExportFormat` 类型，当前代码中不存在。

**决策**：在 `mhost-core/src/models.rs` 中定义（与 Profile 等核心类型放在一起）：

```rust
// mhost-core/src/models.rs

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Hosts,
    Json,
}
```

前端 `src/types/index.ts` 同步定义：
```typescript
export type ExportFormat = 'hosts' | 'json';
```

---

### A.5 `get_managed_block_content` 实现方案（重要级）

**问题**：`read_system_hosts` 是 Tauri command，`get_managed_block_content` 也是 command，command 之间不能互相调用。

**决策**：在 `mhost-hosts/src/parser.rs` 中新增一个纯函数（与 `extract_managed_block` 同级）：

```rust
// mhost-hosts/src/parser.rs

impl Parser {
    /// 从 hosts 文本中提取托管区块的内容字符串。
    /// 返回 Some(区块内容) 或 None（无托管区块）。
    pub fn extract_managed_block_content(input: &str) -> Option<String> {
        let (start, end) = Self::extract_managed_block(input)?;
        let lines: Vec<&str> = input.lines().collect();
        // start 和 end 是标记行，内容在标记之间
        if end <= start + 1 {
            return Some(String::new());
        }
        let content_lines = &lines[start + 1..end];
        Some(content_lines.join("\n"))
    }
}
```

`get_managed_block_content` command 内部：读取系统 hosts → 调用 `Parser::extract_managed_block_content()` → 返回结果。

---

### A.6 `last_applied` 时间戳存储方案（重要级）

**问题**：文档原说写入 `~/.local/share/mHost/last_applied`，但 macOS 上 `FileStorage` 使用 `~/Library/Application Support/mHost`。

**决策**：统一使用 `dirs::data_dir().join("mHost")`（即 `FileStorage` 的根目录），写入 `{root}/last_applied.json`。

```json
// last_applied.json 内容
{ "timestamp": "2026-06-20T14:30:00Z" }
```

- 格式：ISO 8601 字符串（与 `Profile` 的 `DateTime<Utc>` 一致）
- 读取：`get_last_applied` command 读取此文件，返回 `Option<String>`（ISO 8601 或 null）
- 写入时机：`apply_hosts` command 成功后写入
- 路径获取：通过 `AppState` 中的 `storage` 获取根目录（`FileStorage` 暴露 `root()` 方法）

---

### A.7 `read_file_text` / `write_file_text` 安全边界（重要级）

**问题**：这两个 command 可以读写系统任意文件，存在安全风险。

**决策**：
- **路径限制**：只允许读写用户选择的路径（由前端 `@tauri-apps/plugin-dialog` 返回），Rust 端不做路径白名单限制（因为路径来自用户自己的选择，不是外部输入）
- **文件大小限制**：`read_file_text` 限制最大 1MB（约 10000 行），超出返回错误
- **文件类型限制**：不做扩展名限制（用户可能导入 `.txt`、`.hosts`、无扩展名等文件）
- **写入覆盖确认**：由前端 save dialog 处理（Tauri dialog 的 save 模式会提示覆盖）

```rust
const MAX_FILE_READ_SIZE: usize = 1_048_576; // 1MB

#[tauri::command]
pub fn read_file_text(path: String) -> Result<String, MhostError> {
    let metadata = std::fs::metadata(&path)?;
    if metadata.len() > MAX_FILE_READ_SIZE as u64 {
        return Err(MhostError::InvalidInput(
            format!("File too large (max {} bytes)", MAX_FILE_READ_SIZE)
        ));
    }
    std::fs::read_to_string(&path).map_err(Into::into)
}
```

---

### A.8 `RuleEditor` 的 ID 处理策略（重要级）

**问题**：`HostRule` 包含 `id: RuleId(UUID)`，文本编辑后解析出的规则是新的（没有原始 ID），导致 React key 变化。

**决策**：**每次文本编辑提交时替换整个 `rules` 数组，生成全新 UUID**。不做 ID 匹配保留。

理由：
- Phase 1 的文本编辑器是全量替换模式（textarea），不是逐行编辑
- ID 匹配（按 IP+domain）在多域名规则、注释变更等场景下行为不明确
- 撤销/重做功能留到 Phase 2（表格编辑器）
- React key 使用数组索引而非 RuleId，避免不必要的重渲染

**前端实现**：
```typescript
// RuleEditor 内部状态管理
const [text, setText] = useState(formatRulesToText(rules));
const [errors, setErrors] = useState<ParseErrorAtLine[]>([]);

// onChange 只在校验通过时调用，传入全新 HostRule[]（新 UUID）
const handleChange = useCallback(() => {
  const result = await validateHostsText(text);
  if (result.errors.length === 0) {
    onChange(result.rules); // 全新规则，新 UUID
  }
}, [text]);
```

---

### A.9 冲突阻止策略（重要级）

**问题**：`apply_hosts` command 不检查冲突，但前端 UI 需要在冲突时阻止应用。

**决策**：**纯前端阻止**，后端不修改。

- `ApplyConfirmDialog` 检查 `applyPlan.conflicts.length > 0`，禁用确认按钮
- `apply_hosts` command 保持不变（不检查冲突），因为后端不应该替用户做决策
- 如果用户通过其他方式（如直接调用 API）绕过前端冲突检查，`apply_hosts` 仍然会执行（冲突规则会被合并引擎跳过，不会写入）

---

### A.10 `ParseResult` 序列化（重要级）

**问题**：`validate_hosts_text` 返回 `ValidateResult`（A.1 中定义），需要序列化。但 `ParseResult`（原 `Parser::parse()` 返回值）没有 `Serialize`/`Deserialize`。

**决策**：
- `ParseResult` **不添加** serde derive（保持纯内部类型，避免不必要的序列化约束）
- `ValidateResult`（新增类型）**添加** `Serialize`/`Deserialize`（Tauri command 返回值需要）
- `HostRule` 已有 `Serialize`/`Deserialize`，无需修改
- `ParseError` 已有 `Serialize`/`Deserialize`，无需修改
- `ParseErrorAtLine`（新增类型）添加 `Serialize`/`Deserialize`

---

### A.11 `format_rules` 与 `Parser::format` 去重（建议级）

**问题**：`mhost-hosts/src/formatter.rs` 的 `format_rules()` 和 `mhost-hosts/src/parser.rs` 的 `Parser::format()` 逻辑完全相同。

**决策**：T0.3 Rust 拆分时，将 `Parser::format()` 改为委托调用 `formatter::format_rules()`：

```rust
// parser.rs
impl Parser {
    pub fn format(rules: &[HostRule]) -> String {
        crate::formatter::format_rules(rules)
    }
}
```

保留 `Parser::format()` 作为便捷方法（已有测试使用），但实际逻辑只有一个实现。后续新代码统一使用 `formatter::format_rules()`。

---

### A.12 导入解析容错策略（建议级）

**问题**：严格拒绝任何解析错误可能导致用户无法导入真实世界的 hosts 文件（可能包含非标准格式）。

**决策**：**零错误容忍**，但提供清晰的错误反馈。

- `import_profile` 在解析有错误时返回错误，不创建 Profile
- 错误信息包含具体行号和原因（使用 `ValidateResult.errors`）
- 前端 ImportDialog 展示错误详情，用户可以修正后重新导入
- 不支持"跳过错误行"（避免部分导入导致用户困惑）

理由：hosts 文件格式简单，真实文件很少包含不可解析内容。如果遇到，用户可以手动编辑后重新导入。

---

### A.13 `duplicate_profile` 名称校验规则（建议级）

**问题**：`duplicate_profile` 的 `new_name` 参数校验规则未明确。

**决策**：与 `import_profile` 一致——同名时追加数字后缀（如 "Development (2)"）。

- `new_name` 不允许空字符串（返回 `InvalidInput` 错误）
- `new_name` 与已有 Profile 同名时，自动追加后缀
- 新 Profile 状态为 `disabled`（不继承原 Profile 的 enabled 状态）
- 新 Profile 的 `created_at` 和 `updated_at` 为当前时间

---

### A.14 `import_profile` 名称默认值（建议级）

**问题**：导入时名称是否可以从文件名提取作为默认值。

**决策**：**名称由用户手动输入，不自动推断**。导入弹窗中名称输入框为空时禁用确认按钮。

理由：文件名可能与 Profile 用途无关（如 `hosts_backup_2024.txt`），自动推断可能误导用户。

---

### A.15 `ApplyStatus` "Pending Changes" 判断逻辑（建议级）

**问题**：如何判断"有未应用的变更"。

**决策**：**复用 `generate_apply_plan` 的结果**，不做额外计算。

- 前端在 `ApplyStatus` 组件中调用 `generateApplyPlanActionAtom`
- 如果 `applyPlan.diff.added.length > 0 || applyPlan.diff.removed.length > 0`，则显示 "Pending Changes"
- 不需要对比托管区块内容与 plan（`generate_apply_plan` 已经做了这个 diff）
- 如果 `generate_apply_plan` 调用失败（如无权限读取 `/etc/hosts`），不显示 "Pending Changes"

---

### A.16 前端 Tauri invoke mock 策略（建议级）

**问题**：前端 TDD 测试需要 mock `@tauri-apps/api/core` 的 `invoke` 函数。

**决策**：在 `src/test/setup.ts` 中统一 mock：

```typescript
// src/test/setup.ts
import { vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

// 导出类型化的 mock invoke 供测试使用
export const mockInvoke = vi.mocked(await import('@tauri-apps/api/core')).invoke;
```

各组件测试中通过 `vi.mocked(invoke)` 设置返回值：
```typescript
import { invoke } from '@tauri-apps/api/core';
vi.mocked(invoke).mockResolvedValue({ rules: [], errors: [] });
```