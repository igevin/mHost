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

  const onPointerDown = useCallback(
    (handler: () => void) => (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      if (!fire()) return;
      handler();
      setTimeout(release, RESET_DELAY_MS);
    },
    [fire, release],
  );

  return { fire, release, onPointerDown };
}
