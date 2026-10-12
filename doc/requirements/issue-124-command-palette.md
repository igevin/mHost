# Issue #124：域名中心 Cmd+K 命令面板 — 实现总结

实现日期：2026-10-12（Phase 3）

## 目标回顾

把 Cmd+K 命令面板做成产品的命令中心：域名查找（含生效判定与跳转）、常用命令、
fuzzy 匹配、查询历史，全局任意路由可用。

## 架构总览

```txt
Cmd+K (useGlobalKeybindings, Layout 挂载)
   │
   ▼
CommandPalette (src/components/CommandPalette.tsx, portal → body)
   │  非空查询 ──100ms debounce──► queryDomainAtom ──IPC──► query_domain (Rust)
   │  空查询    → 历史(≤5) + 默认命令列表
   │  任意查询  → fuzzyRank 本地过滤命令表
   ▼
匹配行 Enter/点击 → navigate(/profiles|:id|/dns-profiles/:id|/hosts,
                              state: { focusLine }) → RuleEditor/SystemHosts
                              滚动 + gutter 高亮
```

## 后端：`query_domain`（src-tauri/src/commands/query.rs）

### 为什么不用 mhost-apply::Merger

`merge.rs` 按 `(domain, ip)` 去重且 `ResolvedRule` 不带行号 —— 查找 UI 两个都需要。
本模块走独立的**只读聚合**路径，绝不复用 Merger。

### 三个数据源与行号获取

1. **DNS 规则**：`storage.list_profiles_by_mode(Dns)` —— 与 `DnsServer::reload_rules`
   消费的是同一份列表，展示即解析器实际所供。仅当 DNS 模式开启且 profile/rule
   双启用时才 effective。
2. **Hosts profile**：`AppState::cached_profiles()`（issue #181 的缓存；所有变更
   IPC 都会失效化）。**行号 = 规则在 `Profile.rules` 中的 1-based 索引** ——
   profile 落盘是结构化 JSON（没有可 re-parse 的 hosts 源文本），而前端
   `RuleEditor.rulesToText` 恰好一条规则渲染一行，索引即编辑器行号。
3. **系统 hosts**：读 `/etc/hosts`，把 mHost managed block 的行**逐行置空**
   （不是删除）后再交给 `Parser::parse_with_lines` —— 行号因此保持原文件物理行号。
   managed block 内的规则已由 profile 匹配覆盖，不能重复出现。

### 优先级规则（文档化，与 issue 一致）

1. DNS 代理规则（仅 DNS 模式开启时）—— 高于一切。
2. 启用的 hosts profile —— 按 profile 列表顺序 first-wins（与 apply merger 的
   冲突规则一致）。
3. system hosts —— 兜底。

**非有效匹配**（disabled profile、profile 内被注释的 rule、DNS 模式关闭时的
DNS 规则）一律排在最后、UI 灰显、**永远不能成为 WIN**。排序键：
`(rank, fuzzy score desc, 源内顺序)`；排序后第一个 effective 匹配即 winner
（`is_winner=true`，有且仅有一个；全部不可赢时无 winner）。

### Fuzzy 匹配

手搓 subsequence matcher（`fuzzy_score`）：大小写不敏感；连续命中 +10、
段边界（`.`/`-`/`_` 之后或串首）+5、首个命中位置越晚扣分越多。
`src/lib/fuzzy.ts` 是它的 TS 镜像（命令/历史过滤用），两端打分哲学保持一致。
无任何 fuzzy 库依赖（issue 硬性要求）。

## 前端

- **`src/components/CommandPalette.tsx`**（+ `.module.css`）：overlay + 居中
  面板（top 12vh），输入框 + 可滚结果列表。行渲染：domain（fuzzy 命中字符
  `<mark>` 高亮）→ IP、WIN 徽标（accent 色）、来源标签（`profile · :行号` /
  `System hosts · :行号` / `DNS · profile · :行号`）。非有效行 `opacity: 0.45`。
  键盘：↑↓ 环绕导航、Enter 执行、Esc 关闭；点击行同样执行。
  匹配行渲染上限 50 条（大 profile 的首屏保护）。
- **`src/hooks/useGlobalKeybindings.ts`**：表格化 keybinding 注册（目前只有
  Cmd/Ctrl+K 开关面板；后续加绑定加一行即可）。面板打开时再按 Cmd+K = 关闭
  （toggle 语义）；面板内部按键不进这张表。
