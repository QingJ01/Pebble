import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import { loadAttachmentPdf, previewAnnotationMode } from "@/lib/attachment-pdf";

export default function AttachmentPdfPreview({ base64, onError }: { base64: string; onError: () => void }) {
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(true);
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useEffect(() => {
    let cancelled = false;
    let pdf: ReturnType<typeof loadAttachmentPdf>;
    try { pdf = loadAttachmentPdf(base64); } catch { onErrorRef.current(); return; }
    const timeout = window.setTimeout(() => { if (!cancelled) { onErrorRef.current(); pdf.destroy(); } }, 30000);
    pdf.task.promise.then((loaded) => {
      if (cancelled) return;
      if (loaded.numPages > 2000) throw new Error("Too many pages");
      setDocument(loaded);
    }).catch(() => { if (!cancelled) onErrorRef.current(); }).finally(() => window.clearTimeout(timeout));
    return () => { cancelled = true; window.clearTimeout(timeout); pdf.destroy(); };
  }, [base64]);

  useEffect(() => {
    if (!document) return;
    let cancelled = false;
    let render: RenderTask | undefined;
    setLoading(true);
    const timeout = window.setTimeout(() => { if (!cancelled) { render?.cancel(); onErrorRef.current(); } }, 30000);
    document.getPage(page).then(async (pdfPage) => {
      if (cancelled || !canvasRef.current) return;
      const original = pdfPage.getViewport({ scale: 1 });
      if (!Number.isFinite(original.width) || !Number.isFinite(original.height) || original.width <= 0 || original.height <= 0) throw new Error("Invalid page size");
      const scale = Math.min(1.5, 1600 / original.width, 2000 / original.height);
      const viewport = pdfPage.getViewport({ scale });
      const canvas = canvasRef.current;
      canvas.width = Math.ceil(viewport.width);
      canvas.height = Math.ceil(viewport.height);
      render = pdfPage.render({ canvas, viewport, annotationMode: previewAnnotationMode });
      await render.promise;
      if (!cancelled) { setLoading(false); pdfPage.cleanup(); }
    }).catch(() => { if (!cancelled) onErrorRef.current(); }).finally(() => window.clearTimeout(timeout));
    return () => { cancelled = true; window.clearTimeout(timeout); render?.cancel(); };
  }, [document, page]);

  return <div className="attachment-pdf-preview">
    <nav aria-label={t("attachmentViewer.pdfPages")} className="attachment-preview-actions">
      <button disabled={!document || page <= 1} onClick={() => setPage((value) => value - 1)}>{t("attachmentViewer.previousPage")}</button>
      <span aria-live="polite">{document ? t("attachmentViewer.pageCount", { page, count: document.numPages }) : t("attachmentViewer.loading")}</span>
      <button disabled={!document || page >= document.numPages} onClick={() => setPage((value) => value + 1)}>{t("attachmentViewer.nextPage")}</button>
    </nav>
    {loading && <p role="status">{t("attachmentViewer.loading")}</p>}
    <canvas ref={canvasRef} role="img" aria-label={t("attachmentViewer.pdfPage", { page })} hidden={loading} />
  </div>;
}
