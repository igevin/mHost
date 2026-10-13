import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router-dom";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import type { Profile, RuleMatch } from "../../types";
import {
  paletteOpenAtom,
  paletteQueryAtom,
  paletteResultsAtom,
  paletteHistoryAtom,
  profilesAtom,
  dnsProfilesAtom,
  dnsEnabledAtom,
  quickApplyOnToggleAtom,
} from "../../stores/profiles";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  save: vi.fn(),
}));

const MATCH_FIXTURE: RuleMatch[] = [
  {
    domain: "api.local",
    ip: "10.0.0.1",
    source: { type: "profile", id: "p1", name: "dev", line: 3, enabled: true },
    effective: true,
    is_winner: true,
    via_suffix: false,
  },
  {
    domain: "api.local",
    ip: "192.168.1.1",
    source: { type: "system_hosts", line: 7 },
    effective: true,
    is_winner: false,
    via_suffix: false,
  },
  {
    domain: "api.local",
    ip: "10.0.0.9",
    source: { type: "profile", id: "p2", name: "old", line: 1, enabled: false },
    effective: false,
    is_winner: false,
    via_suffix: false,
  },
];

const mockQueryDomain = vi.fn((query: string) =>
  Promise.resolve({
    query,
    matches: query === "api.local" ? MATCH_FIXTURE : [],
  }),
);

vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    queryDomain: (q: string) => mockQueryDomain(q),
    listProfiles: vi.fn().mockImplementation(async () => {
      return getDefaultStore().get(profilesAtom) ?? [];
    }),
  };
});

import CommandPalette from "../CommandPalette";
import ConfirmDialog from "../ConfirmDialog";

