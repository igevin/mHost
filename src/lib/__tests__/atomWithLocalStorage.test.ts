import { describe, it, expect, beforeEach, vi } from "vitest";
import { getDefaultStore } from "jotai";
import { atomWithLocalStorage } from "../atomWithLocalStorage";

describe("atomWithLocalStorage (issue #123)", () => {
  // A minimal in-memory localStorage shim. Real browsers implement
  // more (Storage events, iterators), but our helper only reads /
  // writes by key, so this is enough to exercise every branch.
  let mockStorage: Record<string, string>;
  let store: ReturnType<typeof getDefaultStore>;

  beforeEach(() => {
    store = getDefaultStore();
    mockStorage = {};
    globalThis.localStorage = {
      getItem: (k: string) => (k in mockStorage ? mockStorage[k] : null),
      setItem: (k: string, v: string) => {
        mockStorage[k] = v;
      },
      removeItem: (k: string) => {
        delete mockStorage[k];
      },
      clear: () => {
        for (const k of Object.keys(mockStorage)) delete mockStorage[k];
      },
      key: () => null,
      length: 0,
    } as Storage;
  });

  it("uses the initial value when localStorage is empty", () => {
    const a = atomWithLocalStorage<boolean>("mhost.flag", false);
    expect(store.get(a)).toBe(false);
  });

  it("reads the stored value on creation", () => {
    mockStorage["mhost.flag"] = JSON.stringify(true);
    const a = atomWithLocalStorage<boolean>("mhost.flag", false);
    expect(store.get(a)).toBe(true);
  });

  it("falls back to the initial value when the stored JSON is corrupt", () => {
    mockStorage["mhost.flag"] = "{not json";
    const a = atomWithLocalStorage<boolean>("mhost.flag", false);
    expect(store.get(a)).toBe(false);
  });

  it("writes the JSON-serialized value to localStorage on update", () => {
    const a = atomWithLocalStorage<boolean>("mhost.flag", false);
    store.set(a, true);
    expect(mockStorage["mhost.flag"]).toBe(JSON.stringify(true));
  });

  it("supports functional updates", () => {
    const a = atomWithLocalStorage<number>("counter", 0);
    store.set(a, (n) => n + 5);
    expect(store.get(a)).toBe(5);
    store.set(a, (n) => n * 2);
    expect(store.get(a)).toBe(10);
    // Persistence also keeps up with the functional update.
    expect(mockStorage["counter"]).toBe(JSON.stringify(10));
  });

  it("does not throw when localStorage.setItem raises (e.g. quota)", () => {
    const a = atomWithLocalStorage<boolean>("mhost.flag", false);
    const setSpy = vi
      .spyOn(Storage.prototype, "setItem")
      .mockImplementation(() => {
        throw new Error("quota exceeded");
      });
    try {
      expect(() => store.set(a, true)).not.toThrow();
      // The in-memory state still updates even when persistence fails —
      // this is the explicit "best-effort" contract.
      expect(store.get(a)).toBe(true);
    } finally {
      setSpy.mockRestore();
    }
  });

  it("survives when localStorage is undefined (SSR / Node)", () => {
    // Simulate jsdom / Node: no localStorage on globalThis.
    const original = globalThis.localStorage;
    // @ts-expect-error — intentionally clearing for the test.
    delete globalThis.localStorage;
    try {
      const a = atomWithLocalStorage<boolean>("mhost.flag", false);
      expect(store.get(a)).toBe(false);
      // Writes are no-ops; in-memory state still changes.
      store.set(a, true);
      expect(store.get(a)).toBe(true);
    } finally {
      globalThis.localStorage = original;
    }
  });
});
