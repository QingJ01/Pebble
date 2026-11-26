import { Component, lazy, Suspense, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight, Download, X } from "lucide-react";
import type { Attachment } from "@/lib/api";
import { previewAttachment, type AttachmentPreviewData } from "@/lib/attachments-api";
import { describeError } from "@/lib/extractErrorMessage";
import IconButton from "./ui/IconButton";
import Button from "./ui/Button";
import { useOverlayLayer } from "@/hooks/useOverlayLayer";
import "./attachment-preview.css";

const AttachmentPdfPreview = lazy(() => import("./AttachmentPdfPreview"));

class PreviewBoundary extends Component<{ children: ReactNode; onError: () => void }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch() { this.props.onError(); }
  render() { return this.state.failed ? null : this.props.children; }
}

interface Props {
  attachments: Attachment[];
  index: number;
  onNavigate: (index: number) => void;
  onClose: () => void;
  onDownload: (attachment: Attachment) => void;
  downloading: boolean;
}

export default function AttachmentPreview(props: Props) {
  const { t } = useTranslation();
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const [loaded, setLoaded] = useState<{ id: string; data: AttachmentPreviewData } | null>(null);
  const [failure, setFailure] = useState<{ id: string; reason: string } | null>(null);
  const [imageLoading, setImageLoading] = useState(true);
  const [attempt, setAttempt] = useState(0);
  const attachment = props.attachments[props.index];
  const preview = loaded?.id === attachment.id ? loaded.data : null;
  const error = failure?.id === attachment.id ? failure.reason : null;
  const setError = (reason: string) => setFailure({ id: attachment.id, reason });
  const layer = useOverlayLayer({ modal: true });

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    function handleKey(event: KeyboardEvent) {
      // A dialog stacked above this viewer owns the keyboard.
      if (!layer.isTop()) return;
      const { index, attachments, onNavigate, onClose } = latest.current;
      // Do not let mail-list shortcuts act on the message behind this modal.
      event.stopImmediatePropagation();
      if (event.key === "Escape") { event.preventDefault(); onClose(); }
      if (event.key === "ArrowLeft" && index > 0) { event.preventDefault(); onNavigate(index - 1); }
      if (event.key === "ArrowRight" && index < attachments.length - 1) { event.preventDefault(); onNavigate(index + 1); }
      if (event.key === "Tab") {
        const buttons = Array.from(dialogRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
        const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.shiftKey ? (current <= 0 ? buttons.length - 1 : current - 1) : (current + 1) % buttons.length;
        event.preventDefault(); buttons[next]?.focus();
      }
    }
    document.addEventListener("keydown", handleKey, true);
    return () => { document.removeEventListener("keydown", handleKey, true); if (previous?.isConnected) previous.focus(); };
  }, [layer]);

  useEffect(() => {
    let cancelled = false;
    setLoaded(null); setFailure(null); setImageLoading(true);
    previewAttachment(attachment.id).then((data) => { if (!cancelled) setLoaded({ id: attachment.id, data }); }).catch((reason: unknown) => {
      if (!cancelled) setFailure({ id: attachment.id, reason: describeError(reason).detail });
    });
    return () => { cancelled = true; };
  }, [attachment.id, attempt]);

  return createPortal(<div className="overlay-backdrop anim-backdrop-in attachment-preview-overlay" role="presentation" onClick={(event) => { event.stopPropagation(); if (event.target === event.currentTarget) props.onClose(); }}>
    <div className="attachment-preview-dialog anim-dialog-in" role="dialog" aria-modal="true" aria-labelledby={titleId} ref={dialogRef}>
      <header className="attachment-preview-header">
        <h2 id={titleId} title={attachment.filename}>{attachment.filename}</h2>
        <IconButton size="md" icon={<Download size={16} />} onClick={() => props.onDownload(attachment)} loading={props.downloading} label={t("attachments.downloadNamed", { name: attachment.filename })} tooltip={t("attachments.download")} />
        <IconButton ref={closeRef} size="md" icon={<X size={18} />} onClick={props.onClose} label={t("attachmentViewer.close")} />
      </header>
      <nav className="attachment-preview-actions" aria-label={t("attachmentViewer.navigation")}>
        <IconButton variant="secondary" icon={<ChevronLeft size={16} />} disabled={props.index === 0} onClick={() => props.onNavigate(props.index - 1)} label={t("attachmentViewer.previous")} />
        <span aria-live="polite">{props.index + 1} / {props.attachments.length}</span>
        <IconButton variant="secondary" icon={<ChevronRight size={16} />} disabled={props.index === props.attachments.length - 1} onClick={() => props.onNavigate(props.index + 1)} label={t("attachmentViewer.next")} />
        <span className="attachment-preview-hint">{t("attachmentViewer.keyboardHint")}</span>
      </nav>
      <div className="attachment-preview-body" key={attachment.id + ":" + attempt}>
        {error !== null ? <div role="alert"><p>{t("attachmentViewer.failed")}</p>{error && <p className="attachment-preview-reason">{error}</p>}<p>{t("attachmentViewer.fallback")}</p><Button variant="secondary" size="sm" onClick={() => setAttempt((value) => value + 1)}>{t("attachmentViewer.retry")}</Button></div> : !preview ? <p role="status">{t("attachmentViewer.loading")}</p> : preview.mime_type === "image/png" ? <>
          {imageLoading && <p role="status">{t("attachmentViewer.loading")}</p>}
          <img src={`data:image/png;base64,${preview.base64}`} alt={attachment.filename} hidden={imageLoading} onLoad={() => setImageLoading(false)} onError={() => setError("")} />
        </> : <PreviewBoundary onError={() => setError("")}><Suspense fallback={<p role="status">{t("attachmentViewer.loading")}</p>}><AttachmentPdfPreview base64={preview.base64} onError={() => setError("")} /></Suspense></PreviewBoundary>}
      </div>
      <footer>{t("attachmentViewer.staticPreview")}</footer>
    </div>
  </div>, document.body);
}