- **挂载点**：`Layout.tsx` 最外层 —— 任意路由可用。
- **状态**：`paletteOpenAtom` / `paletteQueryAtom` / `paletteResultsAtom` /
  `paletteHistoryAtom`（state.ts）+ `queryDomainAtom` / `pushPaletteHistoryAtom`
  （actions.ts）。历史 = 最近 5 条执行过的查询，`atomWithLocalStorage`
  （key `mhost.paletteHistory`）持久化，去重、最新在前。Enter 复用 = 点历史条目
  回填输入框并触发新一轮查询。
- **响应乱序守卫**：`queryDomainAtom` 在写 results 前比对当前 query，
  晚到的旧响应直接丢弃。
- **跳转接线**：`RuleEditor` 新增 `focusLine` prop（滚动 + gutter 高亮，
  只读模式同样生效，并同步 lastSel/lastLine 增量缓存）；`HostsProfileView` /
  `DnsProfileView` 从 `location.state.focusLine` 读入并透传；`SystemHosts`
  页用同一 state 滚 pre + 高亮 gutter（行高 19.2px = 12px × 1.6）。
  跳转纯导航，不进入编辑模式（只读纪律：面板永不改规则）。

## 命令表

| 命令 | 行为 |
| --- | --- |
| Enable/Disable profile X | 与侧栏 toggle 同路径：quickApplyOnToggle ON → `quickApplyToggleAtom`，否则 `previewApplyAtom`（hosts）；DNS profile → `toggleDnsProfileEnabledAtom` |
| Create new profile | 复用 `CreateProfileDialog` |
| Apply current profile | `previewApplyAtom({ id: enabled.id, enabled: true })`（重新 diff + 确认写盘） |
| Switch DNS ↔ Hosts mode | `toggleDnsModeAtom(!dnsEnabled)` |
| Open settings | navigate("/settings") |
| Import profile… | 复用 `ImportDialog`（见「偏差」） |

## 测试

- **Rust**（`commands/query.rs` 内联）：14 个新用例 —— winner 选择、首启用
  profile 获胜、被遮蔽项排序、disabled profile/rule 不可赢、DNS 开/关两个方向、
  system hosts 兜底、managed block 排除、mask 后行号保持、fuzzy 匹配与打分、
  wire 格式（tagged snake_case）。
- **前端**：`lib/__tests__/fuzzy.test.ts`（12 个：子序列/大小写/连续与边界
  打分/稳定排序）+ `components/__tests__/CommandPalette.test.tsx`（14 个：
  默认命令列表、debounce IPC、WIN/灰显/来源渲染、↑↓ 环绕 + Enter 跳转 +
  focusLine、/hosts 跳转、Esc/overlay 关闭、命令 fuzzy、Open settings 导航、
  Enable 命令走 preview 流、历史写入/回填/上限 5、乱序响应丢弃）+
  `Layout.test.tsx` 加 2 个（Cmd+K / Ctrl+K 开关）。

## 偏差说明

- **「Import from URL」未实现为 URL 导入**：产品没有 URL 导入的后端
  （ImportDialog 是粘贴文本/选文件）。命令实现为「Import profile…」，打开既有
  ImportDialog。若 issue 坚持 URL 导入，需要一个新 IPC（fetch + 解析），超出
  本次范围。
- **profile 行号语义**：issue 建议「profile 不存行号就用 parse_with_lines 重
  parse」——实际 profile 落盘是结构化 JSON 而非 hosts 文本，没有可重 parse 的
  源；采用「rules 数组索引 = 编辑器行号」的等价方案（依据是
  `RuleEditor.rulesToText` 一条规则一行）。
- **AdBlock 来源的规则不参与域名查找**：issue 的 Source 枚举只有
  SystemHosts/Profile/DnsRule 三种；adblock 条目量太大（十万级），逐条 fuzzy
  不值当，且它们统一应答 0.0.0.0/NXDOMAIN，查找价值低。
- **Enabled hosts profile 的「first-wins」按 storage 列表顺序**：hosts 模式同一
  时刻通常只有一个启用 profile（enable 会自动 disable 其他），多启用是异常态，
  此时顺序与 apply 合并器一致即可。
