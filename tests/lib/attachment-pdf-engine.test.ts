// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import { getDocument } from "pdfjs-dist/legacy/build/pdf.mjs";

function documentBytes(): Uint8Array {
  const content = "0 0 1 rg 10 10 50 50 re f";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R /OpenAction 5 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources <<>> /Contents 4 0 R /Annots [6 0 R] >>",
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    "<< /S /JavaScript /JS (globalThis.__pdfScriptExecuted = true) >>",
    "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /URI /URI (https://tracking.invalid/pixel) >> >>",
  ];
  let pdf = "%PDF-1.7\n";
  const offsets = [0];
  objects.forEach((object, index) => { offsets.push(pdf.length); pdf += `${index + 1} 0 obj\n${object}\nendobj\n`; });
  const xref = pdf.length;
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  pdf += offsets.slice(1).map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return new TextEncoder().encode(pdf);
}

afterEach(() => vi.unstubAllGlobals());
describe("real bundled PDF engine (no native UI)", () => {
  it("parses bounded byte-only PDF graphics without executing JavaScript or fetching a remote link", async () => {
    const fetch = vi.fn(() => Promise.reject(new Error("Network forbidden")));
    vi.stubGlobal("fetch", fetch);
    const task = getDocument({ data: documentBytes(), useWorkerFetch: false, useWasm: false, disableFontFace: true, useSystemFonts: false, enableXfa: false, stopAtErrors: true, verbosity: 0 });
    try {
      const document = await task.promise;
      expect(document.numPages).toBe(1);
      const page = await document.getPage(1);
      const operations = await page.getOperatorList({ annotationMode: 0 });
      expect(operations.fnArray.length).toBeGreaterThan(0);
      expect(fetch).not.toHaveBeenCalled();
      expect((globalThis as Record<string, unknown>).__pdfScriptExecuted).toBeUndefined();
    } finally { await task.destroy(); }
  });
  it("rejects corrupt PDFs even when the header and EOF marker look valid", async () => {
    const task = getDocument({ data: new TextEncoder().encode("%PDF-1.7\nthis is not a PDF\n%%EOF"), stopAtErrors: true, verbosity: 0 });
    try { await expect(task.promise).rejects.toThrow(); } finally { await task.destroy(); }
  });
});
