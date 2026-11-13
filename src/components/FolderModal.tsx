import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import "./folder-management.css";

export default function FolderModal({ title, busy, onClose, children }: {title: string; busy: boolean; onClose: () => void; children: ReactNode}) {
  const ref = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose); closeRef.current = onClose;
  const busyRef = useRef(busy); busyRef.current = busy;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const focusable = () => Array.from(ref.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),[tabindex="0"]') ?? []);
    focusable()[0]?.focus();
    function keydown(event: KeyboardEvent) {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (!busyRef.current) closeRef.current(); }
      if (event.key === "Tab") {
        const items = focusable();
        if (!items.length) { event.preventDefault(); return; }
        const first = items[0], last = items[items.length-1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      }
    }
    document.addEventListener("keydown", keydown, true);
    return () => { document.removeEventListener("keydown", keydown, true); previous?.focus(); };
  }, []);
  return createPortal(<div className="folder-modal-backdrop" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
    <div ref={ref} role="dialog" aria-modal="true" aria-label={title} aria-busy={busy} className="folder-modal">
      <h2>{title}</h2>{children}
    </div>
  </div>, document.body);
}
