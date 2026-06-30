# 功能4：查找和替换 — 开发计划

创建日期：2026-06-30

## 目标

用户在 RuleEditor 中可以搜索规则并批量替换 IP 或域名。

## 现状

- `RuleEditor` 是 textarea + 语法高亮 overlay 的叠加方案
- 无内置查找替换功能
- 浏览器原生 Ctrl+F/Cmd+F 可搜索但不能替换，且搜索结果不在高亮层显示
- `highlightText()` 函数生成高亮 HTML，可注入搜索匹配的 span

## 核心挑战与解决方案

**挑战：** 搜索匹配标记需要与语法高亮共存。搜索词可能跨越 HTML 标签边界（如搜索 `"127.0.0.1 localhost"` 会跨越 `tokenIp` 和 `tokenDomain`）。

**方案：** 在生成语法高亮 HTML 的过程中，同时处理搜索匹配。对每行原始文本先按搜索匹配边界分割为 segment，再对每个 segment 分别做语法高亮，最后将位于匹配区域内的 segment 包裹在 `<mark>` 标签中。

## 开发任务（纯前端，无后端）

### 1. 新增 SearchBar 组件

**文件：** `src/components/SearchBar.tsx`

- 搜索框 + 替换输入框（替换框默认折叠，点击 "Replace" 展开）
- 计数显示：如 "3/12"
- 导航按钮：上一个（↑）/ 下一个（↓）
- 操作按钮：Replace（替换当前）、Replace All（全部替换）
- 快捷键：
  - `Cmd/Ctrl + F` 打开搜索栏
  - `Esc` 关闭搜索栏
  - `Enter` 下一个匹配
  - `Shift + Enter` 上一个匹配

**Props 接口：**
```ts
interface SearchBarProps {
  visible: boolean;
  onClose: () => void;
  query: string;
  onQueryChange: (q: string) => void;
  replaceText: string;
  onReplaceTextChange: (t: string) => void;
  matchCount: number;
  currentMatchIndex: number;
  onPrev: () => void;
  onNext: () => void;
  onReplace: () => void;
  onReplaceAll: () => void;
  readOnly?: boolean;
}
```

### 2. RuleEditor 集成搜索状态与匹配计算

**文件：** `src/components/RuleEditor.tsx`

新增搜索相关状态：
- `searchQuery`, `replaceText`
- `matches: MatchInfo[]`（包含 `start, end, lineIndex`）
- `currentMatchIndex`
- `searchBarVisible`

**匹配计算函数 `findMatches(text, query)`：**
- 简单字符串匹配（字面量，case-insensitive）
- 每次 `text` 或 `searchQuery` 变化时重新计算

**快捷键监听：**
- `Cmd+F` → 打开搜索栏并聚焦搜索框
- `Esc` → 关闭搜索栏
- `Enter` / `Shift+Enter` → 导航匹配项

### 3. highlightText 改造以支持搜索标记

将 `highlightText(text)` 重构为 `highlightText(text, searchQuery, matches, activeMatchIndex)`。

**具体实现策略（按行处理）：**

```typescript
function highlightText(
  text: string,
  searchQuery: string,
  matches: MatchInfo[],
  activeMatchIndex: number
): string {
  if (!text) return "";
  const lines = text.split("\n");
  const lineStarts = lines.map((_, i) =>
    lines.slice(0, i).join("\n").length + (i > 0 ? 1 : 0)
  );

  return lines.map((line, lineIdx) => {
    const lineStart = lineStarts[lineIdx];
    const lineEnd = lineStart + line.length;
    const lineMatches = matches.filter(m => m.start < lineEnd && m.end > lineStart);

    if (lineMatches.length === 0) {
      return highlightLine(line);
    }

    const segments = splitLineByMatches(line, lineStart, lineMatches);
    return segments.map(seg => {
      const segHtml = highlightLine(seg.text);
      if (seg.isMatch) {
        const isActive = seg.matchIndex === activeMatchIndex;
        const markClass = isActive ? styles.searchMatchActive : styles.searchMatch;
        return `<mark class="${markClass}">${segHtml}</mark>`;
      }
      return segHtml;
    }).join("");
  }).join("\n");
}
```

### 4. 导航与滚动定位

- `currentMatchIndex` 从 0 循环到 `matches.length - 1`
- 通过 `matches[currentMatchIndex].lineIndex` 获取所在行
- 计算 scrollTop：`lineIndex * lineHeight`（lineHeight = 13px * 1.6 = 20.8px）
- 设置 `textareaRef.current.scrollTop = targetScrollTop`

### 5. 替换逻辑

**Replace（替换当前）：**
1. 获取当前匹配项
2. 用 `replaceText` 替换对应区间
3. 更新 `setText`，触发 `debouncedValidate`
4. 重新计算匹配

**Replace All（全部替换）：**
1. 从后往前遍历所有匹配项（避免偏移量变化）
2. 一次性替换所有匹配
3. 更新 `setText`，触发 `debouncedValidate`

### 6. 新增样式

**文件：** `src/components/RuleEditor.module.css`

新增：
- `.searchBar` — 搜索栏容器
- `.searchInput`, `.replaceInput` — 输入框
- `.matchCount` — 计数显示
- `.searchMatch` — 匹配项高亮（黄色背景）
- `.searchMatchActive` — 当前匹配项高亮（橙色背景）

## 涉及文件清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `src/components/SearchBar.tsx` | 新建 | 搜索栏 UI 组件 |
| `src/components/SearchBar.module.css` | 新建 | 搜索栏样式 |
| `src/components/RuleEditor.tsx` | 修改 | 集成搜索状态、匹配计算、替换逻辑、highlightText 改造 |
| `src/components/RuleEditor.module.css` | 修改 | 新增搜索匹配高亮样式、搜索栏样式 |
| `src/components/__tests__/RuleEditor.test.tsx` | 修改 | 新增搜索替换测试用例 |
| `src/components/__tests__/SearchBar.test.tsx` | 新建 | 搜索栏组件测试 |

## 测试策略

1. **搜索功能：** 输入搜索词后正确计算匹配数量；导航上一个/下一个循环正确；无匹配时显示 "0/0"
2. **替换功能：** Replace 只替换当前匹配项；Replace All 替换所有匹配项；替换后重新计算匹配
3. **高亮显示：** 匹配项在 highlight layer 中正确标记；当前匹配项使用 active 样式；搜索关闭后清除高亮
4. **快捷键：** Cmd+F 打开搜索栏；Esc 关闭搜索栏；Enter/Shift+Enter 导航

## 工作量评估

| 任务 | 预估 |
|------|------|
| SearchBar 组件 UI | 1h |
| 匹配计算 + highlightText 改造 | 2-3h |
| 导航与滚动定位 | 1h |
| 替换逻辑 | 1h |
| 快捷键监听 | 0.5h |
| 样式调整 | 1h |
| 测试编写 | 1.5h |
| **总计** | **约 8-9h（1 天）** |

## 风险与注意事项

1. **highlightText 改造风险：** 改造后的 HTML 生成必须仍然与 textarea 的文本位置严格对齐（字体、字号、padding、white-space 等完全一致）。
2. **性能：** 大文件搜索时，匹配计算和 HTML 重新生成可能有性能问题。可用 `useDeferredValue` 或 `useMemo` 缓存。
3. **readOnly 模式：** 搜索功能在 readOnly 模式下仍然可用（便于查看），但替换功能应禁用。