function makeProfile(overrides: Partial<Profile> = {}): Profile {
  return {
    id: "p1",
    name: "dev",
    description: null,
    enabled: false,
    protected: false,
    tags: [],
    rules: [],
    mode: "hosts",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function LocationProbe() {
  const location = useLocation();
  return (
    <div data-testid="location">
      {location.pathname}
      {location.state ? ` ${JSON.stringify(location.state)}` : ""}
    </div>
  );
}

function renderPalette() {
  return render(
    <JotaiProvider store={getDefaultStore()}>
      <MemoryRouter initialEntries={["/"]}>
        <CommandPalette />
        <LocationProbe />
      </MemoryRouter>
    </JotaiProvider>,
  );
}

function openPalette() {
  act(() => {
    getDefaultStore().set(paletteOpenAtom, true);
  });
}

async function typeQuery(text: string) {
  const input = screen.getByLabelText("Command palette input");
  fireEvent.change(input, { target: { value: text } });
  // 100ms debounce（issue #124）—— 用 waitFor 等 IPC 落地。
  await waitFor(() => expect(mockQueryDomain).toHaveBeenCalledWith(text));
}

describe("CommandPalette (issue #124)", () => {
  // 与 atomWithLocalStorage.test.ts 同款的内存 localStorage shim ——
  // 本仓库的 vitest 环境里 Node 实验性 localStorage 不可用。
  let mockStorage: Record<string, string>;

  beforeEach(() => {
    vi.clearAllMocks();
    // clearAllMocks 不恢复 mockImplementation —— 显式重置回默认实现
    // （stale-response 用例会替换它）。
    mockQueryDomain.mockImplementation((query: string) =>
      Promise.resolve({
        query,
        matches: query === "api.local" ? MATCH_FIXTURE : [],
      }),
    );
    mockStorage = {};
    globalThis.localStorage = {
      getItem: (k: string) => (k in mockStorage ? mockStorage[k] : null),
      setItem: (k: string, v: string) => {
        mockStorage[k] = v;
      },
      removeItem: (k: string) => {
        delete mockStorage[k];
      },
      clear: () => {
        for (const k of Object.keys(mockStorage)) delete mockStorage[k];
      },
      key: () => null,
      length: 0,
    } as Storage;
    const store = getDefaultStore();
    store.set(paletteOpenAtom, false);
    store.set(paletteQueryAtom, "");
    store.set(paletteResultsAtom, null);
    store.set(paletteHistoryAtom, []);
    store.set(profilesAtom, []);
    store.set(dnsProfilesAtom, []);
    store.set(dnsEnabledAtom, false);
    store.set(quickApplyOnToggleAtom, false);
  });

  it("renders nothing when closed", () => {
    renderPalette();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("shows the default command list when the query is empty", () => {
    renderPalette();
    openPalette();

    expect(screen.getByText("Create new profile")).toBeInTheDocument();
    expect(screen.getByText("Open settings")).toBeInTheDocument();
    expect(screen.getByText("Switch to DNS mode")).toBeInTheDocument();
    expect(screen.getByText("Import profile…")).toBeInTheDocument();
  });

  it("queries the backend after a 100ms debounce and renders matches with WIN + source + grayed rows", async () => {
    renderPalette();
    openPalette();
    await typeQuery("api.local");

    await waitFor(() => {
      expect(screen.getAllByText("api.local").length).toBeGreaterThanOrEqual(3);
    });

    // WIN 标记只出现在生效的 winner 行
    expect(screen.getByText("WIN")).toBeInTheDocument();
    // 来源标签：profile · 行号 / system hosts · 行号
    expect(screen.getByText("dev · :3")).toBeInTheDocument();
    expect(screen.getByText("System hosts · :7")).toBeInTheDocument();
    // disabled profile 的行灰显（rowDisabled class）
    const rows = screen.getAllByRole("option");
    const grayed = rows.filter((r) => r.className.includes("rowDisabled"));
    expect(grayed).toHaveLength(1);
    expect(grayed[0].textContent).toContain("10.0.0.9");
  });

  it("navigates with ↑↓ (wrapping) and Enter executes the active match as a jump", async () => {
    renderPalette();
    openPalette();
    await typeQuery("api.local");
    await waitFor(() => screen.getAllByText("api.local"));

    const input = screen.getByLabelText("Command palette input");
    const options = screen.getAllByRole("option");
    expect(options[0]).toHaveAttribute("aria-selected", "true");

    // ↓ 移动到第二条，↑ 回第一条，再 ↑ 环绕到末尾
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(screen.getAllByRole("option")[1]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(screen.getAllByRole("option")[0]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(input, { key: "ArrowUp" });
    const afterWrap = screen.getAllByRole("option");
    expect(afterWrap[afterWrap.length - 1]).toHaveAttribute("aria-selected", "true");

    // 回到第一条（winner，profile 来源）并 Enter → 跳转 + 行号
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });

    // F6: 跳转 state 带 jumpId nonce
    expect(screen.getByTestId("location")).toHaveTextContent(
      /\/profiles\/p1 \{"focusLine":3,"jumpId":\d+\}/,
    );
    // F1: 执行后关闭是 deferred（trailing-click 防护）—— 等它落地
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });

  it("clicking a system-hosts match jumps to /hosts with the file line", async () => {
    renderPalette();
    openPalette();
    await typeQuery("api.local");
    await waitFor(() => screen.getAllByText("api.local"));

    fireEvent.click(screen.getByText("System hosts · :7"));

    expect(screen.getByTestId("location")).toHaveTextContent(
      /\/hosts \{"focusLine":7,"jumpId":\d+\}/,
    );
  });

  it("Esc closes the palette", () => {
    renderPalette();
    openPalette();
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    fireEvent.keyDown(screen.getByLabelText("Command palette input"), {
      key: "Escape",
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("overlay click closes the palette", () => {
    renderPalette();
    openPalette();
    fireEvent.click(screen.getByTestId("palette-overlay"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("fuzzy-matches commands (subsequence, not just substring)", async () => {
    renderPalette();
    openPalette();
    await typeQuery("opset");

    expect(screen.getByText("Open settings")).toBeInTheDocument();
  });

  it("executing 'Open settings' navigates to /settings", async () => {
    renderPalette();
    openPalette();
    await typeQuery("settings");

    const input = screen.getByLabelText("Command palette input");
    // commands 排在 matches 之后；找到目标行的索引
    const options = screen.getAllByRole("option");
    const idx = options.findIndex((o) => o.textContent === "Open settings");
    expect(idx).toBeGreaterThanOrEqual(0);
    for (let i = 0; i < idx; i++) {
      fireEvent.keyDown(input, { key: "ArrowDown" });
    }
    fireEvent.keyDown(input, { key: "Enter" });

    expect(screen.getByTestId("location")).toHaveTextContent("/settings");
  });

  it("'Enable profile X' goes through the same preview flow as the sidebar toggle", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const store = getDefaultStore();
    store.set(profilesAtom, [makeProfile({ id: "p1", name: "dev", enabled: false })]);

    renderPalette();
    openPalette();
    await typeQuery("enable");

    const option = screen
      .getAllByRole("option")
      .find((o) => o.textContent === 'Enable profile "dev"');
    expect(option).toBeTruthy();
    fireEvent.click(option!);

    // quickApplyOnToggle 默认 OFF → 走 preview_apply_outcome（与侧栏一致）
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "preview_apply_outcome",
        expect.objectContaining({ id: "p1", enabled: true }),
      ),
    );
  });

  it("pushes executed queries to history (persisted, cap 5, deduped)", async () => {
    renderPalette();
    openPalette();
    await typeQuery("api.local");
    await waitFor(() => screen.getAllByText("api.local"));

    fireEvent.keyDown(screen.getByLabelText("Command palette input"), {
      key: "Enter",
    });

    // 写入 atom + localStorage
    expect(getDefaultStore().get(paletteHistoryAtom)).toEqual(["api.local"]);
    expect(JSON.parse(mockStorage["mhost.paletteHistory"])).toEqual([
      "api.local",
    ]);
  });

  it("shows history on reopen; selecting an entry refills the input (Enter reuses)", async () => {
    const store = getDefaultStore();
    store.set(paletteHistoryAtom, ["api.local"]);

    renderPalette();
    openPalette();

    const entry = screen.getByText("api.local");
    expect(entry).toBeInTheDocument();
    expect(screen.getByText("Recent")).toBeInTheDocument();

    fireEvent.click(entry);
    expect(screen.getByLabelText("Command palette input")).toHaveValue("api.local");
    // 回填触发新一轮查询
    await waitFor(() => expect(mockQueryDomain).toHaveBeenCalledWith("api.local"));
  });

  it("history is capped at 5 entries", async () => {
    renderPalette();
    const store = getDefaultStore();
    store.set(paletteHistoryAtom, ["q1", "q2", "q3", "q4", "q5"]);
    openPalette();
    await typeQuery("api.local");
    await waitFor(() => screen.getAllByText("api.local"));
    fireEvent.keyDown(screen.getByLabelText("Command palette input"), {
      key: "Enter",
    });

    const history = getDefaultStore().get(paletteHistoryAtom);
    expect(history).toEqual(["api.local", "q1", "q2", "q3", "q4"]);
    expect(history).toHaveLength(5);
  });

  it("discards stale IPC responses (response-order guard)", async () => {
    // 让慢查询（api.local）的响应晚于快查询（zzz）落地。
    let resolveSlow: (v: { query: string; matches: RuleMatch[] }) => void;
    mockQueryDomain.mockImplementation((q: string) => {
      if (q === "api.local") {
        return new Promise((r) => {
          resolveSlow = r;
        });
      }
      return Promise.resolve({ query: q, matches: [] });
    });

    renderPalette();
    openPalette();
    const input = screen.getByLabelText("Command palette input");
    fireEvent.change(input, { target: { value: "api.local" } });
    await waitFor(() => expect(mockQueryDomain).toHaveBeenCalledWith("api.local"));
    // 在慢响应挂起期间改查询
    fireEvent.change(input, { target: { value: "zzz" } });
    await waitFor(() => expect(mockQueryDomain).toHaveBeenCalledWith("zzz"));

    // 慢响应落地 —— 查询已是 "zzz"，旧响应必须被丢弃
    await act(async () => {
      resolveSlow!({ query: "api.local", matches: MATCH_FIXTURE });
    });
    expect(screen.queryByText("dev · :3")).not.toBeInTheDocument();
  });

  it("shows a 'suffix match' badge for resolver suffix hits (R1)", async () => {
    mockQueryDomain.mockImplementation((q: string) =>
      Promise.resolve({
        query: q,
        matches: [
          {
            domain: "example.com",
            ip: "0.0.0.0",
            source: { type: "dns_rule", id: "d1", name: "ads", line: 2 },
            effective: true,
            is_winner: true,
            via_suffix: true,
          } as RuleMatch,
        ],
      }),
    );

    renderPalette();
    openPalette();
    await typeQuery("ad.example.com");

    expect(screen.getByText("suffix match")).toBeInTheDocument();
    expect(screen.getByText("WIN")).toBeInTheDocument();
    expect(screen.getByText("DNS · ads · :2")).toBeInTheDocument();
  });

  // F1 回归：pointerdown 执行后面板必须保持挂载到 trailing click 之后，
  // 否则同一手势的 click 会穿透到面板下方的页面元素。
  it("defers close past the trailing click so it can't hit the page beneath (F1)", async () => {
    renderPalette();
    openPalette();
    await typeQuery("api.local");
    await waitFor(() => screen.getAllByText("api.local"));

    // pointerdown 执行跳转 —— 面板此时必须**还开着**（deferred close）。
    // 修复前这里同步 unmount，同一手势的 trailing click 会被 hit-test
    // 到面板下方的页面元素上（侧栏 toggle / Enable 按钮）。
    const row = screen.getAllByRole("option")[0];
    fireEvent.pointerDown(row, { button: 0 });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    // 执行（跳转）已经发生 —— 只有关闭被推迟
    expect(screen.getByTestId("location")).toHaveTextContent("/profiles/p1");

    // trailing click 落在仍挂载的 overlay 上 → 被 overlay 吞掉
    // （它的唯一副作用是关闭面板，不会触发任何页面 handler）。
    fireEvent.click(screen.getByTestId("palette-overlay"));

    // 50ms defer 窗口后关闭
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });

  // F3 回归：面板（z 1300）叠在 ConfirmDialog（z 1200）上时，一次 Esc
  // 只能关最顶层 —— ConfirmDialog 必须保持打开。
  it("Esc closes only the palette, not a ConfirmDialog beneath it (F3)", async () => {
    const onCancel = vi.fn();
    render(
      <JotaiProvider store={getDefaultStore()}>
        <MemoryRouter initialEntries={["/"]}>
          <CommandPalette />
          <ConfirmDialog
            open={true}
            title="Delete Profile"
            message="Delete this profile?"
            onConfirm={vi.fn()}
            onCancel={onCancel}
          />
        </MemoryRouter>
      </JotaiProvider>,
    );
    openPalette();

    fireEvent.keyDown(screen.getByLabelText("Command palette input"), {
      key: "Escape",
    });

    expect(screen.queryByRole("dialog", { name: "Command palette" })).not.toBeInTheDocument();
    // 下层的确认框没被这次 Esc 一起关掉
    expect(onCancel).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Delete Profile" })).toBeInTheDocument();
  });
});
