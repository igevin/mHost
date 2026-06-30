# 功能 5：重复规则检测 — 详细开发计划

创建日期：2026-06-30

## 背景

同一 Profile 内存在重复域名时，可能导致预期外解析行为（系统 hosts 按第一匹配生效，后续重复规则被静默忽略）。需要在前端给出提示，帮助用户发现配置错误。

## 设计决策

| 决策 | 方案 | 理由 |
|------|------|------|
| 检测时机 | 文本编辑时实时检测（300ms debounce） | 与现有语法校验保持一致，用户即时获得反馈 |
| 检测范围 | 只检测 `enabled=true` 且有 IP 的规则 | 注释行和已禁用规则不参与生效，无需检测 |
| 错误分类 | `SameIp`(warning) / `DifferentIp`(error) | 同 IP 重复是冗余配置，不同 IP 可能是冲突 |
| 数据传递 | 扩展 `ValidateResult` 新增 `duplicates` 字段 | `ParseError` 表示语法错误，重复是逻辑问题，不扩展 |
| 行号来源 | 从 `HostRule.source` 中的 `Line(u32)` 提取 | `parse_with_lines` 已记录行号，无需新增字段 |

## 检测逻辑

```
输入: Vec<HostRule> (已解析的规则列表)
输出: Vec<DuplicateRule>

步骤:
1. 筛选出 enabled=true 且 ip.is_some() 的规则
2. 展开每个规则的 domains 列表，建立映射:
   domain -> Vec<(line_number, ip)>
3. 遍历每个 domain 的映射:
   a. 如果该 domain 只出现一次 → 跳过
   b. 如果该 domain 出现多次:
      - 收集所有出现行的 IP
      - 如果所有 IP 相同 → DuplicateKind::SameIp
      - 如果 IP 不同 → DuplicateKind::DifferentIp
      - 生成 DuplicateRule { domain, lines, kind }
4. 按 domain 字母顺序返回结果
```

## 后端开发任务

### 1. 新增核心数据结构（`src-tauri/crates/mhost-core/src/models.rs`）

