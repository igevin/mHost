/**
 * Extract a human-readable message from a Tauri / Rust error.
 *
 * Tauri returns errors as plain objects (not Error instances) when
 * a Rust command returns `Err(...)`.  The object shape depends on
 * the serialization of the Rust error enum (`MhostError`).
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

    // MhostError::Io { kind, message }
    if (typeof obj.message === "string") {
      return obj.message;
    }

    // MhostError variants with a single string payload (Parse, Apply, Storage, InvalidInput)
    if (typeof obj.Parse === "string") return obj.Parse;
    if (typeof obj.Apply === "string") return obj.Apply;
    if (typeof obj.Storage === "string") return obj.Storage;
    if (typeof obj.InvalidInput === "string") return obj.InvalidInput;

    // Fallback: try to stringify the whole object
    try {
      return JSON.stringify(obj);
    } catch {
      // ignore
    }
  }

  return String(err);
}
