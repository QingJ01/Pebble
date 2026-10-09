import {
  File,
  FileArchive,
  FileAudio,
  FileCode,
  FileImage,
  FileJson,
  FileSpreadsheet,
  FileText,
  FileVideo,
  Presentation,
  type LucideIcon,
} from "lucide-react";

export interface AttachmentVisual {
  Icon: LucideIcon;
  color: string;
}

const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "heic", "tiff", "ico"]);
const VIDEO_EXTS = new Set(["mp4", "mov", "avi", "mkv", "webm", "m4v", "wmv"]);
const AUDIO_EXTS = new Set(["mp3", "wav", "flac", "aac", "ogg", "m4a", "wma"]);
const SPREADSHEET_EXTS = new Set(["xls", "xlsx", "csv", "ods", "tsv"]);
const PRESENTATION_EXTS = new Set(["ppt", "pptx", "key", "odp"]);
const DOCUMENT_EXTS = new Set(["doc", "docx", "rtf", "odt", "pages"]);
const ARCHIVE_EXTS = new Set(["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz"]);
const CODE_EXTS = new Set([
  "js", "jsx", "ts", "tsx", "py", "rs", "go", "java", "c", "cpp", "h", "hpp",
  "html", "css", "scss", "sh", "yml", "yaml", "toml", "xml", "sql",
]);

function extensionOf(filename: string): string {
  const dot = filename.lastIndexOf(".");
  return dot >= 0 ? filename.slice(dot + 1).toLowerCase() : "";
}

export function attachmentVisual(mimeType: string, filename: string): AttachmentVisual {
  const mime = mimeType.toLowerCase();
  const ext = extensionOf(filename);

  if (mime.startsWith("image/") || IMAGE_EXTS.has(ext)) return { Icon: FileImage, color: "#22c55e" };
  if (mime.startsWith("video/") || VIDEO_EXTS.has(ext)) return { Icon: FileVideo, color: "#8b5cf6" };
  if (mime.startsWith("audio/") || AUDIO_EXTS.has(ext)) return { Icon: FileAudio, color: "#ec4899" };
  if (mime.includes("pdf") || ext === "pdf") return { Icon: FileText, color: "#ef4444" };
  if (mime.includes("spreadsheet") || mime.includes("excel") || mime.includes("csv") || SPREADSHEET_EXTS.has(ext)) {
    return { Icon: FileSpreadsheet, color: "#10b981" };
  }
  if (mime.includes("presentation") || mime.includes("powerpoint") || PRESENTATION_EXTS.has(ext)) {
    return { Icon: Presentation, color: "#f97316" };
  }
  if (mime.includes("word") || mime.includes("wordprocessing") || DOCUMENT_EXTS.has(ext)) {
    return { Icon: FileText, color: "#3b82f6" };
  }
  if (
    mime.includes("zip") || mime.includes("archive") || mime.includes("compressed")
    || mime.includes("tar") || mime.includes("rar") || ARCHIVE_EXTS.has(ext)
  ) {
    return { Icon: FileArchive, color: "#f59e0b" };
  }
  if (mime.includes("json") || ext === "json") return { Icon: FileJson, color: "#14b8a6" };
  if (CODE_EXTS.has(ext)) return { Icon: FileCode, color: "#6366f1" };
  if (mime.startsWith("text/") || ext === "txt" || ext === "md" || ext === "log") {
    return { Icon: FileText, color: "#64748b" };
  }
  return { Icon: File, color: "#6b7280" };
}
