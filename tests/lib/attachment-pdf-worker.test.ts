// @vitest-environment node
import { Worker } from "node:worker_threads";
import { fileURLToPath } from "node:url";
import { build } from "vite";
import { describe, expect, it } from "vitest";
import { getDocument, PDFWorker } from "pdfjs-dist/legacy/build/pdf.mjs";

describe("production PDF worker startup", () => {
  it("handles document requests sent before its bundled module finishes loading", async () => {
    const result = await build({
      configFile: false,
      logLevel: "silent",
      build: {
        write: false,
        target: "es2022",
        lib: { entry: fileURLToPath(new URL("../../src/lib/attachment-pdf.worker.ts", import.meta.url)), formats: ["es"] },
      },
    });
    const output = (Array.isArray(result) ? result[0] : result) as { output: { type: string; code?: string }[] };
    const chunks = output.output.filter((entry) => entry.type === "chunk");
    expect(chunks).toHaveLength(1);
    const code = chunks[0].code!;
    // Model browser module workers: queue until entry evaluation, not until
    // fire-and-forget dynamic imports complete; late message listeners lose IPC.
    const native = new Worker(`
      const { parentPort, workerData } = require("node:worker_threads");
      const listeners = new Set(), queued = [];
      let initialized = false;
      const dispatch = data => { for (const fn of listeners) fn({ data }); };
      parentPort.on("message", data => initialized ? dispatch(data) : queued.push(data));
      globalThis.self = globalThis;
      globalThis.onmessage = null;
      globalThis.postMessage = (data, transfer) => parentPort.postMessage(data, transfer);
      globalThis.addEventListener = (type, fn, options) => {
        if (type !== "message") return;
        listeners.add(fn);
        options?.signal?.addEventListener("abort", () => listeners.delete(fn));
      };
      globalThis.process = undefined;
      import("data:text/javascript;base64," + Buffer.from(workerData).toString("base64")).then(() => {
        initialized = true;
        for (const data of queued) dispatch(data);
      }).catch(error => { throw error; });
    `, { eval: true, workerData: code });
    const port = {
      postMessage: (data: unknown, transfer: Transferable[]) => native.postMessage(data, transfer as ArrayBuffer[]),
      addEventListener: (_type: string, fn: (event: { data: unknown }) => void, options?: { signal?: AbortSignal }) => {
        const listener = (data: unknown) => fn({ data });
        native.on("message", listener);
        options?.signal?.addEventListener("abort", () => native.off("message", listener));
      },
    };
    const worker = PDFWorker.create({ port: port as unknown as globalThis.Worker });
    const data = new TextEncoder().encode("%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Resources<<>>>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF");
    const task = getDocument({ data, worker, useWorkerFetch: false, useWasm: false, verbosity: 0 });
    try {
      const pdf = await task.promise;
      expect(pdf.numPages).toBe(1);
      expect((await pdf.getPage(1)).getViewport({ scale: 1 }).width).toBe(100);
    } finally {
      await task.destroy();
      worker.destroy();
      await native.terminate();
    }
  }, 30000);
});
