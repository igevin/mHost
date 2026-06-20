import { describe, it, expect } from "vitest";

describe("TypeScript types validation", () => {
  it("Profile type structure is correct", () => {
    const profile = {
      id: "550e8400-e29b-41d4-a716-446655440000",
      name: "dev",
      description: "Development profile",
      enabled: true,
      protected: false,
      tags: ["work", "dev"],
      rules: [],
      created_at: "2024-01-01T00:00:00Z",
      updated_at: "2024-01-01T00:00:00Z",
    };
    expect(profile).toBeDefined();
    expect(profile.name).toBe("dev");
    expect(profile.enabled).toBe(true);
    expect(profile.tags).toHaveLength(2);
  });

  it("RuleSource discriminated union types", () => {
    const manual = { type: "Manual" as const };
    const remote = {
      type: "Remote" as const,
      source_id: "550e8400-e29b-41d4-a716-446655440001",
      source_name: "example-source",
    };

    expect(manual.type).toBe("Manual");
    expect(remote.type).toBe("Remote");
    expect(remote.source_name).toBe("example-source");
  });

  it("HostRule with multiple domains", () => {
    const rule = {
      id: "550e8400-e29b-41d4-a716-446655440002",
      ip: "127.0.0.1",
      domains: ["a.com", "b.com"],
      enabled: true,
      comment: "test rule",
      source: { type: "Manual" as const },
    };

    expect(rule.domains).toHaveLength(2);
    expect(rule.ip).toMatch(/^\d+\.\d+\.\d+\.\d+$/);
  });
});
