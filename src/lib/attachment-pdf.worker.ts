// This worker only parses IPC-provided bytes; it must never fetch document resources.
globalThis.fetch = () => Promise.reject(new Error("Network access is disabled in attachment previews"));
void import("pdfjs-dist/build/pdf.worker.min.mjs");
export {};
