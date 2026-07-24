import { describe, it, expect } from "vitest";
import { extractErrorMessage, isPreviewRequired } from "../error";

describe("extractErrorMessage", () => {
  it("returns string verbatim", () => {
    expect(extractErrorMessage("oops")).toBe("oops");
  });

  it("returns Error.message", () => {
    expect(extractErrorMessage(new Error("boom"))).toBe("boom");
  });

  it("unwraps MhostError::Io { kind, message }", () => {
    // message is preserved; kind is appended in parentheses for context.
    expect(
      extractErrorMessage({ Io: { kind: "NotFound", message: "missing" } }),
    ).toBe("missing (NotFound)");
  });

  it("returns string-payload InvalidInput variant", () => {
    expect(extractErrorMessage({ InvalidInput: "bad args" })).toBe(
      "invalid input: bad args",
    );
  });

  it("returns string-payload Network variant (no raw JSON)", () => {
    const result = extractErrorMessage({ Network: "connection refused" });
    expect(result).not.toContain("{");
    expect(result).toBe("network error: connection refused");
  });

  it("returns string-payload ExternalApi variant (no raw JSON)", () => {
    const result = extractErrorMessage({ ExternalApi: "GitHub API error: 403" });
    expect(result).not.toContain("{");
    expect(result).toBe("external API error: GitHub API error: 403");
  });

  it("returns human message for ProfileNotFound (no raw JSON)", () => {
    // Regression guard for issue #100: the user used to see the raw JSON
    // envelope. The output must never contain `{` and must surface the
    // human-readable phrase plus the missing id.
    const result = extractErrorMessage({
      Storage: { ProfileNotFound: "848b140b-ec4f-4d2b-baff-66d48ec12fce" },
    });
    expect(result).not.toContain("{");
    expect(result).toContain("profile not found");
    expect(result).toContain("848b140b-ec4f-4d2b-baff-66d48ec12fce");
  });

  it("returns human message for VersionMismatch (no raw JSON)", () => {
    const result = extractErrorMessage({
      Storage: { VersionMismatch: { expected: 2, found: 1 } },
    });
    expect(result).not.toContain("{");
    expect(result).toContain("version mismatch");
    expect(result).toContain("expected=2");
    expect(result).toContain("found=1");
  });

  it("returns human message for ParseError (no raw JSON)", () => {
    const result = extractErrorMessage({
      Parse: { InvalidIp: "999.999.999.999" },
    });
    expect(result).not.toContain("{");
    expect(result).toContain("invalid ip");
    expect(result).toContain("999.999.999.999");
  });

  it("returns human message for ApplyError (no raw JSON)", () => {
    const result = extractErrorMessage({
      Apply: { PermissionDenied: "no sudo" },
    });
    expect(result).not.toContain("{");
    expect(result).toContain("permission denied");
    expect(result).toContain("no sudo");
  });

  it("returns human message for unit-variant ApplyError (no raw JSON)", () => {
    // ApplyError::HostsFileNotFound has no payload.
    const result = extractErrorMessage({ Apply: { HostsFileNotFound: null } });
    expect(result).not.toContain("{");
    expect(result).toContain("hosts file not found");
  });

  it("returns string-payload PreviewRequired variant (no raw JSON)", () => {
    const result = extractErrorMessage({ PreviewRequired: "conflicts detected" });
    expect(result).not.toContain("{");
    expect(result).toBe("preview required: conflicts detected");
  });
});

describe("isPreviewRequired (Refs #127)", () => {
  it("true for a { PreviewRequired: string } envelope", () => {
    expect(isPreviewRequired({ PreviewRequired: "conflicts detected" })).toBe(true);
  });

  it("false for other MhostError shapes", () => {
    expect(isPreviewRequired({ InvalidInput: "bad" })).toBe(false);
    expect(isPreviewRequired({ Io: { kind: "NotFound", message: "x" } })).toBe(false);
    expect(isPreviewRequired({ Apply: { PermissionDenied: "no sudo" } })).toBe(false);
  });

  it("false for non-object / non-string-payload values", () => {
    expect(isPreviewRequired(null)).toBe(false);
    expect(isPreviewRequired("PreviewRequired")).toBe(false);
    expect(isPreviewRequired(new Error("boom"))).toBe(false);
    expect(isPreviewRequired({ PreviewRequired: 123 })).toBe(false);
  });
});