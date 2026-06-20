import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import RuleEditor from "../RuleEditor";
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

  it("renders rules as hosts text with syntax highlighting", () => {
    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);

    const editor = screen.getByRole("textbox");
    expect(editor).toHaveTextContent("127.0.0.1");
    expect(editor).toHaveTextContent("localhost");
    expect(editor).toHaveTextContent("192.168.1.1");
    expect(editor).toHaveTextContent("example.com");

    // Check syntax highlighting: IP tokens should be in colored spans
    const spans = editor.querySelectorAll("span");
    const hasIpSpan = Array.from(spans).some((s) =>
      s.textContent?.includes("127.0.0.1"),
    );
    const hasDomainSpan = Array.from(spans).some((s) =>
      s.textContent?.includes("localhost"),
    );
    expect(hasIpSpan).toBe(true);
    expect(hasDomainSpan).toBe(true);
  });

  it.skip("shows validation errors inline", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [{ line_number: 2, error: "invalid IP address" }],
    });

    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);
    const editor = screen.getByRole("textbox");

    // Simulate typing by setting textContent and firing input event
    editor.textContent = "127.0.0.1 localhost\ninvalid-line";
    fireEvent.input(editor);

    // Advance debounce timer
    act(() => {
      vi.advanceTimersByTime(350);
    });

    // Flush async promise resolution
    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(screen.getByText(/Line 2.*invalid IP address/)).toBeInTheDocument();
  });

  it.skip("emits parsed rules on valid change", async () => {
    const newRules: HostRule[] = [
      makeRule({ id: "new-1", ip: "10.0.0.1", domains: ["test.com"] }),
    ];
    mockValidateHostsText.mockResolvedValue({
      rules: newRules,
      errors: [],
    });

    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);
    const editor = screen.getByRole("textbox");

    editor.textContent = "10.0.0.1 test.com";
    fireEvent.input(editor);

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(onChange).toHaveBeenCalledWith(newRules);
  });

  it.skip("does not emit onChange on invalid input", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [{ line_number: 1, error: "parse error" }],
    });

    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);
    const editor = screen.getByRole("textbox");

    editor.textContent = "bad input";
    fireEvent.input(editor);

    act(() => {
      vi.advanceTimersByTime(350);
    });

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    expect(mockValidateHostsText).toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();
  });

  it.skip("handles empty input", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [],
    });

    const onChange = vi.fn();
    render(<RuleEditor rules={sampleRules} onChange={onChange} />);
    const editor = screen.getByRole("textbox");

    editor.textContent = "";
    fireEvent.input(editor);

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

    const editor = screen.getByRole("textbox");
    expect(editor).toHaveAttribute("contenteditable", "false");
  });
});
