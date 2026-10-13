
import { useRef, useCallback } from "react";

/**
 * Public alias for the pointerdown↔click debounce window. Re-exported so
 * consumers that drive `fire()` themselves (e.g. sync toggles) can share
 * the same value the wrapper uses internally.
 */
export const POINTER_DOWN_DEBOUNCE_MS = 50;

/** Retarget-guard window: the gesture's compat `click` arrives within a few
 * ms of pointerdown; the listener only lives this long in case it never
 * comes (engine suppressed it via preventDefault). Kept well under human
 * double-click speed so a fast intentional re-click at the same spot (e.g.
 * re-clicking the ⋯ area to close a freshly opened PopupMenu) is not
 * swallowed. */
const SWALLOW_WINDOW_MS = 100;

/** Position tolerance (px) for matching the trailing click to its gesture. */
const SWALLOW_POS_TOLERANCE = 4;

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
 *   Precondition: the handler must tolerate double invocation — jsdom
 *   `fireEvent` dispatches pointerdown and click independently, and both
 *   reach the handler (issue #100: a double-bound Delete once fired two
 *   confirms and a `ProfileNotFound`). Acceptable when the handler is
 *   idempotent, or the button disables/unmounts synchronously on the first
 *   call (e.g. a dialog opens over it). Anything else belongs to pattern B.
 *
 *   The wrapper also protects the gesture's trailing `click`: if the
 *   handler mutates the layout under the cursor (e.g. "Edit Rules" swaps
 *   the header to [Cancel][Save]), the compat click is hit-tested against
 *   the NEW layout and would activate whatever replaced the button. The
 *   wrapper swallows a click only when it lands at the same coordinates on
 *   a DIFFERENT target; preventDefault additionally cancels compat events
 *   outright in spec-compliant engines.
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
        e.preventDefault();
        if (!fire()) return;
        handler(e);
        // Pattern A 里 onClick 也绑了同一个 handler —— releaseSoon 把
        // wrapper 自己的 guard 留到 trailing click 之后再复位，免得同一
        // 手势的下一次 pointerdown 被误判成连点。注意这只复位 wrapper
        // 实例；onClick 路径不经过这里，handler 自己要能容忍双触发
        // （见上方 pattern A 的前提条件）。
        releaseSoon();
        // Retarget guard (PR #244 follow-up, hosts profile "无法编辑"):
        // the pointerdown handler may synchronously mutate the layout under
        // the cursor — e.g. "Edit Rules" swapping the header to
        // [Cancel][Save]. The gesture's trailing `click` is then hit-tested
        // against the NEW layout and lands on whatever replaced the button
        // (Cancel instantly exited edit mode). Swallow exactly that click:
        // same coordinates, different target. Two caveats:
        // - React REUSES DOM nodes for same-type unkeyed siblings, so the
        //   swapped-in button can literally be the origin node patched in
        //   place — swapped button groups need `key` props for this guard
        //   to see a distinct target (see HostsProfileView/DnsProfileView
        //   headers).
        // - Clicks on the original element (normal double-fire; keyboard
        //   activation targets the element too) and clicks anywhere else
        //   pass through untouched. The listener is one-shot and
        //   self-removes after SWALLOW_WINDOW_MS in case the engine
        //   suppressed the compat click (preventDefault above) or the
        //   gesture ends without one.
        const native = e.nativeEvent;
        const origin = native.target;
        const onDocClick = (ev: MouseEvent) => {
          document.removeEventListener("click", onDocClick, true);
          if (ev.target === origin) return;
          if (origin instanceof Node && ev.target instanceof Node && origin.contains(ev.target)) return;
          if (
            Math.abs(ev.clientX - native.clientX) > SWALLOW_POS_TOLERANCE ||
            Math.abs(ev.clientY - native.clientY) > SWALLOW_POS_TOLERANCE
          ) {
            return;
          }
          ev.preventDefault();
          ev.stopPropagation();
        };
        document.addEventListener("click", onDocClick, true);
        setTimeout(
          () => document.removeEventListener("click", onDocClick, true),
          SWALLOW_WINDOW_MS,
        );
      },
    [fire, releaseSoon],
  );

  return { fire, release, releaseSoon, onPointerDown };
}
