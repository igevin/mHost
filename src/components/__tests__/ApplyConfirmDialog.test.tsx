import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ApplyConfirmDialog from "../../components/ApplyConfirmDialog";

function Wrapper({ children }: { children: React.ReactNode }) {
  return <>{children}</>;
}

const mockPlan = {
  rules: [],
  conflicts: [],
  diff: {
    added: ["127.0.0.1 example.com"],
    removed: ["127.0.0.1 old.example.com"],
    unchanged: ["127.0.0.1 static.example.com"],
  },
  backup_required: true,
};

const mockPlanWithConflicts = {
  rules: [],
  conflicts: [
    {
      domain: "conflict.example.com",
      rules: [
        { ip: "127.0.0.1", domain: "conflict.example.com", source_profile_id: "p1", source_profile_name: "Profile A" },
        { ip: "127.0.0.2", domain: "conflict.example.com", source_profile_id: "p2", source_profile_name: "Profile B" },
      ],
    },
  ],
  diff: {
    added: ["127.0.0.1 conflict.example.com"],
    removed: [],
    unchanged: [],
  },
  backup_required: true,
};

const emptyPlan = {
  rules: [],
  conflicts: [],
  diff: {
    added: [],
    removed: [],
    unchanged: [],
  },
  backup_required: true,
};

describe("ApplyConfirmDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows progress during apply", async () => {
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          plan={null}
          onConfirm={vi.fn()}
          onCancel={vi.fn()}
          isApplying={true}
        />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText(/applying/i)).toBeInTheDocument();
    });
  });

  it("shows success after apply", async () => {
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          plan={null}
          onConfirm={vi.fn()}
          onCancel={vi.fn()}
          isApplying={false}
          applyResult="success"
        />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText(/success/i)).toBeInTheDocument();
    });
  });

  it("shows error and rollback button on failure", async () => {
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          plan={null}
          onConfirm={vi.fn()}
          onCancel={vi.fn()}
          isApplying={false}
          applyResult="error"
          applyError="Permission denied"
          onRollback={vi.fn()}
        />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(screen.getByText("Apply failed")).toBeInTheDocument();
      expect(screen.getByText("Permission denied")).toBeInTheDocument();
    });

    expect(
      screen.getByRole("button", { name: /rollback/i }),
    ).toBeInTheDocument();
  });

  it("renders preview state with diff", async () => {
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          plan={mockPlan}
          onConfirm={vi.fn()}
          onCancel={vi.fn()}
          isApplying={false}
        />
      </Wrapper>,
    );

    expect(screen.getByText("Confirm Changes")).toBeInTheDocument();
    expect(screen.getByText(/127\.0\.0\.1 example\.com/)).toBeInTheDocument();
    expect(screen.getByText(/127\.0\.0\.1 old\.example\.com/)).toBeInTheDocument();
    expect(screen.getByText(/A backup will be created before applying/)).toBeInTheDocument();
  });

  it("shows empty diff message when no changes", async () => {
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          plan={emptyPlan}
          onConfirm={vi.fn()}
          onCancel={vi.fn()}
          isApplying={false}
        />
      </Wrapper>,
    );

    expect(screen.getByText("No changes detected")).toBeInTheDocument();
  });

  it("disables confirm button when conflicts exist", async () => {
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          plan={mockPlanWithConflicts}
          onConfirm={vi.fn()}
          onCancel={vi.fn()}
          isApplying={false}
        />
      </Wrapper>,
    );

    expect(screen.getByText(/Warning: 1 conflict\(s\) detected/)).toBeInTheDocument();
    const confirmBtn = screen.getByRole("button", { name: /confirm apply/i });
    expect(confirmBtn).toBeDisabled();
  });

  it("triggers onConfirm when confirm button is clicked", async () => {
    const onConfirm = vi.fn();
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          plan={mockPlan}
          onConfirm={onConfirm}
          onCancel={vi.fn()}
          isApplying={false}
        />
      </Wrapper>,
    );

    const confirmBtn = screen.getByRole("button", { name: /confirm apply/i });
    await userEvent.click(confirmBtn);
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("triggers onCancel when cancel button is clicked", async () => {
    const onCancel = vi.fn();
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          plan={mockPlan}
          onConfirm={vi.fn()}
          onCancel={onCancel}
          isApplying={false}
        />
      </Wrapper>,
    );

    const cancelBtn = screen.getByRole("button", { name: /cancel/i });
    await userEvent.click(cancelBtn);
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("expands unchanged lines when clicked", async () => {
    render(
      <Wrapper>
        <ApplyConfirmDialog
          open={true}
          plan={mockPlan}
          onConfirm={vi.fn()}
          onCancel={vi.fn()}
          isApplying={false}
        />
      </Wrapper>,
    );

    const toggleBtn = screen.getByText(/...1 unchanged lines.../);
    await userEvent.click(toggleBtn);
    expect(screen.getByText(/static.example.com/)).toBeInTheDocument();
  });
});
