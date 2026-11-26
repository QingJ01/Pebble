/**
 * Dialog — modal shell shared by every app dialog.
 *
 *   <Dialog open={open} onClose={close} labelledBy={titleId} initialFocus={okRef}>
 *     <h2 id={titleId} className="dialog-title">…</h2>
 *     …
 *     <div className="dialog-actions"><Button …/></div>
 *   </Dialog>
 *
 * - Portaled to document.body; backdrop/panel use the overlay tokens
 *   (.overlay-backdrop / .dialog-panel, --z-modal or --z-nested).
 * - On the overlay stack as a modal layer: Escape (only when top-most and
 *   `dismissible`) calls onClose; Tab is trapped inside the panel.
 * - Focus moves in on open (initialFocus, default first focusable) and is
 *   restored to the trigger on close.
 * - Exit animation via usePresence + .anim-dialog-out / .anim-backdrop-out:
 *   keep the component mounted and toggle `open` to get it.
 * - React events inside do not bubble to React parents of the portal.
 */
import { useRef, type CSSProperties, type ReactNode } from "react";
import OverlayPortal from "./OverlayPortal";
import { useOverlayLayer } from "@/hooks/useOverlayLayer";
import { usePresence } from "@/hooks/usePresence";
import { useFocusScope, type InitialFocus } from "@/hooks/useFocusScope";
import "./overlay.css";

export interface DialogProps {
  open?: boolean;
  onClose: () => void;
  /** When false, Escape and backdrop clicks are ignored (e.g. while busy). */
  dismissible?: boolean;
  closeOnBackdrop?: boolean;
  labelledBy?: string;
  ariaLabel?: string;
  describedBy?: string;
  busy?: boolean;
  role?: "dialog" | "alertdialog";
  initialFocus?: InitialFocus;
  /** Stack above another dialog (--z-nested). */
  nested?: boolean;
  /** Replaces the default `dialog-panel` class. */
  panelClassName?: string;
  className?: string;
  style?: CSSProperties;
  children: ReactNode;
}

const stop = (event: { stopPropagation: () => void }) => event.stopPropagation();

export default function Dialog({
  open = true,
  onClose,
  dismissible = true,
  closeOnBackdrop = true,
  labelledBy,
  ariaLabel,
  describedBy,
  busy,
  role = "dialog",
  initialFocus = "first",
  nested = false,
  panelClassName = "dialog-panel",
  className,
  style,
  children,
}: DialogProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const { mounted, exiting } = usePresence(open);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const dismissibleRef = useRef(dismissible);
  dismissibleRef.current = dismissible;
  const downOnBackdrop = useRef(false);

  useOverlayLayer({
    open,
    modal: true,
    containerRef: panelRef,
    onEscape: () => {
      if (dismissibleRef.current) onCloseRef.current();
    },
  });
  useFocusScope({ active: open, containerRef: panelRef, initialFocus });

  if (!mounted) return null;

  return (
    <OverlayPortal>
      <div
        className={[
          "overlay-backdrop",
          nested ? "overlay-backdrop--nested" : "",
          exiting ? "anim-backdrop-out" : "anim-backdrop-in",
        ].filter(Boolean).join(" ")}
        role="presentation"
        aria-hidden={exiting || undefined}
        onMouseDown={(event) => {
          event.stopPropagation();
          downOnBackdrop.current = event.target === event.currentTarget;
        }}
        onClick={(event) => {
          event.stopPropagation();
          const fromBackdrop = downOnBackdrop.current && event.target === event.currentTarget;
          downOnBackdrop.current = false;
          if (fromBackdrop && closeOnBackdrop && dismissible && open) onClose();
        }}
        onDoubleClick={stop}
        onContextMenu={stop}
        onKeyDown={stop}
      >
        <div
          ref={panelRef}
          role={role}
          aria-modal="true"
          aria-labelledby={labelledBy}
          aria-label={labelledBy ? undefined : ariaLabel}
          aria-describedby={describedBy}
          aria-busy={busy || undefined}
          tabIndex={-1}
          inert={exiting || undefined}
          className={[panelClassName, exiting ? "anim-dialog-out" : "anim-dialog-in", className].filter(Boolean).join(" ")}
          style={style}
        >
          {children}
        </div>
      </div>
    </OverlayPortal>
  );
}
