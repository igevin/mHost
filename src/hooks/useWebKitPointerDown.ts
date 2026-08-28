import { useRef, useCallback } from "react";

const RESET_DELAY_MS = 50;

/**
 * Workaround for a WebKit/Tauri WebView quirk where the first click on a button
 * after typing in an input field is swallowed during the input→button focus
 * transfer. Bind `onPointerDown` alongside `onClick`; the pointerdown event
 * fires reliably and the click fallback ensures accessibility/testing compatibility.
 *
 * Usage:
 *   const { fire, release, onPointerDown } = useWebKitPointerDown();
 *
 *   // Sync handler (button onPointerDown)
 *   <button onPointerDown={onPointerDown(handleCancel)} />
 *
 *   // Async handler – guard with fire() and release() in finally
 *   const handleCreate = async () => {
 *     if (!fire()) return;
 *     try { ... } finally { release(); }
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

  // **issue #149 follow-up (c4eb339)**: when both `onPointerDown` AND `onClick`
  // route through one handler (Settings.tsx DNS toggle after Cancel-button
  // mount), the trailing synthetic `click` fires ~10ms after pointerdown.
  // Calling `release()` synchronously would let the click handler pass the
  // `fire()` guard a second time → double toggle → AbortController slot
  // overwritten, Cancel button ineffective on the first click.
  //
  // `releaseSoon()` defers the reset by `RESET_DELAY_MS` so the trailing
  // click is absorbed by the same `fire()` guard. Use this in handlers that
  // bind both `onPointerDown` and `onClick`; use plain `release()` for
  // pointerdown-only handlers where the click fallback is not bound.
  const releaseSoon = useCallback(() => {
    setTimeout(release, RESET_DELAY_MS);
  }, [release]);

  const onPointerDown = useCallback(
    (handler: () => void) => (e: React.PointerEvent) => {
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
