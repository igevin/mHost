
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
   * **fix (issue #149 follow-up, c4eb339)**: when both `onPointerDown` AND
   * `onClick` route through one handler (Settings.tsx DNS toggle after
   * Cancel-button mount), the trailing synthetic `click` fires ~10ms after
   * pointerdown. Calling `release()` synchronously would let the click
   * handler pass the `fire()` guard a second time -> double toggle ->
   * AbortController slot overwritten, Cancel button ineffective on first
   * click.
   *
   * `releaseSoon()` defers the reset by POINTER_DOWN_DEBOUNCE_MS so the
   * trailing click is absorbed by the same `fire()` guard. Use this in
   * handlers that bind both `onPointerDown` and `onClick`; use plain
   * `release()` for pointerdown-only handlers where the click fallback
   * is not bound.
   *
   * **fix (issue #123)**: rename (Group 4 introduced `releaseSoon`
   * originally as `RESET_DELAY_MS` constant ref - master always had
   * `POINTER_DOWN_DEBOUNCE_MS` so we use that).
   */
  const releaseSoon = useCallback(() => {
    setTimeout(release, POINTER_DOWN_DEBOUNCE_MS);
  }, [release]);

  const onPointerDown = useCallback(
    (handler: (e: React.PointerEvent) => void) =>
      (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      if (!fire()) return;
      handler();
      // Pointerdown-only handler — onClick is NOT bound, so we don't need
      // the trailing-click protection `releaseSoon` provides.
      releaseSoon();
    },
    [fire, releaseSoon],
  );

  return { fire, release, releaseSoon, onPointerDown };
}
