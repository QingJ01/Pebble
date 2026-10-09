import { describe, expect, it } from "vitest";
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
} from "lucide-react";
import { attachmentVisual } from "../../src/components/attachmentVisual";

describe("attachmentVisual", () => {
  it("maps media types to distinct icons", () => {
    expect(attachmentVisual("image/png", "photo.png").Icon).toBe(FileImage);
    expect(attachmentVisual("video/mp4", "clip.mp4").Icon).toBe(FileVideo);
    expect(attachmentVisual("audio/mpeg", "song.mp3").Icon).toBe(FileAudio);
  });

  it("falls back to the filename extension when the mime type is generic", () => {
    expect(attachmentVisual("application/octet-stream", "archive.zip").Icon).toBe(FileArchive);
    expect(attachmentVisual("application/octet-stream", "notes.txt").Icon).toBe(FileText);
    expect(attachmentVisual("application/octet-stream", "chart.xlsx").Icon).toBe(FileSpreadsheet);
    expect(attachmentVisual("application/octet-stream", "deck.key").Icon).toBe(Presentation);
    expect(attachmentVisual("application/octet-stream", "clip.mkv").Icon).toBe(FileVideo);
  });

  it("classifies office documents ahead of the generic text fallback", () => {
    expect(attachmentVisual("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "report.xlsx").Icon).toBe(FileSpreadsheet);
    expect(attachmentVisual("text/csv", "rows.csv").Icon).toBe(FileSpreadsheet);
    expect(attachmentVisual("application/vnd.openxmlformats-officedocument.presentationml.presentation", "slides.pptx").Icon).toBe(Presentation);
    expect(attachmentVisual("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "letter.docx").Icon).toBe(FileText);
    expect(attachmentVisual("application/pdf", "doc.pdf").Icon).toBe(FileText);
  });

  it("distinguishes document colors, not only icons", () => {
    expect(attachmentVisual("application/pdf", "doc.pdf").color).toBe("#ef4444");
    expect(attachmentVisual("application/msword", "letter.doc").color).toBe("#3b82f6");
    expect(attachmentVisual("text/plain", "notes.txt").color).toBe("#64748b");
  });

  it("maps code, json and unknown files", () => {
    expect(attachmentVisual("application/json", "payload.json").Icon).toBe(FileJson);
    expect(attachmentVisual("application/octet-stream", "script.ts").Icon).toBe(FileCode);
    expect(attachmentVisual("application/octet-stream", "mystery").Icon).toBe(File);
  });
});
