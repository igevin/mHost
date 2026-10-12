import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import styles from "./PopupMenu.module.css";

export interface PopupMenuItem {
  label: string;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

interface PopupMenuProps {
  /** Anchor rect of the ⋯ button (getBoundingClientRect at open time). */
  anchorRect: DOMRect;
  items: PopupMenuItem[];
  onClose: () => void;
}

const MENU_WIDTH = 160;
const MENU_ITEM_HEIGHT = 30;
const MENU_PADDING = 8;

/**
 * Self-drawn ⋯ popover menu — shared by the Sidebar profile rows and the
 * HostsProfileView header (Phase 2: 收敛三个并行的管理入口).
 *
 * Items bind **onClick only** — deliberately no onPointerDown: a
 * pointerdown-triggered select would unmount the menu *before* the trailing
 * synthetic click lands, and that click would hit whatever row/button sits
 * underneath the cursor (re-opening the menu or navigating the row). The
 * WebKit swallowed-click quirk (useWebKitPointerDown) only applies to the
 * input→button focus transfer, which can't happen here — the menu opens
 * from a button click, not from typing.
 */
function PopupMenu({ anchorRect, items, onClose }: PopupMenuProps) {
  // The ⋯ button opens this menu on *pointerdown* (WebKit swallowed-click
  // workaround, see useWebKitPointerDown). The trailing synthetic `click`
  // of that same gesture then lands on the freshly-mounted overlay and
  // would instantly close the menu. Distinguish it from a genuine outside
  // click by requiring the overlay to see its own pointerdown first: the
  // opening gesture's pointerdown happened on the ⋯ button (before the
  // overlay existed), so only its trailing click arrives — and is ignored.
  const overlayArmedRef = useRef(false);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  const menuHeight = items.length * MENU_ITEM_HEIGHT + MENU_PADDING * 2;
  const flipUp = anchorRect.bottom + 4 + menuHeight > window.innerHeight;
  const top = flipUp
    ? Math.max(8, anchorRect.top - 4 - menuHeight)
    : anchorRect.bottom + 4;
  const left = Math.max(
    8,
    Math.min(anchorRect.right - MENU_WIDTH, window.innerWidth - MENU_WIDTH - 8),
  );

  return createPortal(
    <>
      {/* Transparent click-catcher — closes on outside click. */}
      <div
        className={styles.overlay}
        onPointerDown={() => {
          overlayArmedRef.current = true;
        }}
        onClick={() => {
          if (!overlayArmedRef.current) return;
          overlayArmedRef.current = false;
          onClose();
        }}
      />
      <div
        className={styles.menu}
        style={{ top, left }}
        role="menu"
        onClick={(e) => e.stopPropagation()}
      >
        {items.map((item) => (
          <button
            key={item.label}
            role="menuitem"
            className={`${styles.menuItem} ${item.danger ? styles.menuItemDanger : ""}`}
            disabled={item.disabled}
            onClick={() => {
              if (item.disabled) return;
              onClose();
              item.onSelect();
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
    </>,
    document.body,
  );
}

export default PopupMenu;
