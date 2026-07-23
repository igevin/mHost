import { describe, it, expect } from "vitest";
import type { ApplyOutcome } from "../../types";
import { decideApplyMode, DESTRUCTIVE_THRESHOLD } from "../applyPolicy";

function outcomeWith(overrides: Partial<ApplyOutcome>): ApplyOutcome {
  return {
    plan: {
      rules: [],
      conflicts: [],
      diff: { added: [], removed: [], unchanged: [] },
      backup_required: false,
    },
    added_count: 0,
    removed_count: 0,
    unchanged_count: 0,
    disabled_profile_ids: [],
    has_conflicts: false,
    snapshot_id: null,
    backup_path: null,
    ...overrides,
  };
}

describe("decideApplyMode (Refs #127)", () => {
  it("conflicts -> require_preview", () => {
    expect(
      decideApplyMode(outcomeWith({ has_conflicts: true })),
    ).toBe("require_preview");
  });

  it("non-empty disabled_profile_ids -> require_preview", () => {
    expect(
      decideApplyMode(outcomeWith({ disabled_profile_ids: ["x"] })),
    ).toBe("require_preview");
  });

  it("above DESTRUCTIVE_THRESHOLD total changes -> require_preview", () => {
    expect(
      decideApplyMode(
        outcomeWith({
          added_count: DESTRUCTIVE_THRESHOLD + 1,
          removed_count: 0,
        }),
      ),
    ).toBe("require_preview");
  });

  it("at-threshold total changes -> quick_apply", () => {
    expect(
      decideApplyMode(
        outcomeWith({
          added_count: DESTRUCTIVE_THRESHOLD,
          removed_count: 0,
        }),
      ),
    ).toBe("quick_apply");
  });

  it("zero changes -> quick_apply", () => {
    expect(decideApplyMode(outcomeWith({}))).toBe("quick_apply");
  });

  it("combined rules: conflicts gate wins first", () => {
    // has_conflicts=true AND disabled AND 5 changes -> conflicts wins
    expect(
      decideApplyMode(
        outcomeWith({
          has_conflicts: true,
          disabled_profile_ids: ["x"],
          added_count: 5,
        }),
      ),
    ).toBe("require_preview");
  });

  it("combined rules: disabled gate wins before threshold", () => {
    // no conflicts, disabled AND 5 changes -> disabled gate
    expect(
      decideApplyMode(
        outcomeWith({
          disabled_profile_ids: ["x"],
          added_count: 5,
        }),
      ),
    ).toBe("require_preview");
  });

  it("split added+removed above threshold still triggers", () => {
    expect(
      decideApplyMode(
        outcomeWith({
          added_count: 60,
          removed_count: 41, // total 101 > 100
        }),
      ),
    ).toBe("require_preview");
  });
});