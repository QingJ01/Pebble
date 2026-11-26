import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { Download, Check } from "lucide-react";
import { listAttachments, downloadAttachment } from "@/lib/api";
import type { Attachment } from "@/lib/api";
import { sanitizeFilename } from "@/lib/sanitizeFilename";
import { attachmentVisual } from "./attachmentVisual";
import { useToastStore } from "@/stores/toast.store";
import { canPreviewAttachment, saveAllAttachments, type AttachmentSaveResult } from "@/lib/attachments-api";
import AttachmentPreview from "./AttachmentPreview";
import Button from "./ui/Button";
import IconButton from "./ui/IconButton";
import { describeError } from "@/lib/extractErrorMessage";

interface Props {
  messageId: string;
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Raw backend reason (localised kind titles are too generic here), or null. */
function getErrorMessage(err: unknown): string | null {
  if (err && typeof err === "object" && !("message" in err) && typeof (err as { error?: unknown }).error === "string") {
    return (err as { error: string }).error;
  }
  return describeError(err).detail || null;
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

  if (loading) return <p role="status" className="attachment-section attachment-section__status">{t("attachmentViewer.loadingAttachments")}</p>;
  if (loadError) return <div role="alert" className="attachment-section attachment-section__status">{t("attachmentViewer.listFailed")} <Button size="sm" variant="secondary" onClick={() => setLoadAttempt((value) => value + 1)}>{t("attachmentViewer.retry")}</Button></div>;
  if (attachments.length === 0) return null;

  return (
    <div className="attachment-section">
      <div className="attachment-list-heading">
        <div className="attachment-list-title">
          {t("attachments.title")} <span className="attachment-list-count">{attachments.length}</span>
        </div>
        <Button size="sm" variant="ghost" onClick={handleSaveAll} disabled={savingAll || downloadingId !== null}>
          {t(savingAll ? "attachmentViewer.savingAll" : "attachmentViewer.saveAll")}
        </Button>
      </div>
      <div className="attachment-list">
        {attachments.map((attachment) => {
          const { Icon, color } = attachmentVisual(attachment.mime_type, attachment.filename);
          const isDownloading = downloadingId === attachment.id || (savingAll && downloadProgress[attachment.id] != null && downloadProgress[attachment.id] < 100);
          const downloadedPath = downloadedPaths[attachment.id];

          return (
            <div key={attachment.id} className="attachment-card">
              <div className="attachment-icon-tile">
                <Icon size={16} color={color} />
              </div>
              <div className="attachment-card__text">
                <div className="attachment-name" title={attachment.filename}>
                  {canPreviewAttachment(attachment.mime_type) ? (
                    <Button
                      variant="link"
                      className="attachment-open-preview"
                      onClick={() => setPreviewIndex(previewable.findIndex((item) => item.id === attachment.id))}
                      aria-label={t("attachmentViewer.previewNamed", { name: attachment.filename })}
                    >
                      {attachment.filename}
                    </Button>
                  ) : attachment.filename}
                </div>
                <div className="attachment-meta">{formatFileSize(attachment.size)}</div>
              </div>
              <div className="attachment-card__actions">
                {isDownloading && downloadProgress[attachment.id] != null && (
                  <span className="attachment-progress">{downloadProgress[attachment.id]}%</span>
                )}
                <IconButton
                  className="attachment-download"
                  label={t("attachments.downloadNamed", { name: attachment.filename })}
                  tooltip={isDownloading ? t("attachments.downloading") : downloadedPath ? downloadedPath : t("attachments.download")}
                  disabled={downloadingId !== null || savingAll}
                  loading={isDownloading}
                  onClick={() => handleDownload(attachment)}
                  icon={downloadedPath ? <Check size={14} className="attachment-download__done" /> : <Download size={14} />}
                />
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
