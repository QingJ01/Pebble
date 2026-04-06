import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { File, FileText, Image, FileArchive, Film, Music, Download, Loader, Check } from "lucide-react";
import { listAttachments, downloadAttachment } from "@/lib/api";
import type { Attachment } from "@/lib/api";
import { sanitizeFilename } from "@/lib/sanitizeFilename";
import { useToastStore } from "@/stores/toast.store";

interface Props {
  messageId: string;
}

function getMimeIcon(mimeType: string) {
  if (mimeType.startsWith("image/")) return Image;
  if (mimeType.startsWith("video/")) return Film;
  if (mimeType.startsWith("audio/")) return Music;
  if (mimeType.includes("zip") || mimeType.includes("archive") || mimeType.includes("compressed") || mimeType.includes("tar") || mimeType.includes("rar")) return FileArchive;
  if (mimeType.includes("text") || mimeType.includes("pdf") || mimeType.includes("document") || mimeType.includes("word")) return FileText;
  return File;
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export default function AttachmentList({ messageId }: Props) {
  const { t } = useTranslation();
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [loading, setLoading] = useState(true);
  const [downloadingId, setDownloadingId] = useState<string | null>(null);
  const [downloadedPaths, setDownloadedPaths] = useState<Record<string, string>>({});

  useEffect(() => {
    let cancelled = false;
    setLoading(true);

    listAttachments(messageId)
      .then((list) => {
        if (!cancelled) {
          setAttachments(list.filter((a) => !a.is_inline));
        }
      })
      .catch(() => {
        if (!cancelled) setAttachments([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [messageId]);

  async function handleDownload(attachment: Attachment) {
    setDownloadingId(attachment.id);
    try {
      const { downloadDir } = await import("@tauri-apps/api/path");
      const dir = await downloadDir();
      const safeName = sanitizeFilename(attachment.filename);
      const savePath = `${dir}/${safeName}`;
      await downloadAttachment(attachment.id, savePath);
      setDownloadedPaths((prev) => ({ ...prev, [attachment.id]: savePath }));
    } catch (err) {
      console.error("Failed to download attachment:", err);
      useToastStore.getState().addToast({ message: t("attachments.downloadFailed", "Failed to download attachment"), type: "error" });
    } finally {
      setDownloadingId(null);
    }
  }

  if (loading) return null;
  if (attachments.length === 0) return null;

  return (
    <div
      style={{
        padding: "12px 16px",
        borderTop: "1px solid var(--color-border)",
        backgroundColor: "var(--color-bg)",
      }}
    >
      <div
        style={{
          fontSize: "12px",
          fontWeight: "600",
          color: "var(--color-text-secondary)",
          marginBottom: "8px",
          textTransform: "uppercase",
          letterSpacing: "0.5px",
        }}
      >
        {t("attachments.title")} ({attachments.length})
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: "4px" }}>
        {attachments.map((attachment) => {
          const Icon = getMimeIcon(attachment.mime_type);
          const isDownloading = downloadingId === attachment.id;

          return (
            <div
              key={attachment.id}
              style={{
                display: "flex",
                alignItems: "center",
                gap: "8px",
                padding: "6px 8px",
                borderRadius: "6px",
                backgroundColor: "var(--color-bg-hover)",
                fontSize: "13px",
              }}
            >
              <Icon size={16} color="var(--color-text-secondary)" style={{ flexShrink: 0 }} />
              <span
                style={{
                  flex: 1,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  color: "var(--color-text-primary)",
                }}
              >
                {attachment.filename}
              </span>
              <span
                style={{
                  fontSize: "11px",
                  color: "var(--color-text-secondary)",
                  flexShrink: 0,
                }}
              >
                {formatFileSize(attachment.size)}
              </span>
              <button
                onClick={() => handleDownload(attachment)}
                disabled={isDownloading}
                title={isDownloading ? t("attachments.downloading") : downloadedPaths[attachment.id] ? downloadedPaths[attachment.id] : t("attachments.download")}
                style={{
                  background: "none",
                  border: "none",
                  cursor: isDownloading ? "default" : "pointer",
                  padding: "2px",
                  borderRadius: "4px",
                  color: "var(--color-text-secondary)",
                  display: "flex",
                  alignItems: "center",
                  flexShrink: 0,
                  opacity: isDownloading ? 0.5 : 1,
                }}
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
          );
        })}
      </div>
    </div>
  );
}
