// Install IPC before the initial worker turn dispatches queued document messages.
import "pdfjs-dist/legacy/build/pdf.worker.min.mjs";

// This worker only parses IPC-provided bytes; it must never fetch document resources.
globalThis.fetch = () => Promise.reject(new Error("Network access is disabled in attachment previews"));
export {};
