import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { Download, Loader, Check } from "lucide-react";
import { listAttachments, downloadAttachment } from "@/lib/api";
import type { Attachment } from "@/lib/api";
import { sanitizeFilename } from "@/lib/sanitizeFilename";
import { attachmentVisual } from "./attachmentVisual";
import { useToastStore } from "@/stores/toast.store";
import { canPreviewAttachment, saveAllAttachments, type AttachmentSaveResult } from "@/lib/attachments-api";
import AttachmentPreview from "./AttachmentPreview";

interface Props {
  messageId: string;
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function getErrorMessage(err: unknown): string | null {
  if (typeof err === "string") return err;
  if (!err || typeof err !== "object") return null;
  const record = err as Record<string, unknown>;
  if (typeof record.message === "string") return record.message;
  if (typeof record.error === "string") return record.error;
  return null;
}

export default function AttachmentList({ messageId }: Props) {
  return <MessageAttachments key={messageId} messageId={messageId} />;
}

function MessageAttachments({ messageId }: Props) {
  const { t } = useTranslation();
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [loading, setLoading] = useState(true);
  const [downloadingId, setDownloadingId] = useState<string | null>(null);
  const [downloadedPaths, setDownloadedPaths] = useState<Record<string, string>>({});
  const [downloadProgress, setDownloadProgress] = useState<Record<string, number>>({});
  const [previewIndex, setPreviewIndex] = useState<number | null>(null);
  const [savingAll, setSavingAll] = useState(false);
  const [saveResults, setSaveResults] = useState<AttachmentSaveResult[] | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const activeMessage = useRef(messageId);
  activeMessage.current = messageId;
  const previewable = attachments.filter((attachment) => canPreviewAttachment(attachment.mime_type));
  useEffect(() => {
    activeMessage.current = messageId;
    return () => { activeMessage.current = ""; };
  }, [messageId]);

  // Listen for download progress events
  useEffect(() => {
    const unlisten = listen<{ attachment_id: string; bytes_copied: number; total_bytes: number }>(
      "attachment:download-progress",
      (event) => {
        const { attachment_id, bytes_copied, total_bytes } = event.payload;
        const pct = total_bytes > 0 ? Math.round((bytes_copied / total_bytes) * 100) : 0;
        setDownloadProgress((prev) => ({ ...prev, [attachment_id]: pct }));
      },
    );
    return () => { void unlisten.then((fn) => fn()).catch(() => {}); };
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setAttachments([]); setLoadError(false); setPreviewIndex(null); setSaveResults(null);
    setDownloadedPaths({}); setDownloadProgress({}); setDownloadingId(null); setSavingAll(false);

    listAttachments(messageId)
      .then((list) => {
        if (!cancelled) {
          setAttachments(list.filter((a) => !a.is_inline));
        }
      })
      .catch(() => {
        if (!cancelled) { setAttachments([]); setLoadError(true); }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [messageId, loadAttempt]);

  async function handleDownload(attachment: Attachment) {
    const currentMessage = messageId;
    setDownloadingId(attachment.id);
    try {
      const { downloadDir } = await import("@tauri-apps/api/path");
      const dir = await downloadDir();
      const safeName = sanitizeFilename(attachment.filename);
      const savePath = `${dir}/${safeName}`;
      const downloadedPath = await downloadAttachment(attachment.id, savePath);
      if (activeMessage.current !== currentMessage) return;
      setDownloadedPaths((prev) => ({ ...prev, [attachment.id]: downloadedPath }));
      setDownloadProgress((prev) => { const next = { ...prev }; delete next[attachment.id]; return next; });
    } catch (err) {
      const reason = getErrorMessage(err);
      useToastStore.getState().addToast({
        message: reason
          ? t("attachments.downloadFailedWithReason", "Failed to download attachment: {{reason}}", { reason })
          : t("attachments.downloadFailed", "Failed to download attachment"),
        type: "error",
      });
    } finally {
      if (activeMessage.current === currentMessage) setDownloadingId(null);
    }
  }

  async function handleSaveAll() {
    const currentMessage = messageId;
    setSavingAll(true); setSaveResults(null);
    try {
      const results = await saveAllAttachments(currentMessage);
      if (activeMessage.current !== currentMessage || results === null) return;
      setSaveResults(results);
      setDownloadedPaths((previous) => ({ ...previous, ...Object.fromEntries(results.filter((result) => result.saved_path).map((result) => [result.attachment_id, result.saved_path!])) }));
    } catch (error) {
      useToastStore.getState().addToast({ message: t("attachmentViewer.saveFailed", { reason: getErrorMessage(error) ?? "" }), type: "error" });
    } finally {
      if (activeMessage.current === currentMessage) { setSavingAll(false); setDownloadProgress({}); }
    }
  }

  if (loading) return <p role="status">{t("attachmentViewer.loadingAttachments")}</p>;
  if (loadError) return <div role="alert">{t("attachmentViewer.listFailed")} <button onClick={() => setLoadAttempt((value) => value + 1)}>{t("attachmentViewer.retry")}</button></div>;
  if (attachments.length === 0) return null;

  return (
    <div
      style={{
        padding: "12px 16px",
        borderTop: "1px solid var(--color-border)",
        backgroundColor: "var(--color-bg)",
      }}
    >
      <div className="attachment-list-heading"><div
        style={{
          fontSize: "12px",
          fontWeight: "600",
          color: "var(--color-text-secondary)",
          textTransform: "uppercase",
          letterSpacing: "0.5px",
        }}
      >
        {t("attachments.title")} ({attachments.length})
      </div><button className="attachment-save-all" onClick={handleSaveAll} disabled={savingAll || downloadingId !== null}>{t(savingAll ? "attachmentViewer.savingAll" : "attachmentViewer.saveAll")}</button></div>
      <div className="attachment-list">
        {attachments.map((attachment) => {
          const { Icon, color } = attachmentVisual(attachment.mime_type, attachment.filename);
          const isDownloading = downloadingId === attachment.id || (savingAll && downloadProgress[attachment.id] != null && downloadProgress[attachment.id] < 100);

          return (
            <div key={attachment.id} className="attachment-card">
              <div className="attachment-icon-tile" style={{ backgroundColor: `${color}1f` }}>
                <Icon size={17} color={color} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="attachment-name" title={attachment.filename}>
                  {canPreviewAttachment(attachment.mime_type) ? <button className="attachment-open-preview" onClick={() => setPreviewIndex(previewable.findIndex((item) => item.id === attachment.id))} aria-label={t("attachmentViewer.preview") + ": " + attachment.filename}>{attachment.filename}</button> : attachment.filename}
                </div>
                <div className="attachment-meta">{formatFileSize(attachment.size)}</div>
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: "6px", flexShrink: 0 }}>
                {isDownloading && downloadProgress[attachment.id] != null && (
                  <span style={{ fontSize: "10px", color: "var(--color-accent)", minWidth: "28px", textAlign: "right" }}>
                    {downloadProgress[attachment.id]}%
                  </span>
                )}
                <button
                  onClick={() => handleDownload(attachment)}
                  disabled={downloadingId !== null || savingAll}
                  aria-label={t("attachments.download") + ": " + attachment.filename}
                  title={isDownloading ? t("attachments.downloading") : downloadedPaths[attachment.id] ? downloadedPaths[attachment.id] : t("attachments.download")}
                  className="attachment-download"
                >
                  {isDownloading ? (
                    <Loader size={14} className="spinner" />
                  ) : downloadedPaths[attachment.id] ? (
                    <Check size={14} style={{ color: "var(--color-accent)" }} />
                  ) : (
                    <Download size={14} />
                  )}
                </button>
              </div>
            </div>
          );
        })}
      </div>
      {saveResults && <div className="attachment-save-results" role="status">
        <p>{t("attachmentViewer.saveSummary", { saved: saveResults.filter((result) => result.saved_path).length, failed: saveResults.filter((result) => result.error).length })}</p>
        <ul>{saveResults.map((result) => <li key={result.attachment_id}>{result.filename}: {result.saved_path ?? result.error}</li>)}</ul>
      </div>}
      {previewIndex !== null && previewable[previewIndex] && <AttachmentPreview attachments={previewable} index={previewIndex} onNavigate={setPreviewIndex} onClose={() => setPreviewIndex(null)} onDownload={handleDownload} downloading={downloadingId !== null || savingAll} />}
    </div>
  );
}
