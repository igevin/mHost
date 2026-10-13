import { useEffect } from "react";
import { useSetAtom } from "jotai";
import { paletteOpenAtom } from "../stores/profiles";

interface Keybinding {
  /** e.g. "k" — compared against `KeyboardEvent.key` lowercased. */
  key: string;
  /** Platform primary modifier: Cmd on macOS / Ctrl elsewhere (issue #124). */
  mod: boolean;
  run: () => void;
}

/**
 * Global keyboard shortcuts, registered once at the Layout level
 * (issue #124). Currently: Cmd+K (macOS) / Ctrl+K (elsewhere) toggles the
 * command palette.
 *
 * Structure note: bindings live in a table so future shortcuts are one
 * row each. The palette's own Esc / ↑↓ / Enter handling lives inside
 * CommandPalette — this hook only owns the global toggle.
 *
 * review F7 gating rules:
 * - macOS requires metaKey AND !ctrlKey — 否则 Ctrl+K 会劫持 Cocoa 文本
 *   编辑的 kill-line（delete-to-end-of-line）快捷键；
 * - other platforms require ctrlKey AND !metaKey;
 * - Shift held → never fire (Cmd+Shift+K / Ctrl+Shift+K 是常见的
 *   「保留给 app 自己」的键位).
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
      // 事件时读取（而非模块级缓存），让测试可以 stub navigator.platform。
      const isMac = /mac/i.test(navigator.platform ?? "");
      const primaryMod = isMac ? e.metaKey : e.ctrlKey;
      const otherMod = isMac ? e.ctrlKey : e.metaKey;
      for (const b of bindings) {
        if (
          b.mod &&
          primaryMod &&
          !otherMod &&
          !e.shiftKey &&
          e.key.toLowerCase() === b.key
        ) {
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
