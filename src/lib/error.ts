/**
 * Extract a human-readable message from a Tauri / Rust error.
 *
 * Tauri returns errors as plain objects (not Error instances) when
 * a Rust command returns `Err(...)`.  The object shape depends on
 * the serialization of the Rust error enum (`MhostError`):
 *
 *   MhostError::Io { kind, message }      → { Io: { kind, message } }
 *   MhostError::InvalidInput(String)      → { InvalidInput: "…" }
 *   MhostError::Parse(ParseError)         → { Parse: { <variant>: <payload> } }
 *   MhostError::Apply(ApplyError)         → { Apply: { <variant>: <payload> } }
 *   MhostError::Storage(StorageError)     → { Storage: { <variant>: <payload> } }
 *
 * Inner enums (`ParseError`, `ApplyError`, `StorageError`) also serialize as
 * tagged objects, so we walk the shape and render `"<variant>: <payload>"`
 * strings instead of leaking the raw JSON envelope to the UI.
 */
export function extractErrorMessage(err: unknown): string {
  if (err instanceof Error) {
    return err.message;
  }

  if (typeof err === "string") {
    return err;
  }

  if (err && typeof err === "object") {
    const obj = err as Record<string, unknown>;

    // MhostError::Io { kind, message } — { Io: { kind, message } }
    if (isPlainObject(obj.Io)) {
      const ioObj = obj.Io as Record<string, unknown>;
      if (typeof ioObj.message === "string") {
        return typeof ioObj.kind === "string"
          ? `${ioObj.message} (${ioObj.kind})`
          : ioObj.message;
      }
    }

    // MhostError::InvalidInput(String)
    if (typeof obj.InvalidInput === "string") {
      return `invalid input: ${obj.InvalidInput}`;
    }

    // MhostError variants whose payload is a nested tagged enum.
    for (const outer of ["Parse", "Apply", "Storage"] as const) {
      const payload = obj[outer];
      if (isPlainObject(payload)) {
        const rendered = renderTaggedEnum(outer, payload);
        if (rendered) return rendered;
      }
    }

    // Fallback: never leak raw JSON to the UI. Use a generic phrase.
    try {
      return JSON.stringify(obj);
    } catch {
      // ignore
    }
  }

  return String(err);
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * Render a single-variant tagged enum (e.g. `{ ProfileNotFound: "uuid" }`).
 * Returns `null` if the payload has no recognizable shape.
 */
function renderTaggedEnum(
  outer: string,
  payload: Record<string, unknown>,
): string | null {
  const entries = Object.entries(payload);
  if (entries.length === 0) {
    return outer.toLowerCase();
  }
  const [variant, value] = entries[0]!;

  // Unit variant (e.g. HostsFileNotFound, ExternalModification) — payload is
  // absent or `null`.
  if (value === undefined || value === null) {
    return camelToPhrase(variant);
  }

  // Struct variant (e.g. VersionMismatch { expected, found }) — payload is
  // an object whose fields describe the mismatch.
  if (isPlainObject(value)) {
    const details = Object.entries(value)
      .map(([k, v]) => `${k}=${typeof v === "string" ? v : JSON.stringify(v)}`)
      .join(", ");
    return `${camelToPhrase(variant)} (${details})`;
  }

  // Tuple / newtype variant (e.g. ProfileNotFound(<uuid>)) — payload is a
  // primitive (most commonly a string).
  return `${camelToPhrase(variant)}: ${String(value)}`;
}

/**
 * Convert a CamelCase variant name to a lower-case phrase.
 *   "ProfileNotFound" → "profile not found"
 *   "VersionMismatch" → "version mismatch"
 */
function camelToPhrase(s: string): string {
  return s
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
    .toLowerCase();
}