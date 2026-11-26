import { useId, type ReactNode } from "react";
import Dialog from "./ui/Dialog";
import type { InitialFocus } from "@/hooks/useFocusScope";
import "./folder-management.css";

/** Form-style modal used by folder management, move-to and the sender notice. */
export default function FolderModal({
  title,
  busy,
  onClose,
  open = true,
  initialFocus,
  children,
}: {
  title: string;
  busy: boolean;
  onClose: () => void;
  open?: boolean;
  initialFocus?: InitialFocus;
  children: ReactNode;
}) {
  const titleId = useId();
  return (
    <Dialog
      open={open}
      onClose={onClose}
      dismissible={!busy}
      busy={busy}
      labelledBy={titleId}
      initialFocus={initialFocus}
      panelClassName="dialog-panel folder-modal"
    >
      <h2 id={titleId} className="dialog-title">{title}</h2>
      {children}
    </Dialog>
  );
}
