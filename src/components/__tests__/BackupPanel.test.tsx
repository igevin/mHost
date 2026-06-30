import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mock tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const mockInvoke = vi.mocked(invoke);

// Import component after mock setup
import BackupPanel from "../../components/BackupPanel";

function Wrapper({ children }: { children: React.ReactNode }) {
  return <>{children}</>;
}

function makeBackup(overrides: Partial<{
  id: string;
  filename: string;
  timestamp: string;
  size: number;
  path: string;
}> = {}) {
  return {
    id: "hosts-20240615_103000.bak",
    filename: "hosts-20240615_103000.bak",
    timestamp: "2024-06-15T10:30:00Z",
    size: 1024,
    path: "/tmp/backups/hosts-20240615_103000.bak",
    ...overrides,
  };
}

describe("BackupPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows empty state when no backups", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_backups") return [];
      return null;
    });

    render(
      <Wrapper>
        <BackupPanel />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText(/no backups yet/i)).toBeInTheDocument();
    });
  });

  it("renders backup list", async () => {
    const backups = [
      makeBackup({ id: "b1", filename: "hosts-20240615_103000.bak", size: 1024 }),
      makeBackup({ id: "b2", filename: "hosts-20240616_080000.bak", size: 2048 }),
    ];

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_backups") return backups;
      return null;
    });

    render(
      <Wrapper>
        <BackupPanel />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("hosts-20240615_103000.bak")).toBeInTheDocument();
    });

    expect(screen.getByText("hosts-20240616_080000.bak")).toBeInTheDocument();
    expect(screen.getByText("1 KB")).toBeInTheDocument();
    expect(screen.getByText("2 KB")).toBeInTheDocument();
  });

  it("opens confirmation dialog on rollback click", async () => {
    const user = userEvent.setup();
    const backups = [makeBackup()];

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_backups") return backups;
      return null;
    });

    render(
      <Wrapper>
        <BackupPanel />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("hosts-20240615_103000.bak")).toBeInTheDocument();
    });

    const rollbackBtn = screen.getByRole("button", { name: /rollback/i });
    await user.click(rollbackBtn);

    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
      expect(screen.getByText(/rollback to backup from/i)).toBeInTheDocument();
    });
  });

  it("calls rollbackToBackup on confirm and refreshes list", async () => {
    const user = userEvent.setup();
    const onRollback = vi.fn();
    const backups = [makeBackup()];

    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "list_backups") return backups;
      if (cmd === "rollback_to_backup" && (args as { id: string }).id === "hosts-20240615_103000.bak") {
        return null;
      }
      return null;
    });

    render(
      <Wrapper>
        <BackupPanel onRollback={onRollback} />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("hosts-20240615_103000.bak")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /rollback/i }));

    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /confirm/i }));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("rollback_to_backup", { id: "hosts-20240615_103000.bak" });
    });

    expect(onRollback).toHaveBeenCalledTimes(1);
  });

  it("shows error when rollback fails", async () => {
    const user = userEvent.setup();
    const backups = [makeBackup()];

    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "list_backups") return backups;
      if (cmd === "rollback_to_backup") {
        throw new Error("Backup not found");
      }
      return null;
    });

    render(
      <Wrapper>
        <BackupPanel />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("hosts-20240615_103000.bak")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /rollback/i }));

    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /confirm/i }));

    await waitFor(() => {
      expect(screen.getByText(/backup not found/i)).toBeInTheDocument();
    });
  });
});
