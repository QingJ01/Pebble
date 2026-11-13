import { invoke } from "@tauri-apps/api/core";

export interface AttachmentPreviewData {
  mime_type: "image/png" | "application/pdf";
  base64: string;
}

export interface AttachmentSaveResult {
  attachment_id: string;
  filename: string;
  saved_path: string | null;
  error: string | null;
}

export function canPreviewAttachment(mimeType: string): boolean {
  const mime = mimeType.split(";")[0].trim().toLowerCase();
  return ["", "application/octet-stream", "image/png", "image/jpeg", "image/gif", "image/webp", "application/pdf"].includes(mime);
}

export async function previewAttachment(attachmentId: string): Promise<AttachmentPreviewData> {
  const data = await invoke<AttachmentPreviewData>("preview_attachment", { attachmentId });
  if (!["image/png", "application/pdf"].includes(data.mime_type) || data.base64.length > 28 * 1024 * 1024) {
    throw new Error("Invalid preview response");
  }
  return data;
}

export function saveAllAttachments(messageId: string): Promise<AttachmentSaveResult[] | null> {
  return invoke("save_all_attachments", { messageId });
}
