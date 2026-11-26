import { useEffect, useId, useRef, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useOverlayLayer } from "@/hooks/useOverlayLayer";
import { usePresence } from "@/hooks/usePresence";
import OverlayPortal from "./ui/OverlayPortal";
import Button from "./ui/Button";

interface Props {
  /**
   * Keep the dialog mounted and toggle this for an exit animation. Callers that
   * mount it conditionally can omit it (always open; no exit animation).
   */
  open?: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  destructive?: boolean;
  busy?: boolean;
  confirmDisabled?: boolean;
  busyConfirmLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
  /** Optional third action, rendered left of Cancel as a secondary button. */
  extraActionLabel?: string;
  onExtraAction?: () => void;
  /** Error shown under the message (role=alert), e.g. when onConfirm failed. */
  error?: ReactNode;
}

/**
 * Modal confirm dialog. Portaled to document.body (z: --z-modal), registered on
 * the overlay stack: Escape cancels (unless busy) and Tab is trapped only while
 * this dialog is the top-most overlay; handled keys never reach global shortcuts.
 */
export default function ConfirmDialog({
  open = true,
  title,
  message,
  confirmLabel,
  cancelLabel,
  destructive,
  busy = false,
  confirmDisabled = false,
  busyConfirmLabel,
  onConfirm,
  onCancel,
  extraActionLabel,
  onExtraAction,
  error,
}: Props) {
  const { t } = useTranslation();
  const titleId = useId();
  const messageId = useId();
  const errorId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const busyRef = useRef(busy);
  busyRef.current = busy;
  const onCancelRef = useRef(onCancel);
  onCancelRef.current = onCancel;

  const { mounted, exiting } = usePresence(open);

  useOverlayLayer({
    open,
    modal: true,
    containerRef: dialogRef,
    onEscape: () => {
      if (!busyRef.current) onCancelRef.current();
    },
  });

  useEffect(() => {
    if (!open) return;
    const previousFocus =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;

    // For destructive actions, focus Cancel by default to prevent accidental confirmation
    if (destructive) {
      cancelRef.current?.focus();
    } else {
      confirmRef.current?.focus();
    }

    return () => {
      if (previousFocus?.isConnected) previousFocus.focus();
    };
  }, [open, destructive]);

  const confirmText = busy
    ? busyConfirmLabel || confirmLabel || t("common.confirm", "Confirm")
    : confirmLabel || t("common.confirm", "Confirm");

  if (!mounted) return null;

  return (
    <OverlayPortal>
      <div
        className={`overlay-backdrop ${exiting ? "anim-backdrop-out" : "anim-backdrop-in"}`}
        aria-hidden={exiting || undefined}
      >
        <div
          ref={dialogRef}
          inert={exiting || undefined}
          role="dialog"
          aria-modal="true"
          aria-busy={busy || undefined}
          aria-labelledby={titleId}
          aria-describedby={error ? `${messageId} ${errorId}` : messageId}
          className={`dialog-panel ${exiting ? "anim-dialog-out" : "anim-dialog-in"}`}
        >
          <h3 id={titleId} className="dialog-title">{title}</h3>
          <p id={messageId} className="dialog-message">{message}</p>
          {error && (
            <p id={errorId} role="alert" className="dialog-error">{error}</p>
          )}
          <div className="dialog-actions">
            {extraActionLabel && onExtraAction && (
              <Button
                variant="secondary"
                className="dialog-actions__extra"
                disabled={confirmDisabled}
                aria-disabled={busy || undefined}
                onClick={onExtraAction}
              >
                {extraActionLabel}
              </Button>
            )}
            <Button
              ref={cancelRef}
              variant="secondary"
              aria-disabled={busy || undefined}
              onClick={() => { if (!busy && open) onCancel(); }}
            >
              {cancelLabel || t("common.cancel", "Cancel")}
            </Button>
            <Button
              ref={confirmRef}
              variant={destructive ? "danger" : "primary"}
              disabled={confirmDisabled}
              loading={busy}
              onClick={() => { if (!busy && !confirmDisabled && open) onConfirm(); }}
            >
              {confirmText}
            </Button>
          </div>
        </div>
      </div>
    </OverlayPortal>
  );
}
