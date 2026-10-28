import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import RuleEditor, { EDITOR_LINE_HEIGHT_PX } from "../RuleEditor";
import globalCss from "../../styles/global.css?raw";
import ruleEditorCss from "../RuleEditor.module.css?raw";
import hostsHighlightCss from "../../styles/hostsHighlight.css?raw";
import type { HostRule } from "../../types";

// Mock validateHostsText
const mockValidateHostsText = vi.fn();
vi.mock("../../lib/tauri", () => ({
  validateHostsText: (...args: unknown[]) => mockValidateHostsText(...args),
}));

function makeRule(overrides: Partial<HostRule> = {}): HostRule {
  return {
    id: "rule-001",
    ip: "127.0.0.1",
    domains: ["localhost"],
    enabled: true,
    comment: null,
    source: { type: "Manual" },
    ...overrides,
  };
}

const sampleRules: HostRule[] = [
  makeRule({ id: "r1", ip: "127.0.0.1", domains: ["localhost"], comment: "local" }),
  makeRule({ id: "r2", ip: "192.168.1.1", domains: ["example.com", "www.example.com"] }),
];

describe("RuleEditor", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders rules as hosts text with syntax highlighting layer", () => {
    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);

    const textarea = screen.getByRole("textbox");
    expect(textarea).toHaveValue("127.0.0.1 localhost # local\n192.168.1.1 example.com www.example.com");

    // Highlight layer should contain colored spans
    const highlightLayer = document.querySelector("[aria-hidden='true']");
    expect(highlightLayer).toBeInTheDocument();
    expect(highlightLayer).toHaveTextContent("127.0.0.1");
    expect(highlightLayer).toHaveTextContent("localhost");
  });

  it("shows validation errors inline", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [{ line_number: 2, error: "invalid IP address" }],
      duplicates: [],
    });

    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);
    const textarea = screen.getByRole("textbox");

    fireEvent.change(textarea, { target: { value: "127.0.0.1 localhost\ninvalid-line" } });

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(screen.getByText(/Line 2.*invalid IP address/)).toBeInTheDocument();
  });

  it("emits parsed rules on valid change", async () => {
    const newRules: HostRule[] = [
      makeRule({ id: "new-1", ip: "10.0.0.1", domains: ["test.com"] }),
    ];
    mockValidateHostsText.mockResolvedValue({
      rules: newRules,
      errors: [],
      duplicates: [],
    });

    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);
    const textarea = screen.getByRole("textbox");

    fireEvent.change(textarea, { target: { value: "10.0.0.1 test.com" } });

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(onChange).toHaveBeenCalledWith(newRules);
  });

  it("does not emit onChange on invalid input", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [{ line_number: 1, error: "parse error" }],
      duplicates: [],
    });

    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);
    const textarea = screen.getByRole("textbox");

    fireEvent.change(textarea, { target: { value: "bad input" } });

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(mockValidateHostsText).toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("rolls back the textarea when external rules change while onChange is blocked", async () => {
    // Regression (PR #239 review finding 1): typing must consume
    // `isEditingRef` on the keystroke-triggered effect run — the
    // pre-#226 code reset it there too. If the flag outlives the
    // keystroke, an external rules change arriving while onChange stays
    // blocked (validation errors) hits the mid-edit early-out and is
    // swallowed: the stale draft survives a profile switch instead of
    // being rolled back.
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [{ line_number: 1, error: "parse error" }],
      duplicates: [],
    });
    const onChange = vi.fn();
    const otherProfileRules: HostRule[] = [
      makeRule({ id: "r9", ip: "10.0.0.1", domains: ["other.example.com"] }),
    ];
    const { rerender } = render(<RuleEditor rules={sampleRules} onChange={onChange} />);
    const textarea = screen.getByRole("textbox");

    fireEvent.change(textarea, { target: { value: "bad input" } });
    act(() => {
      vi.advanceTimersByTime(350);
    });
    await act(async () => {
      await vi.runAllTimersAsync();
    });
    expect(onChange).not.toHaveBeenCalled();

    // Profile switch while the draft is stuck — must roll back.
    rerender(<RuleEditor rules={otherProfileRules} onChange={onChange} />);
    expect(textarea).toHaveValue("10.0.0.1 other.example.com");
  });

  it("handles empty input", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [],
      duplicates: [],
    });

    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);
    const textarea = screen.getByRole("textbox");

    fireEvent.change(textarea, { target: { value: "" } });

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("respects readOnly prop", () => {
    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} readOnly />);

    const textarea = screen.getByRole("textbox");
    expect(textarea).toHaveAttribute("readonly");
  });

  it("shows warning for duplicate domain with same IP", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [],
      duplicates: [
        { domain: "example.com", lines: [1, 3], kind: "same_ip" },
      ],
    });

    const onChange = vi.fn();
    const onErrorChange = vi.fn();
    render(<RuleEditor rules={[]} onChange={onChange} onErrorChange={onErrorChange} />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "127.0.0.1 example.com\n\n127.0.0.1 example.com" } });

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    const warningEl = screen.getByText(/Duplicate.*example\.com/);
    expect(warningEl).toBeInTheDocument();
    expect(warningEl.className).toMatch(/warningItem/);
    expect(onErrorChange).toHaveBeenCalledWith(false);
  });

  it("shows error for duplicate domain with different IP", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [],
      duplicates: [
        { domain: "example.com", lines: [1, 2], kind: "different_ip" },
      ],
    });

    const onChange = vi.fn();
    const onErrorChange = vi.fn();
    render(<RuleEditor rules={[]} onChange={onChange} onErrorChange={onErrorChange} />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "127.0.0.1 example.com\n192.168.1.1 example.com" } });

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    const errorEl = screen.getByText(/Conflict.*example\.com/);
    expect(errorEl).toBeInTheDocument();
    expect(errorEl.className).toMatch(/errorItem/);
    expect(onErrorChange).toHaveBeenCalledWith(true);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("does not show duplicate hint when no duplicates", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [],
      duplicates: [],
    });

    const onChange = vi.fn();
    render(<RuleEditor rules={[]} onChange={onChange} />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "127.0.0.1 example.com" } });

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(screen.queryByText(/Duplicate|Conflict/)).not.toBeInTheDocument();
  });

  describe("Search & Replace", () => {
    it("opens search bar on Cmd+F", () => {
      render(<RuleEditor rules={sampleRules} onChange={vi.fn()} />);
      expect(screen.queryByTestId("search-input")).not.toBeInTheDocument();

      fireEvent.keyDown(document, { key: "f", metaKey: true });
      expect(screen.getByTestId("search-input")).toBeInTheDocument();
    });

    it("opens search bar on Ctrl+F", () => {
      render(<RuleEditor rules={sampleRules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", ctrlKey: true });
      expect(screen.getByTestId("search-input")).toBeInTheDocument();
    });

    it("closes search bar on Esc when focus is outside search bar", () => {
      render(<RuleEditor rules={sampleRules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });
      expect(screen.getByTestId("search-input")).toBeInTheDocument();

      // Focus the textarea and press Esc
      const textarea = document.querySelector("textarea")!;
      textarea.focus();
      fireEvent.keyDown(document, { key: "Escape" });
      expect(screen.queryByTestId("search-input")).not.toBeInTheDocument();
    });

    it("highlights search matches in the highlight layer", () => {
      render(<RuleEditor rules={sampleRules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      const searchInput = screen.getByTestId("search-input");
      fireEvent.change(searchInput, { target: { value: "localhost" } });

      const highlightLayer = document.querySelector("[aria-hidden='true']");
      expect(highlightLayer).toBeInTheDocument();
      expect(highlightLayer!.innerHTML).toMatch(/searchMatch/);
    });

    it("shows active match with distinct style", () => {
      const rules = [
        makeRule({ id: "r1", ip: "127.0.0.1", domains: ["localhost", "localhost"] }),
      ];
      render(<RuleEditor rules={rules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      const searchInput = screen.getByTestId("search-input");
      fireEvent.change(searchInput, { target: { value: "localhost" } });

      const highlightLayer = document.querySelector("[aria-hidden='true']");
      expect(highlightLayer!.innerHTML).toMatch(/searchMatchActive/);
    });

    it("shows 0/0 when no matches", () => {
      render(<RuleEditor rules={sampleRules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      const searchInput = screen.getByTestId("search-input");
      fireEvent.change(searchInput, { target: { value: "nonexistent" } });

      expect(screen.getByText("0/0")).toBeInTheDocument();
    });

    it("navigates between matches with prev/next buttons", () => {
      const rules = [
        makeRule({ id: "r1", ip: "127.0.0.1", domains: ["localhost", "localhost", "localhost"] }),
      ];
      render(<RuleEditor rules={rules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      const searchInput = screen.getByTestId("search-input");
      fireEvent.change(searchInput, { target: { value: "localhost" } });

      // Should show 1/3
      expect(screen.getByText("1/3")).toBeInTheDocument();

      // Click next -> 2/3
      fireEvent.click(screen.getByTestId("next-button"));
      expect(screen.getByText("2/3")).toBeInTheDocument();

      // Click next -> 3/3
      fireEvent.click(screen.getByTestId("next-button"));
      expect(screen.getByText("3/3")).toBeInTheDocument();

      // Click next -> cycles to 1/3
      fireEvent.click(screen.getByTestId("next-button"));
      expect(screen.getByText("1/3")).toBeInTheDocument();

      // Click prev -> cycles to 3/3
      fireEvent.click(screen.getByTestId("prev-button"));
      expect(screen.getByText("3/3")).toBeInTheDocument();
    });

    it("navigates matches with Enter and Shift+Enter", () => {
      const rules = [
        makeRule({ id: "r1", ip: "127.0.0.1", domains: ["localhost", "localhost"] }),
      ];
      render(<RuleEditor rules={rules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      const searchInput = screen.getByTestId("search-input");
      fireEvent.change(searchInput, { target: { value: "localhost" } });

      expect(screen.getByText("1/2")).toBeInTheDocument();

      fireEvent.keyDown(searchInput, { key: "Enter", shiftKey: false });
      expect(screen.getByText("2/2")).toBeInTheDocument();

      fireEvent.keyDown(searchInput, { key: "Enter", shiftKey: true });
      expect(screen.getByText("1/2")).toBeInTheDocument();
    });

    it("handles search query with regex special characters", () => {
      const rules = [
        makeRule({ id: "r1", ip: "127.0.0.1", domains: ["test.com"] }),
      ];
      render(<RuleEditor rules={rules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      const searchInput = screen.getByTestId("search-input");
      // Searching for a literal dot should match the IP, not act as regex wildcard
      fireEvent.change(searchInput, { target: { value: "127.0.0" } });

      expect(screen.getByText("1/1")).toBeInTheDocument();
    });

    it("replaces current match", () => {
      mockValidateHostsText.mockResolvedValue({
        rules: [],
        errors: [],
        duplicates: [],
      });

      const rules = [
        makeRule({ id: "r1", ip: "127.0.0.1", domains: ["localhost", "localhost"] }),
      ];
      render(<RuleEditor rules={rules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      const searchInput = screen.getByTestId("search-input");
      fireEvent.change(searchInput, { target: { value: "localhost" } });

      fireEvent.click(screen.getByTestId("toggle-replace"));
      fireEvent.change(screen.getByTestId("replace-input"), { target: { value: "newhost" } });
      fireEvent.click(screen.getByTestId("replace-button"));

      const textarea = document.querySelector("textarea")!;
      expect(textarea).toHaveValue("127.0.0.1 newhost localhost");
    });

    it("replaces all matches", () => {
      mockValidateHostsText.mockResolvedValue({
        rules: [],
        errors: [],
        duplicates: [],
      });

      const rules = [
        makeRule({ id: "r1", ip: "127.0.0.1", domains: ["localhost", "localhost", "localhost"] }),
      ];
      render(<RuleEditor rules={rules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      const searchInput = screen.getByTestId("search-input");
      fireEvent.change(searchInput, { target: { value: "localhost" } });

      fireEvent.click(screen.getByTestId("toggle-replace"));
      fireEvent.change(screen.getByTestId("replace-input"), { target: { value: "newhost" } });
      fireEvent.click(screen.getByTestId("replace-all-button"));

      const textarea = document.querySelector("textarea")!;
      expect(textarea).toHaveValue("127.0.0.1 newhost newhost newhost");
    });

    it("hides replace functionality in readOnly mode", () => {
      render(<RuleEditor rules={sampleRules} onChange={vi.fn()} readOnly />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      expect(screen.getByTestId("search-input")).toBeInTheDocument();
      expect(screen.queryByTestId("toggle-replace")).not.toBeInTheDocument();
    });

    it("clears search highlights when search bar is closed", () => {
      render(<RuleEditor rules={sampleRules} onChange={vi.fn()} />);
      fireEvent.keyDown(document, { key: "f", metaKey: true });

      const searchInput = screen.getByTestId("search-input");
      fireEvent.change(searchInput, { target: { value: "localhost" } });

      let highlightLayer = document.querySelector("[aria-hidden='true']");
      expect(highlightLayer!.innerHTML).toMatch(/searchMatch/);

      // Close search bar via Escape inside search input
      fireEvent.keyDown(searchInput, { key: "Escape" });

      // Search bar should be gone
      expect(screen.queryByTestId("search-input")).not.toBeInTheDocument();

      // Highlights should be cleared (no searchMatch in highlight layer)
      highlightLayer = document.querySelector("[aria-hidden='true']");
      expect(highlightLayer!.innerHTML).not.toMatch(/searchMatch/);
    });
  });

  describe("focusLine jump target (issue #124)", () => {
    const fiveRules: HostRule[] = [1, 2, 3, 4, 5].map((i) =>
      makeRule({ id: `r${i}`, ip: "10.0.0.1", domains: [`d${i}.local`] }),
    );

    function activeGutterRow(): Element | null {
      const active = document.querySelectorAll("[class*='lineNumberActive']");
      expect(active.length).toBeLessThanOrEqual(1);
      return active[0] ?? null;
    }

    it("scrolls to the focus line and highlights its gutter row in read-only mode", () => {
      render(
        <RuleEditor rules={fiveRules} onChange={vi.fn()} readOnly focusLine={3} />,
      );

      expect(activeGutterRow()?.textContent).toBe("3");
      const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
      // issue #248: 行高用真实渲染值（20px），不再是 13px × 1.6 = 20.8px
      expect(textarea.scrollTop).toBe(2 * EDITOR_LINE_HEIGHT_PX);
    });

    it("highlights no gutter row in read-only mode without focusLine", () => {
      render(<RuleEditor rules={fiveRules} onChange={vi.fn()} readOnly />);
      expect(activeGutterRow()).toBeNull();
    });

    it("keeps the caret-line cache consistent after a jump", () => {
      render(<RuleEditor rules={fiveRules} onChange={vi.fn()} focusLine={3} />);

      // 跳转后用户把 caret 挪到第 5 行行首 —— delta walk 必须从
      // 「目标行行首」的自洽基准出发，算出绝对行号 5 而不是错位值。
      const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
      const lines = textarea.value.split("\n");
      const line5Start = lines
        .slice(0, 4)
        .reduce((n, l) => n + l.length + 1, 0);
      textarea.setSelectionRange(line5Start, line5Start);
      fireEvent.select(textarea);

      expect(activeGutterRow()?.textContent).toBe("5");
    });

    it("clamps an out-of-range focusLine to the last line", () => {
      render(
        <RuleEditor rules={fiveRules} onChange={vi.fn()} readOnly focusLine={99} />,
      );
      expect(activeGutterRow()?.textContent).toBe("5");
    });

    it("re-scrolls on a repeated jump to the same line via jumpNonce (F6)", () => {
      const { rerender } = render(
        <RuleEditor
          rules={fiveRules}
          onChange={vi.fn()}
          readOnly
          focusLine={2}
          jumpNonce={1}
        />,
      );
      const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
      expect(textarea.scrollTop).toBe(1 * EDITOR_LINE_HEIGHT_PX);

      // 用户手动滚走
      textarea.scrollTop = 0;

      // 同一行、新的 nonce —— 必须重新滚回目标行
      rerender(
        <RuleEditor
          rules={fiveRules}
          onChange={vi.fn()}
          readOnly
          focusLine={2}
          jumpNonce={2}
        />,
      );
      expect(textarea.scrollTop).toBe(1 * EDITOR_LINE_HEIGHT_PX);
      expect(activeGutterRow()?.textContent).toBe("2");
    });
  });
});

/**
 * issue #248 回归护栏。
 *
 * 这个 bug 只在真实渲染层可见：gutter 的行槽位与正文行盒差 0.8px/行，
 * 高亮层的合成粗体把字形步进撑宽 4.6% —— jsdom 不排版，DOM 断言抓不到
 * 度量类回归，所以这里直接对样式源码立规矩（vitest.config.ts 开了
 * `css: true`，`?raw` 才会返回样式原文）：
 *   1. 行高只有一个来源（global.css 的 token），且与 JS 的滚动换算一致；
 *   2. 编辑器各层不得自行声明行高 —— 只能继承 .editorWrapper 的那一份；
 *   3. hosts token 样式必须「度量中性」：字重不能触发合成粗体。
 */
describe("editor metrics invariants (issue #248)", () => {
  /** 去掉注释后再断言 —— 注释里会引用旧值做对比说明。 */
  function declarations(css: string): string {
    return css.replace(/\/\*[\s\S]*?\*\//g, "");
  }

  function block(css: string, selector: string): string {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const m = css.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
    expect(m, `找不到 ${selector} 的样式块`).not.toBeNull();
    return m![1];
  }

  it("keeps the CSS line-height token in sync with the JS scroll math", () => {
    const token = globalCss.match(/--editor-line-height:\s*([\d.]+)px/);
    expect(token).not.toBeNull();
    expect(Number(token![1])).toBe(EDITOR_LINE_HEIGHT_PX);
  });

  it("declares the line height once on the wrapper and inherits it everywhere else", () => {
    const css = declarations(ruleEditorCss);
    expect(block(css, ".editorWrapper")).toMatch(/line-height:\s*var\(--editor-line-height\)/);
    // gutter 与两层正文都继承同一个值 —— 各自再声明行高就会与继承值分叉
    for (const selector of [".lineNumbers", ".lineNumber", ".highlightLayer", ".textarea"]) {
      expect(block(css, selector), selector).not.toMatch(/line-height:/);
    }
    // 分数行高（1.6）在 WebKit 会被向下取整，与 min-height 的行槽位错开
    expect(css).not.toMatch(/line-height:\s*1\.\d/);
  });

  it("keeps the hosts token styles metric-neutral (no synthetic bold)", () => {
    const ip = block(declarations(hostsHighlightCss), ".hosts-token-ip");
    const weight = Number(ip.match(/font-weight:\s*(\d+)/)?.[1]);
    expect(weight).toBeGreaterThan(0);
    // JetBrains Mono 只打包 400/500；>=600 会命中合成粗体，撑宽字形步进
    expect(weight).toBeLessThanOrEqual(500);
  });
});
