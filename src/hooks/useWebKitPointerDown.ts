
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
 * Two supported call-site patterns — pick exactly one per button:
 *
 * **A. Wrapper (handler does NOT own a guard).** For plain buttons whose
 * handler has no `fire()` call of its own:
 *
 *   const { onPointerDown } = useWebKitPointerDown();
 *   <button onClick={handleSave} onPointerDown={onPointerDown(handleSave)} />
 *
 * **B. Handler-owned guard (both events feed one handler).** For toggles
 * and async actions that must dedupe the pointerdown+click pair across a
 * longer window (issue #123/#149 legacy): the handler calls `fire()` +
 * `releaseSoon()` itself, and `onPointerDown` is bound with only an inline
 * `e.button !== 0` filter — NEVER through the wrapper, which would
 * double-consume `firedRef` and drop every gesture on the floor:
 *
 *   const { fire, releaseSoon } = useWebKitPointerDown();
 *   const handleToggle = () => {
 *     if (!fire()) return;
 *     releaseSoon();
 *     doWork();
 *   };
 *   <button
 *     onClick={handleToggle}
 *     onPointerDown={(e) => { if (e.button !== 0) return; handleToggle(); }}
 *   />
 *
 * Use a separate hook instance per independent action (e.g. SnapshotPanel's
 * save/rollback/delete guards) so one action's latch can't block another's.
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
      handler(e);
      // Pointerdown-only handler — onClick is NOT bound, so we don't need
      // the trailing-click protection `releaseSoon` provides.
      releaseSoon();
    },
    [fire, releaseSoon],
  );

  return { fire, release, releaseSoon, onPointerDown };
}
