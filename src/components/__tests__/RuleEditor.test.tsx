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

  it("handles empty input", async () => {
    mockValidateHostsText.mockResolvedValue({
      rules: [],
      errors: [],
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
});
