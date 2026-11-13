import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { canPreviewAttachment, previewAttachment, saveAllAttachments } from "../../src/lib/attachments-api";
import en from "../../src/locales/attachments.en.json";
import zh from "../../src/locales/attachments.zh.json";
import { createInstance } from "i18next";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => vi.clearAllMocks());

describe("attachment API security boundary", () => {
  it("uses ID-only commands and preserves cancelled/partial Save All results", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ mime_type: "image/png", base64: "safe" }).mockResolvedValueOnce(null);
    expect(await previewAttachment("id")).toEqual({ mime_type: "image/png", base64: "safe" });
    expect(invoke).toHaveBeenNthCalledWith(1, "preview_attachment", { attachmentId: "id" });
    expect(await saveAllAttachments("message")).toBeNull();
    expect(invoke).toHaveBeenNthCalledWith(2, "save_all_attachments", { messageId: "message" });
  });
  it("rejects active IPC MIME responses and excessive payloads", async () => {
    for (const mime_type of ["text/html", "image/svg+xml", "application/javascript", "image/jpeg"]) {
      vi.mocked(invoke).mockResolvedValueOnce({ mime_type, base64: "anything" });
      await expect(previewAttachment("id")).rejects.toThrow("Invalid preview response");
    }
    vi.mocked(invoke).mockResolvedValueOnce({ mime_type: "image/png", base64: "A".repeat(28 * 1024 * 1024 + 1) });
    await expect(previewAttachment("id")).rejects.toThrow("Invalid preview response");
  });
  it("offers generic staged files but not active MIME types", () => {
    for (const mime of ["image/png", "image/jpeg", "image/gif", "image/webp", "application/pdf", "application/octet-stream", "", " IMAGE/PNG ; extra=x"]) expect(canPreviewAttachment(mime)).toBe(true);
    for (const mime of ["image/svg+xml", "text/html", "application/javascript", "application/zip", "image/x-bmp"]) expect(canPreviewAttachment(mime)).toBe(false);
  });
  it("provides matching real English and Chinese resources, including plural page interpolation", async () => {
    expect(Object.keys(en.attachmentViewer).sort()).toEqual(Object.keys(zh.attachmentViewer).sort());
    const i18n = createInstance();
    await i18n.init({ resources: { en: { translation: en }, zh: { translation: zh } }, lng: "en", fallbackLng: "en" });
    expect(i18n.t("attachmentViewer.pageCount", { page: 1, count: 2 })).toBe("Page 1 of 2");
    await i18n.changeLanguage("zh");
    expect(i18n.t("attachmentViewer.pageCount", { page: 1, count: 2 })).toBe("第 1 / 2 页");
    expect(i18n.t("attachmentViewer.saveSummary", { saved: 2, failed: 1 })).toContain("已保存 2 个 · 失败 1 个");
  });
});
