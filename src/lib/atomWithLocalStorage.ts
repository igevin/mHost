import { atom } from "jotai";

/**
 * Persistent jotai atom backed by `localStorage`.
 *
 * Used for client-side user preferences that don't have a backend
 * representation (e.g. issue #123's Quick Apply toggle). Mirrors
 * `jotai/utils`'s `atomWithStorage` API surface but keeps the dep
 * footprint minimal — we only need a handful of primitive settings,
 * not a full storage abstraction.
 *
 * Behavior:
 * - The initial value is read from localStorage on atom creation so
 *   the in-memory state matches the last persisted value.
 * - Reads/writes are JSON-serialized. Only JSON-safe values (booleans,
 *   numbers, strings, arrays, plain objects) round-trip cleanly.
 * - Write failures (quota exceeded, private-mode SecurityError) are
 *   swallowed — the in-memory state still updates and persistence is
 *   best-effort. We don't want a write failure to crash the UI.
 * - No cross-tab `storage` event subscription. mHost is a single
 *   user-facing desktop window; the one-tab assumption is intentional.
 */
export function atomWithLocalStorage<T>(key: string, initial: T) {
  const readInitial = (): T => {
    if (typeof localStorage === "undefined") return initial;
    try {
      const raw = localStorage.getItem(key);
      if (raw === null) return initial;
      return JSON.parse(raw) as T;
    } catch {
      // Corrupted value — fall back to the default rather than crashing.
      return initial;
    }
  };

  const baseAtom = atom(readInitial());

  return atom(
    (get) => get(baseAtom),
    (get, set, update: T | ((prev: T) => T)) => {
      const next =
        typeof update === "function"
          ? (update as (prev: T) => T)(get(baseAtom))
          : update;
      set(baseAtom, next);
      if (typeof localStorage !== "undefined") {
        try {
          localStorage.setItem(key, JSON.stringify(next));
        } catch {
          // Quota exceeded / SecurityError — best-effort persistence.
        }
      }
    },
  );
}
