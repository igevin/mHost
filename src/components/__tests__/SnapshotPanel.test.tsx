import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import { getDefaultStore, Provider as JotaiProvider } from "jotai";
import {
  snapshotsAtom,
  isLoadingSnapshotsAtom,
  snapshotErrorAtom,
} from "../../stores/profiles";
import type { SnapshotMeta } from "../../types";

// Mock tauri dialog
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn().mockResolvedValue(true),
}));

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue([]),
}));

import SnapshotPanel from "../SnapshotPanel";

function makeSnapshot(overrides: Partial<SnapshotMeta> = {}): SnapshotMeta {
  return {
    id: "snap-1",
    name: "test-snapshot",
    description: "A test snapshot",
    profile_count: 2,
    created_at: "2024-01-15T10:30:00Z",
    ...overrides,
  };
}

function renderWithProviders(ui: React.ReactElement) {
  return render(<JotaiProvider store={getDefaultStore()}>{ui}</JotaiProvider>);
}

describe("SnapshotPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const store = getDefaultStore();
    store.set(snapshotsAtom, []);
    store.set(isLoadingSnapshotsAtom, false);
    store.set(snapshotErrorAtom, null);
  });

  // 1. 渲染空状态
  it("renders empty state when no snapshots", async () => {
    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={false}
        onCloseCreateDialog={vi.fn()}
        onCreateSnapshot={vi.fn().mockResolvedValue(undefined)}
      />,
    );
    // Wait for async fetchSnapshots to complete
    await waitFor(() =>
      expect(screen.getByText("No snapshots yet.")).toBeInTheDocument(),
    );
    expect(
      screen.getByText("Create a snapshot to save your current configuration."),
    ).toBeInTheDocument();
  });

  // 2. 渲染快照列表
  it("renders snapshot list with meta info", () => {
    const store = getDefaultStore();
    store.set(snapshotsAtom, [
      makeSnapshot({ id: "snap-1", name: "dev-backup", profile_count: 3 }),
      makeSnapshot({ id: "snap-2", name: "prod-backup", profile_count: 1 }),
    ]);

    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={false}
        onCloseCreateDialog={vi.fn()}
        onCreateSnapshot={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    expect(screen.getByText("dev-backup")).toBeInTheDocument();
    expect(screen.getByText("prod-backup")).toBeInTheDocument();
    expect(screen.getByText("3 profiles")).toBeInTheDocument();
    expect(screen.getByText("1 profile")).toBeInTheDocument();
  });

  // 3. 创建快照对话框提交
  it("calls onCreateSnapshot with name and description on Save", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={true}
        onCloseCreateDialog={vi.fn()}
        onCreateSnapshot={onCreate}
      />,
    );

    const nameInput = screen.getByPlaceholderText("Snapshot name");
    const descInput = screen.getByPlaceholderText("Optional description");

    await act(async () => {
      fireEvent.change(nameInput, { target: { value: "my-snapshot" } });
    });
    await act(async () => {
      fireEvent.change(descInput, { target: { value: "My desc" } });
    });

    const saveButton = screen.getByText("Save");
    await act(async () => {
      fireEvent.click(saveButton);
    });

    await waitFor(() => expect(onCreate).toHaveBeenCalledTimes(1));
    expect(onCreate).toHaveBeenCalledWith("my-snapshot", "My desc");
  });

  // 4. 创建快照时不允许空名称
  it("does not call onCreateSnapshot with empty name", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={true}
        onCloseCreateDialog={vi.fn()}
        onCreateSnapshot={onCreate}
      />,
    );

    const saveButton = screen.getByText("Save");
    expect(saveButton).toBeDisabled();

    await act(async () => {
      fireEvent.click(saveButton);
    });
    expect(onCreate).not.toHaveBeenCalled();
  });

  // 5. 回滚确认对话框
  it("calls confirm dialog on Rollback click", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    const store = getDefaultStore();
    store.set(snapshotsAtom, [makeSnapshot({ id: "snap-1", name: "rollback-me" })]);

    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={false}
        onCloseCreateDialog={vi.fn()}
        onCreateSnapshot={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    const rollbackButton = screen.getByText("Rollback");
    await act(async () => {
      fireEvent.click(rollbackButton);
    });

    await waitFor(() =>
      expect(confirm).toHaveBeenCalledWith(
        'Rollback to snapshot "rollback-me"? Current configuration will be overwritten.',
      ),
    );
  });

  // 6. 删除确认对话框
  it("calls confirm dialog on Delete click", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    const store = getDefaultStore();
    store.set(snapshotsAtom, [makeSnapshot({ id: "snap-1", name: "delete-me" })]);

    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={false}
        onCloseCreateDialog={vi.fn()}
        onCreateSnapshot={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    const deleteButton = screen.getByText("Delete");
    await act(async () => {
      fireEvent.click(deleteButton);
    });

    await waitFor(() =>
      expect(confirm).toHaveBeenCalledWith('Delete snapshot "delete-me"?'),
    );
  });

  // 7. 支持 Enter 键提交
  it("triggers save on Enter key", async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={true}
        onCloseCreateDialog={vi.fn()}
        onCreateSnapshot={onCreate}
      />,
    );

    const nameInput = screen.getByPlaceholderText("Snapshot name");
    await act(async () => {
      fireEvent.change(nameInput, { target: { value: "enter-test" } });
    });
    await act(async () => {
      fireEvent.keyDown(nameInput, { key: "Enter", code: "Enter" });
    });

    await waitFor(() => expect(onCreate).toHaveBeenCalledTimes(1));
  });

  // 8. 支持 Escape 键关闭对话框
  it("closes dialog on Escape key", async () => {
    const onClose = vi.fn();
    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={true}
        onCloseCreateDialog={onClose}
        onCreateSnapshot={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    const nameInput = screen.getByPlaceholderText("Snapshot name");
    await act(async () => {
      fireEvent.keyDown(nameInput, { key: "Escape", code: "Escape" });
    });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // 9. 保存时禁用遮罩关闭
  it("disables overlay close while saving", async () => {
    const onClose = vi.fn();
    const onCreate = vi.fn().mockImplementation(
      () => new Promise((resolve) => setTimeout(resolve, 200)),
    );
    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={true}
        onCloseCreateDialog={onClose}
        onCreateSnapshot={onCreate}
      />,
    );

    const nameInput = screen.getByPlaceholderText("Snapshot name");
    await act(async () => {
      fireEvent.change(nameInput, { target: { value: "saving-test" } });
    });

    const saveButton = screen.getByText("Save");
    await act(async () => {
      fireEvent.click(saveButton);
    });

    // Overlay should not trigger onClose while saving
    const overlay = document.querySelector("[class*='dialogOverlay']");
    expect(overlay).toBeTruthy();
    if (overlay) {
      fireEvent.click(overlay);
      expect(onClose).not.toHaveBeenCalled();
    }
  });

  // 10. 错误提示渲染
  it("renders error message from store", async () => {
    renderWithProviders(
      <SnapshotPanel
        showCreateDialog={false}
        onCloseCreateDialog={vi.fn()}
        onCreateSnapshot={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    // Wait for fetch to complete first
    await waitFor(() =>
      expect(screen.queryByText("Loading snapshots...")).not.toBeInTheDocument(),
    );

    // Then set error state
    const store = getDefaultStore();
    store.set(snapshotErrorAtom, "Failed to load snapshots");

    await waitFor(() =>
      expect(screen.getByText("Failed to load snapshots")).toBeInTheDocument(),
    );
  });
});
