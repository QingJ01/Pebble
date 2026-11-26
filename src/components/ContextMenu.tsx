/**
 * ContextMenu — shared right-click / keyboard menu (role="menu").
 *
 *   const menu = useContextMenuState();
 *   <div onContextMenu={menu.openFromMouse} onKeyDown={menu.handleTriggerKeyDown}>…</div>
 *   {menu.position && <ContextMenu label="…" position={menu.position} items={items}
 *                        onClose={menu.close} />}
 *
 * - Portaled to <body>, layered with --z-menu, registered on the overlay stack
 *   (Escape closes it and nothing underneath).
 * - ↑/↓ wrap, Home/End, Enter/Space activate, Tab closes. First item focused.
 * - Closes on outside pointer down, window resize, scroll outside the menu and
 *   window blur; focus returns to the element that opened it.
 * - Positioned inside the viewport using its measured size (max-height 60vh).
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import OverlayPortal from "@/components/ui/OverlayPortal";
import { useOverlayLayer } from "@/hooks/useOverlayLayer";
import "./context-menu.css";

export interface ContextMenuItem {
  id: string;
  label: string;
  icon?: ReactNode;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
  /** Draw a separator above this item. */
  separatorBefore?: boolean;
}

export interface ContextMenuPosition {
  x: number;
  y: number;
}

interface Props {
  label: string;
  position: ContextMenuPosition;
  items: ContextMenuItem[];
  onClose: () => void;
  /** Element to focus when the menu closes (defaults to the element focused at open). */
  restoreFocusTo?: HTMLElement | null;
}

const MARGIN = 8;

export default function ContextMenu({ label, position, items, onClose, restoreFocusTo }: Props) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [placement, setPlacement] = useState<{ left: number; top: number } | null>(null);
  const restoreRef = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  if (restoreRef.current === null) {
    restoreRef.current = restoreFocusTo
      ?? (typeof document !== "undefined" ? (document.activeElement as HTMLElement | null) : null);
  }

  const close = useCallback(() => onCloseRef.current(), []);

  useOverlayLayer({ open: true, modal: false, onEscape: close });

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    const maxLeft = Math.max(MARGIN, window.innerWidth - rect.width - MARGIN);
    const maxTop = Math.max(MARGIN, window.innerHeight - rect.height - MARGIN);
    setPlacement({
      left: Math.min(Math.max(MARGIN, position.x), maxLeft),
      top: Math.min(Math.max(MARGIN, position.y), maxTop),
    });
  }, [position.x, position.y, items.length]);

  // Focus the first enabled item; restore focus on unmount.
  useEffect(() => {
    const first = menuRef.current?.querySelector<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])');
    first?.focus();
    return () => {
      const target = restoreRef.current;
      if (target && target.isConnected && document.activeElement !== target) {
        const active = document.activeElement;
        // Don't steal focus from something the action opened (dialog, compose).
        if (!active || active === document.body || menuRef.current?.contains(active) || !active.isConnected) {
          target.focus({ preventScroll: true });
        }
      }
    };
  }, []);

  useEffect(() => {
    function onPointerDown(event: Event) {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) close();
    }
    function onScroll(event: Event) {
      if (menuRef.current && event.target instanceof Node && menuRef.current.contains(event.target)) return;
      close();
    }
    document.addEventListener("mousedown", onPointerDown, true);
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    document.addEventListener("scroll", onScroll, true);
    return () => {
      document.removeEventListener("mousedown", onPointerDown, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, [close]);

  function menuItems(): HTMLElement[] {
    return Array.from(menuRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])') ?? []);
  }

  function handleKeyDown(event: React.KeyboardEvent) {
    // Keep keys inside the menu away from row handlers and global shortcuts.
    event.stopPropagation();
    const list = menuItems();
    const index = list.indexOf(document.activeElement as HTMLElement);
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        list[(index + 1) % list.length]?.focus();
        break;
      case "ArrowUp":
        event.preventDefault();
        list[(index - 1 + list.length) % list.length]?.focus();
        break;
      case "Home":
        event.preventDefault();
        list[0]?.focus();
        break;
      case "End":
        event.preventDefault();
        list[list.length - 1]?.focus();
        break;
      case "Tab":
        event.preventDefault();
        close();
        break;
      case "Escape":
        event.preventDefault();
        close();
        break;
      default:
        break;
    }
  }

  return (
    <OverlayPortal>
      <div
        ref={menuRef}
        role="menu"
        aria-label={label}
        className="context-menu anim-popover-in"
        style={{
          // First paint already uses the clamped placement (layout effect).
          left: placement?.left ?? position.x,
          top: placement?.top ?? position.y,
        }}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={handleKeyDown}
        onContextMenu={(event) => event.preventDefault()}
      >
        {items.map((item) => (
          <div key={item.id} role="none">
            {item.separatorBefore && <div role="separator" className="context-menu__separator" />}
            <button
              type="button"
              role="menuitem"
              tabIndex={-1}
              aria-disabled={item.disabled || undefined}
              className={`context-menu__item${item.danger ? " context-menu__item--danger" : ""}`}
              onClick={() => {
                if (item.disabled) return;
                close();
                item.onSelect();
              }}
            >
              <span className="context-menu__icon" aria-hidden="true">{item.icon}</span>
              <span>{item.label}</span>
            </button>
          </div>
        ))}
      </div>
    </OverlayPortal>
  );
}

/** Menu open state + trigger handlers (mouse right-click, Shift+F10, ContextMenu key). */
export function useContextMenuState() {
  const [position, setPosition] = useState<ContextMenuPosition | null>(null);
  const [trigger, setTrigger] = useState<HTMLElement | null>(null);

  const openAt = useCallback((x: number, y: number, from: HTMLElement | null) => {
    setTrigger(from);
    setPosition({ x, y });
  }, []);

  const openFromMouse = useCallback((event: React.MouseEvent<HTMLElement>) => {
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget;
    // Keyboard-generated contextmenu events carry no pointer position.
    if (event.clientX === 0 && event.clientY === 0) {
      const rect = target.getBoundingClientRect();
      openAt(rect.left + 16, rect.top + rect.height / 2, target);
    } else {
      openAt(event.clientX, event.clientY, target);
    }
  }, [openAt]);

  /** Shift+F10 / ContextMenu key on the trigger. Returns true when handled. */
  const handleTriggerKeyDown = useCallback((event: React.KeyboardEvent<HTMLElement>) => {
    if ((event.key === "F10" && event.shiftKey) || event.key === "ContextMenu") {
      event.preventDefault();
      event.stopPropagation();
      const rect = event.currentTarget.getBoundingClientRect();
      openAt(rect.left + 16, rect.top + rect.height / 2, event.currentTarget);
      return true;
    }
    return false;
  }, [openAt]);

  const close = useCallback(() => setPosition(null), []);

  return { position, trigger, openAt, openFromMouse, handleTriggerKeyDown, close };
}
