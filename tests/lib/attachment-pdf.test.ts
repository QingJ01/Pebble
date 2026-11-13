import { afterEach, describe, expect, it, vi } from "vitest";
import { getDocument, PDFWorker } from "pdfjs-dist/legacy/build/pdf.mjs";
import { BundledPdfDataFactory, loadAttachmentPdf, previewAnnotationMode } from "../../src/lib/attachment-pdf";

vi.mock("pdfjs-dist/legacy/build/pdf.mjs", () => ({ AnnotationMode: { DISABLE: 0 }, getDocument: vi.fn(), PDFWorker: { create: vi.fn() } }));
afterEach(() => { vi.clearAllMocks(); vi.unstubAllGlobals(); });

describe("offline canvas PDF security settings", () => {
  it("bundles a dedicated module worker, only loads byte data and disables active/browser resource features", () => {
    const terminate = vi.fn(); const workerDestroy = vi.fn(); const taskDestroy = vi.fn().mockResolvedValue(undefined);
    const constructor = vi.fn(function () { return { terminate }; });
    vi.stubGlobal("Worker", constructor);
    vi.mocked(PDFWorker.create).mockReturnValue({ destroy: workerDestroy } as never);
    vi.mocked(getDocument).mockReturnValue({ destroy: taskDestroy } as never);
    const pdf = loadAttachmentPdf(btoa("%PDF-1.7"));
    expect(constructor).toHaveBeenCalledWith(expect.any(URL), { type: "module" });
    const options = vi.mocked(getDocument).mock.calls[0][0] as Record<string, unknown>;
    expect(options.data).toEqual(Uint8Array.from(new TextEncoder().encode("%PDF-1.7")));
    expect(options.url).toBeUndefined();
    expect(options).toMatchObject({ useWorkerFetch: false, useWasm: false, disableFontFace: true, useSystemFonts: false, enableXfa: false, disableStream: true, disableRange: true, disableAutoFetch: true, maxImageSize: 16 * 1024 * 1024, canvasMaxAreaInBytes: 64 * 1024 * 1024 });
    expect(previewAnnotationMode).toBe(0);
    pdf.destroy();
    expect(terminate).toHaveBeenCalledOnce(); expect(workerDestroy).toHaveBeenCalledOnce(); expect(taskDestroy).toHaveBeenCalledOnce();
  });

  it("only fetches build-time bundled fonts/CMaps and never accepts document resource URLs, traversal or unknown files", async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: true, arrayBuffer: () => Promise.resolve(new Uint8Array([1]).buffer) });
    vi.stubGlobal("fetch", fetch);
    const factory = new BundledPdfDataFactory();
    for (const filename of ["https://tracking.invalid/pixel", "../FoxitSymbol.pfb", "..\\FoxitSymbol.pfb", "__proto__", "unbundled.pfb"]) {
      await expect(factory.fetch({ kind: "standardFontDataUrl", filename })).rejects.toThrow("not bundled");
    }
    await expect(factory.fetch({ kind: "remote", filename: "FoxitSymbol.pfb" })).rejects.toThrow("not bundled");
    expect(fetch).not.toHaveBeenCalled();
    expect(await factory.fetch({ kind: "standardFontDataUrl", filename: "FoxitSymbol.pfb" })).toEqual(new Uint8Array([1]));
    expect(fetch).toHaveBeenCalledWith(expect.stringMatching(/FoxitSymbol/), { credentials: "omit", redirect: "error" });
  });
});
