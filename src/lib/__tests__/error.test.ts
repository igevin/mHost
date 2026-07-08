import { describe, it, expect } from "vitest";
import { extractErrorMessage } from "../error";

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
});