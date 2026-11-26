/**
 * Popover — non-modal floating panel portaled to document.body.
 *
 *   <Popover open={open} onClose={close} anchorRef={buttonRef} placement="bottom-end"
 *            role="menu" aria-label={t("snooze.until")}>…</Popover>
 *   <Popover open point={{ x, y }} onClose={close}>…</Popover>
 *
 * - Positioned with @floating-ui (flip/shift/size; max-height ≤ 60vh, scrolls).
 * - On the overlay stack as a non-modal layer: Escape closes it only while it
 *   is the top-most overlay; global shortcuts never see that Escape.
 * - Pointer down outside the panel (and outside anchorRef) closes it, but only
 *   while it is top-most (a dialog opened above it keeps it alive).
 * - Focus moves in on open (initialFocus, default first focusable) and returns
 *   to the trigger on close. Exit animation via usePresence + .anim-popover-*.
 * - React events from inside the panel do not bubble to the React parents
 *   (portal events otherwise reach e.g. a list row's onClick).
 */
import {
  useEffect,
  useRef,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type HTMLAttributes,
  type ReactNode,
  type RefObject,
} from "react";
import type { Placement } from "@floating-ui/dom";
import OverlayPortal from "./OverlayPortal";
import { useOverlayLayer } from "@/hooks/useOverlayLayer";
import { usePresence } from "@/hooks/usePresence";
import { useFocusScope, type InitialFocus } from "@/hooks/useFocusScope";
import { useFloatingPosition, type FloatingPoint } from "@/hooks/useFloatingPosition";
import "./overlay.css";

export interface PopoverProps extends Omit<HTMLAttributes<HTMLDivElement>, "children" | "role"> {
  open: boolean;
  onClose: () => void;
  anchorRef?: RefObject<HTMLElement | null>;
  point?: FloatingPoint | null;
  placement?: Placement;
  gap?: number;
  initialFocus?: InitialFocus;
  restoreFocus?: boolean;
  role?: string;
  className?: string;
  style?: CSSProperties;
  panelRef?: RefObject<HTMLDivElement | null>;
  children: ReactNode;
}

const stop = (event: { stopPropagation: () => void }) => event.stopPropagation();

export default function Popover({
  open,
  onClose,
  anchorRef,
  point,
  placement = "bottom-start",
  gap,
  initialFocus = "first",
  restoreFocus = true,
  role = "dialog",
  className,
  style,
  panelRef: externalRef,
  children,
  ...rest
}: PopoverProps) {
  const innerRef = useRef<HTMLDivElement>(null);
  const panelRef = externalRef ?? innerRef;
  const { mounted, exiting } = usePresence(open);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  const layer = useOverlayLayer({
    open,
    modal: false,
    onEscape: () => onCloseRef.current(),
    containerRef: panelRef,
    trapFocus: false,
  });
  useFocusScope({ active: open, containerRef: panelRef, initialFocus, restoreFocus });
  useFloatingPosition({ open: mounted, floatingRef: panelRef, anchorRef, point, placement, gap });

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent | MouseEvent) => {
      if (!layer.isTop()) return;
      const target = event.target as Node | null;
      if (!target) return;
      if (panelRef.current?.contains(target)) return;
      if (anchorRef?.current?.contains(target)) return;
      onCloseRef.current();
    };
    document.addEventListener("mousedown", onPointerDown, true);
    return () => document.removeEventListener("mousedown", onPointerDown, true);
  }, [open, layer, panelRef, anchorRef]);

  if (!mounted) return null;

  const initialLeft = point ? point.x : undefined;
  const initialTop = point ? point.y + (gap ?? 6) : undefined;

  return (
    <OverlayPortal>
      <div
        {...rest}
        ref={panelRef}
        role={role}
        className={["popover-panel", exiting ? "anim-popover-out" : "anim-popover-in", className].filter(Boolean).join(" ")}
        style={{ left: initialLeft, top: initialTop, ...style }}
        aria-hidden={exiting || undefined}
        inert={exiting || undefined}
        onClick={stop}
        onMouseDown={stop}
        onDoubleClick={stop}
        onContextMenu={stop}
        onKeyDown={(event) => {
          rest.onKeyDown?.(event);
          event.stopPropagation();
        }}
      >
        {children}
      </div>
    </OverlayPortal>
  );
}

/** Arrow/Home/End roving focus for role="menu" containers. Attach as onKeyDown. */
export function handleMenuKeyDown(event: ReactKeyboardEvent<HTMLElement>) {
  const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
  if (!keys.includes(event.key)) return;
  const items = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled):not([aria-disabled="true"])'),
  );
  if (items.length === 0) return;
  event.preventDefault();
  const index = items.indexOf(document.activeElement as HTMLElement);
  let next = 0;
  if (event.key === "ArrowDown") next = index < 0 || index === items.length - 1 ? 0 : index + 1;
  else if (event.key === "ArrowUp") next = index <= 0 ? items.length - 1 : index - 1;
  else if (event.key === "End") next = items.length - 1;
  items[next]?.focus();
}
