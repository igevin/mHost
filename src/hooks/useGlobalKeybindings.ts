import { useEffect } from "react";
import { useSetAtom } from "jotai";
import { paletteOpenAtom } from "../stores/profiles";

interface Keybinding {
  /** e.g. "k" — compared against `KeyboardEvent.key` lowercased. */
  key: string;
  /** Cmd on macOS / Ctrl elsewhere. */
  mod: boolean;
  run: () => void;
}

/**
 * Global keyboard shortcuts, registered once at the Layout level
 * (issue #124). Currently: Cmd+K / Ctrl+K toggles the command palette.
 *
 * Structure note: bindings live in a table so future shortcuts are one
 * row each. The palette's own Esc / ↑↓ / Enter handling lives inside
 * CommandPalette — this hook only owns the global toggle.
 */
export function useGlobalKeybindings() {
  const setPaletteOpen = useSetAtom(paletteOpenAtom);

  useEffect(() => {
    const bindings: Keybinding[] = [
      {
        key: "k",
        mod: true,
        // Toggle: Cmd+K while the palette is open closes it (standard
        // palette UX) — this is not "hijacking" the palette's own keys;
        // Esc/Enter/↑↓ never reach this table.
        run: () => setPaletteOpen((prev) => !prev),
      },
    ];

    const handleKeyDown = (e: KeyboardEvent) => {
      for (const b of bindings) {
        const modPressed = e.metaKey || e.ctrlKey;
        if (b.mod === modPressed && e.key.toLowerCase() === b.key) {
          e.preventDefault();
          b.run();
          return;
        }
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [setPaletteOpen]);
}
