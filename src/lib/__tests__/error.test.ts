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

  /**
   * Issue #153: `probe_system_dns` 在非 macOS 返回 `Unsupported`。
   *
   * 关键断言是「不读成 invalid input」—— 平台限制和用户输入错误是两件事，
   * 一旦被归错类，将来任何把这个 message 显示给用户的代码都会告诉用户
   * 「你输入无效」，而他根本没有输入任何东西。
   */
  it("renders MhostError::Unsupported as a platform limitation, not invalid input", () => {
    const msg = extractErrorMessage({
      Unsupported: "system DNS probe is only supported on macOS",
    });
    expect(msg).toBe(
      "unsupported on this platform: system DNS probe is only supported on macOS",
    );
    expect(msg).not.toMatch(/invalid input/i);
  });

  /** Issue #153: 探测的读 OS 失败走 `Io { kind: "system-dns-probe" }`。 */
  it("renders the system-dns-probe Io kind", () => {
    const msg = extractErrorMessage({
      Io: {
        kind: "system-dns-probe",
        message: "failed to detect active network interface: route failed",
      },
    });
    expect(msg).toBe(
      "failed to detect active network interface: route failed (system-dns-probe)",
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