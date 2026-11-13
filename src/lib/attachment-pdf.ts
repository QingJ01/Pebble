import { AnnotationMode, getDocument, PDFWorker } from "pdfjs-dist";

const fonts = import.meta.glob<string>("../../node_modules/pdfjs-dist/standard_fonts/*.{pfb,ttf}", { eager: true, query: "?url&no-inline", import: "default" });
const cmaps = import.meta.glob<string>("../../node_modules/pdfjs-dist/cmaps/*.bcmap", { eager: true, query: "?url&no-inline", import: "default" });

export class BundledPdfDataFactory {
  async fetch({ kind, filename }: { kind: string; filename: string }): Promise<Uint8Array> {
    const group = kind === "cMapUrl" ? cmaps : kind === "standardFontDataUrl" ? fonts : {};
    const folder = kind === "cMapUrl" ? "cmaps" : "standard_fonts";
    const key = `../../node_modules/pdfjs-dist/${folder}/${filename}`;
    if (filename.includes("/") || filename.includes("\\") || !Object.prototype.hasOwnProperty.call(group, key)) {
      throw new Error("PDF resource is not bundled");
    }
    const response = await fetch(group[key], { credentials: "omit", redirect: "error" });
    if (!response.ok) throw new Error("Bundled PDF resource is unavailable");
    return new Uint8Array(await response.arrayBuffer());
  }
}

export function loadAttachmentPdf(base64: string) {
  const port = new Worker(new URL("./attachment-pdf.worker.ts", import.meta.url), { type: "module" });
  const worker = PDFWorker.create({ port });
  const data = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
  const task = getDocument({
    data, worker,
    BinaryDataFactory: BundledPdfDataFactory,
    useWorkerFetch: false,
    useWasm: false,
    useSystemFonts: false,
    disableFontFace: true,
    enableXfa: false,
    stopAtErrors: true,
    maxImageSize: 16 * 1024 * 1024,
    canvasMaxAreaInBytes: 64 * 1024 * 1024,
    isImageDecoderSupported: false,
    isOffscreenCanvasSupported: false,
    disableAutoFetch: true,
    disableStream: true,
    disableRange: true,
    verbosity: 0,
  });
  return {
    task,
    destroy: () => {
      void task.destroy().catch(() => {});
      worker.destroy();
      port.terminate();
    },
  };
}

export const previewAnnotationMode = AnnotationMode.DISABLE;
