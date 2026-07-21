
import { useRef, useCallback } from "react";

/**
 * Public alias for the pointerdown↔click debounce window. Re-exported so
 * consumers that drive `fire()` themselves (e.g. sync toggles) can share
 * the same value the wrapper uses internally.
 */
export const POINTER_DOWN_DEBOUNCE_MS = 50;

/**
 * Workaround for a WebKit/Tauri WebView quirk where the first click on a button
 * after typing in an input field is swallowed during the input→button focus
 * transfer. Bind `onPointerDown` alongside `onClick`; the pointerdown event
 * fires reliably and the click fallback ensures accessibility/testing compatibility.
 *
 * The wrapped handler receives the original `PointerEvent` so callers can
 * read modifiers (`metaKey`, `altKey`) — e.g. issue #123's Quick Apply
 * override that holds Cmd/Option to force the Preview dialog.
 *
 * Usage:
 *   const { fire, release, onPointerDown } = useWebKitPointerDown();
 *
 *   <button onPointerDown={onPointerDown((e) => handleCancel(e.metaKey))} />
 *
 *   // Sync toggles guarded from double-fire (issue #123). Both `onClick`
 *   // and `onPointerDown` feed `handleToggle()`, which calls `fire()` +
 *   // `releaseSoon()` itself. The wrapper is **not** used here — that
 *   // would double-consume `firedRef` (the wrapper calls `fire()` before
 *   // dispatching the handler) and drop every gesture on the floor.
 *   const handleToggle = () => {
 *     if (!fire()) return;
 *     releaseSoon();
 *     doWork();
 *   };
 */
export function useWebKitPointerDown() {
  const firedRef = useRef(false);

  const fire = useCallback(() => {
    if (firedRef.current) return false;
    firedRef.current = true;
    return true;
  }, []);

  const release = useCallback(() => {
    firedRef.current = false;
  }, []);

  /**
   * Schedule `release()` after POINTER_DOWN_DEBOUNCE_MS. Mirrors the
   * setTimeout the wrapper uses internally so callers driving `fire()`
   * from their own click/pointerdown pair stay in sync with the wrapper.
   */
  const releaseSoon = useCallback(() => {
    setTimeout(release, POINTER_DOWN_DEBOUNCE_MS);
  }, [release]);

  const onPointerDown = useCallback(
    (handler: (e: React.PointerEvent) => void) =>
      (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      if (!fire()) return;
      handler(e);
      setTimeout(release, POINTER_DOWN_DEBOUNCE_MS);
    },
    [fire, release],
  );

  return { fire, release, releaseSoon, onPointerDown };
}