在 `ParseError` 定义之后新增：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateRule {
    pub domain: String,
    pub lines: Vec<usize>,
    pub kind: DuplicateKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DuplicateKind {
    #[serde(rename = "same_ip")]
    SameIp,
    #[serde(rename = "different_ip")]
    DifferentIp,
}
```

### 2. 扩展 `ValidateResult`（`src-tauri/crates/mhost-hosts/src/parser.rs`）

```rust
pub struct ValidateResult {
    pub rules: Vec<HostRule>,
    pub errors: Vec<ParseErrorAtLine>,
    pub duplicates: Vec<DuplicateRule>,
}
```

修改 `parse_with_lines` 中 `ValidateResult` 的构造处，初始化 `duplicates: Vec::new()`。

### 3. 新增重复检测函数（`src-tauri/crates/mhost-hosts/src/validator.rs`）

```rust
use std::collections::BTreeMap;
use mhost_core::models::{DuplicateRule, DuplicateKind, HostRule, RuleSource};

pub fn check_duplicates(rules: &[HostRule]) -> Vec<DuplicateRule> {
    let mut domain_map: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();

    for rule in rules {
        if !rule.enabled || rule.ip.is_none() {
            continue;
        }
        let line_number = match rule.source {
            RuleSource::Line(n) => n as usize,
            _ => continue,
        };
        let ip_str = rule.ip.unwrap().to_string();
        for domain in &rule.domains {
            domain_map
                .entry(domain.clone())
                .or_default()
                .push((line_number, ip_str.clone()));
        }
    }

    let mut duplicates = Vec::new();
    for (domain, entries) in domain_map {
        if entries.len() < 2 {
            continue;
        }
        let lines: Vec<usize> = entries.iter().map(|(line, _)| *line).collect();
        let ips: Vec<String> = entries.iter().map(|(_, ip)| ip.clone()).collect();
        let all_same_ip = ips.iter().all(|ip| ip == &ips[0]);

        let kind = if all_same_ip {
            DuplicateKind::SameIp
        } else {
            DuplicateKind::DifferentIp
        };

        duplicates.push(DuplicateRule { domain, lines, kind });
    }

    duplicates
}
```

### 4. 在 `parse_with_lines` 中集成重复检测（`src-tauri/crates/mhost-hosts/src/parser.rs`）

在现有解析逻辑之后、返回 `ValidateResult` 之前，调用 `check_duplicates`：

```rust
let duplicates = check_duplicates(&rules);
ValidateResult { rules, errors, duplicates }
```

### 5. 在 `lib.rs` 中导出（`src-tauri/crates/mhost-hosts/src/lib.rs`）

确保 `validator` 模块的 `check_duplicates` 被导出（或直接通过 `parser.rs` 调用，不需要额外导出）。

### 6. 后端测试（`src-tauri/crates/mhost-hosts/src/parser.rs` 或新测试文件）

新增单元测试：
- `test_no_duplicates_for_unique_domains` — 无重复时返回空
- `test_same_ip_duplicates_detected` — 同域名同 IP 检测为 SameIp
- `test_different_ip_duplicates_detected` — 同域名不同 IP 检测为 DifferentIp
- `test_disabled_rules_not_checked` — 禁用规则不参与检测
- `test_comment_only_lines_not_checked` — 注释行不参与检测
- `test_duplicate_across_multiple_rules` — 同一 domain 分散在多个规则中也能检测

## 前端开发任务

### 1. 扩展 TypeScript 类型（`src/types/index.ts`）

```typescript
export interface DuplicateRule {
  domain: string;
  lines: number[];
  kind: "same_ip" | "different_ip";
}

export interface ValidateResult {
  rules: HostRule[];
  errors: ParseErrorAtLine[];
  duplicates: DuplicateRule[];
}
```

### 2. RuleEditor 展示重复检测（`src/components/RuleEditor.tsx`）

**现有错误列表渲染逻辑（约第 250-270 行）：**

修改 `errors.length > 0` 的条件渲染，增加 `duplicates.length > 0` 的展示：

```tsx
{(errors.length > 0 || (validateResult?.duplicates?.length ?? 0) > 0) && (
  <div className={styles.errorsList}>
    {errors.map((err) => (
      <div key={`${err.line_number}-${err.error}`} className={styles.errorItem}>
        Line {err.line_number}: {err.error}
      </div>
    ))}
    {validateResult?.duplicates?.map((dup) => (
      <div
        key={`dup-${dup.domain}`}
        className={
          dup.kind === "different_ip"
            ? styles.errorItem
            : styles.warningItem
        }
      >
        {dup.kind === "different_ip"
          ? `冲突: 域名 "${dup.domain}" 映射到不同 IP (行 ${dup.lines.join(", ")})`
          : `冗余: 域名 "${dup.domain}" 重复出现 (行 ${dup.lines.join(", ")})`}
      </div>
    ))}
  </div>
)}
```

注意：`validateResult` 需要从 Tauri 命令返回中获取。当前 `RuleEditor` 的 `errors` 是通过 `validateHostsText(text)` 获取的 `ValidateResult.errors`。需要扩展为同时获取 `duplicates`。

### 3. 新增 CSS 样式（`src/components/RuleEditor.module.css`）

在 `.errorItem` 之后新增：

```css
.warningItem {
  font-size: 12px;
  color: var(--color-warning);
  background: var(--color-warning-bg);
  border: 1px solid var(--color-warning-border);
  border-radius: var(--radius-sm);
  padding: 4px 8px;
}
```

### 4. 前端测试更新（`src/components/__tests__/RuleEditor.test.tsx`）

新增测试：
- 重复域名同 IP 时显示 warning 样式提示
- 重复域名不同 IP 时显示 error 样式提示
- 无重复时不显示重复提示

## 开发顺序

| 顺序 | 任务 | 涉及文件 | 预估工作量 |
|------|------|---------|-----------|
| 1 | 后端：新增 DuplicateRule/DuplicateKind | `mhost-core/src/models.rs` | 小 |
| 2 | 后端：扩展 ValidateResult | `mhost-hosts/src/parser.rs` | 小 |
| 3 | 后端：新增 check_duplicates | `mhost-hosts/src/validator.rs` | 小 |
| 4 | 后端：集成到 parse_with_lines | `mhost-hosts/src/parser.rs` | 小 |
| 5 | 后端：单元测试 | `mhost-hosts/src/parser.rs` (test mod) | 中 |
| 6 | 前端：扩展 TypeScript 类型 | `src/types/index.ts` | 小 |
| 7 | 前端：RuleEditor 展示重复检测 | `src/components/RuleEditor.tsx` | 小 |
| 8 | 前端：新增 CSS 样式 | `src/components/RuleEditor.module.css` | 小 |
| 9 | 前端：测试更新 | `src/components/__tests__/RuleEditor.test.tsx` | 中 |

## 验收标准

- [ ] 同一 Profile 内输入重复域名时，RuleEditor 底部实时显示提示
- [ ] 同域名同 IP → 黄色 warning 提示 "冗余: 域名 'xxx' 重复出现 (行 1, 3)"
- [ ] 同域名不同 IP → 红色 error 提示 "冲突: 域名 'xxx' 映射到不同 IP (行 1, 3)"
- [ ] 注释行和已禁用规则不参与重复检测
- [ ] 前后端测试全部通过
- [ ] TypeScript 和 Rust 编译无错误

## 已知注意事项

1. **`HostRule` 多 domains 处理**：一个 rule 可能包含多个 domains（如 `127.0.0.1 a.com b.com`），每个 domain 需要单独检测重复
2. **行号来源**：`parse_with_lines` 通过 `RuleSource::Line(n)` 记录行号，只有从文本解析的规则才有行号，从 storage 加载的规则（`RuleSource::Profile`）不参与检测
3. **前端 `RuleSource` 类型不一致**：前端 `RuleSource` 只有 `Manual/Remote/AdBlock` 三种变体，与后端的 `Line/Profile` 不同。这不影响本功能，因为重复检测信息通过 `duplicates` 字段独立传递
