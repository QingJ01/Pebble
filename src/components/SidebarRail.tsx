/**
 * Collapsed sidebar (48px icon rail) building blocks, shared by SidebarGrouped
 * and SidebarClassic.
 *
 *   <SidebarRail label=… topLabel=… top={…} middleLabel=… middle={…} bottomLabel=… bottom={…} />
 *
 * Layout: a fixed top group, a divider, a scrolling middle group (accounts /
 * folders), a divider and a fixed bottom group. Every entry is a 32px square
 * centred in the rail (`.sidebar-btn--rail`). Styles live in src/app/shell.css.
 *
 * RailMenuButton is a rail entry that opens a menu to its right (account
 * folders, account switcher, overflow folders): Enter/Space/ArrowRight open it,
 * arrows move inside, Escape closes it and focus returns to the button.
 */
import { useEffect, useId, useRef, useState, type MouseEvent, type ReactNode } from "react";
import Popover, { handleMenuKeyDown } from "./ui/Popover";

export interface RailMenuItem {
  id: string;
  icon: ReactNode;
  label: string;
  /** Right-aligned count (unread); hidden when 0/undefined. */
  count?: number;
  /** Marks the entry that is currently shown (aria-current, neutral fill). */
  current?: boolean;
  onSelect: () => void;
  /** Makes the entry a drop target for dragged mail (see lib/mailDrag). */
  dropFolderId?: string;
}

/** Hovering a rail menu this long while dragging mail opens it. */
const DRAG_OPEN_DELAY_MS = 450;
const isDraggingMail = () => document.body.classList.contains("is-dragging-mail");

export function RailMenuButton({
  face,
  label,
  title,
  isActive = false,
  menuLabel,
  menuHeader,
  items,
  footerItems,
  emptyNote,
  className,
}: {
  /** Visual content of the 32px button (icon or AccountBadge). */
  face: ReactNode;
  /** Accessible name of the button. */
  label: string;
  /** Tooltip; defaults to `label`. */
  title?: string;
  isActive?: boolean;
  menuLabel: string;
  menuHeader?: ReactNode;
  items: RailMenuItem[];
  /** Entries after a separator (e.g. "Manage folders"). */
  footerItems?: RailMenuItem[];
  /** Shown when `items` is empty. */
  emptyNote?: string;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  /** Opened by hovering during a mail drag: closes when the drag ends. */
  const [openedByDrag, setOpenedByDrag] = useState(false);
  const dragOpenTimer = useRef<number | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();
  const hasDropTargets = items.some((item) => item.dropFolderId);

  const cancelDragOpen = () => {
    if (dragOpenTimer.current !== null) window.clearTimeout(dragOpenTimer.current);
    dragOpenTimer.current = null;
  };
  useEffect(() => cancelDragOpen, []);
  useEffect(() => {
    if (!openedByDrag) return;
    // The drag ends on release (dropped or not): close the spring-loaded menu.
    const close = () => {
      setOpen(false);
      setOpenedByDrag(false);
    };
    window.addEventListener("pointerup", close);
    window.addEventListener("pointercancel", close);
    return () => {
      window.removeEventListener("pointerup", close);
      window.removeEventListener("pointercancel", close);
    };
  }, [openedByDrag]);

  function select(item: RailMenuItem) {
    setOpen(false);
    buttonRef.current?.focus();
    item.onSelect();
  }

  function renderItem(item: RailMenuItem) {
    return (
      <button
        key={item.id}
        type="button"
        role="menuitem"
        className={["menu-item", "sidebar-rail-menu__item", item.current ? "is-current" : null].filter(Boolean).join(" ")}
        aria-current={item.current ? "page" : undefined}
        data-autofocus={item.current || undefined}
        data-drop-folder-id={item.dropFolderId}
        onClick={() => select(item)}
      >
        <span className="sidebar-rail-menu__icon" aria-hidden="true">{item.icon}</span>
        <span className="sidebar-rail-menu__label">{item.label}</span>
        {item.count != null && item.count > 0 && (
          <span className="sidebar-rail-menu__count">{item.count}</span>
        )}
      </button>
    );
  }

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className={["sidebar-btn", "sidebar-btn--rail", isActive ? "is-active" : null, className].filter(Boolean).join(" ")}
        aria-label={label}
        title={title ?? label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={(event: MouseEvent<HTMLButtonElement>) => {
          event.currentTarget.focus();
          setOpen((value) => !value);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowRight" || event.key === "ArrowDown") {
            event.preventDefault();
            setOpen(true);
          }
        }}
        onPointerEnter={() => {
          if (!hasDropTargets || open || !isDraggingMail()) return;
          cancelDragOpen();
          dragOpenTimer.current = window.setTimeout(() => {
            dragOpenTimer.current = null;
            if (!isDraggingMail()) return;
            setOpenedByDrag(true);
            setOpen(true);
          }, DRAG_OPEN_DELAY_MS);
        }}
        onPointerLeave={cancelDragOpen}
      >
        {face}
      </button>
      <Popover
        open={open}
        onClose={() => {
          setOpen(false);
          setOpenedByDrag(false);
        }}
        anchorRef={buttonRef}
        placement="right-start"
        gap={12}
        role="menu"
        id={menuId}
        aria-label={menuLabel}
        className="menu sidebar-rail-menu"
        onKeyDown={handleMenuKeyDown}
      >
        {menuHeader && <div className="menu-label sidebar-rail-menu__header" role="presentation">{menuHeader}</div>}
        {items.length > 0
          ? items.map(renderItem)
          : emptyNote && <div className="sidebar-rail-menu__empty" role="presentation">{emptyNote}</div>}
        {footerItems && footerItems.length > 0 && (
          <>
            <div className="menu-separator" role="separator" />
            {footerItems.map(renderItem)}
          </>
        )}
      </Popover>
    </>
  );
}

export default function SidebarRail({
  label,
  top,
  topLabel,
  middle,
  middleLabel,
  bottom,
  bottomLabel,
  onContextMenu,
  children,
}: {
  label: string;
  top: ReactNode;
  topLabel: string;
  middle: ReactNode;
  middleLabel: string;
  bottom: ReactNode;
  bottomLabel: string;
  onContextMenu?: (event: MouseEvent) => void;
  /** Overlays (folder management). */
  children?: ReactNode;
}) {
  return (
    <aside className="app-sidebar app-sidebar--rail" aria-label={label} onContextMenu={onContextMenu}>
      <nav className="sidebar-rail__section sidebar-rail__section--top" aria-label={topLabel}>{top}</nav>
      <div className="sidebar-rail__divider" role="presentation" />
      <nav className="sidebar-rail__section sidebar-rail__section--middle" aria-label={middleLabel}>
        {middle}
      </nav>
      <div className="sidebar-rail__divider" role="presentation" />
      <nav className="sidebar-rail__section sidebar-rail__section--bottom" aria-label={bottomLabel}>
        {bottom}
      </nav>
      {children}
    </aside>
  );
}
