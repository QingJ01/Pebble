import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import AttachmentPdfPreview from "../../src/components/AttachmentPdfPreview";
import { loadAttachmentPdf } from "../../src/lib/attachment-pdf";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("../../src/lib/attachment-pdf", () => ({ loadAttachmentPdf: vi.fn(), previewAnnotationMode: 0 }));
afterEach(() => { cleanup(); vi.clearAllMocks(); vi.useRealTimers(); });

function pdfDocument() {
  const render = vi.fn(() => ({ promise: Promise.resolve(), cancel: vi.fn() }));
  const page = { getViewport: ({ scale }: { scale: number }) => ({ width: 10000 * scale, height: 10000 * scale }), render, cleanup: vi.fn() };
  const document = { numPages: 2, getPage: vi.fn().mockResolvedValue(page) };
  return { document, render, page };
}

describe("AttachmentPdfPreview", () => {
  it("renders one bounded canvas page at a time, supports page controls and destroys parser on close", async () => {
    const { document, render: draw, page } = pdfDocument();
    const destroy = vi.fn(); const onError = vi.fn();
    vi.mocked(loadAttachmentPdf).mockReturnValue({ task: { promise: Promise.resolve(document) }, destroy } as never);
    const view = render(<AttachmentPdfPreview base64="pdf" onError={onError} />);
    const canvas = await screen.findByRole("img", { name: "attachmentViewer.pdfPage" });
    expect(canvas.getAttribute("width")).toBe("1600");
    expect(canvas.getAttribute("height")).toBe("1600");
    expect(draw).toHaveBeenCalledWith(expect.objectContaining({ annotationMode: 0, canvas }));
    expect(document.getPage).toHaveBeenCalledWith(1);
    expect(page.cleanup).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "attachmentViewer.nextPage" }));
    await waitFor(() => expect(document.getPage).toHaveBeenCalledWith(2));
    await waitFor(() => expect(screen.getByRole("button", { name: "attachmentViewer.nextPage" }).hasAttribute("disabled")).toBe(true));
    expect(onError).not.toHaveBeenCalled();
    view.unmount(); expect(destroy).toHaveBeenCalledOnce();
  });
  it("reports corrupt/encrypted PDFs and caps document page counts", async () => {
    const onError = vi.fn();
    vi.mocked(loadAttachmentPdf).mockReturnValueOnce({ task: { promise: Promise.reject(new Error("invalid or password protected")) }, destroy: vi.fn() } as never);
    const view = render(<AttachmentPdfPreview base64="corrupt" onError={onError} />);
    await waitFor(() => expect(onError).toHaveBeenCalledOnce());
    vi.mocked(loadAttachmentPdf).mockReturnValueOnce({ task: { promise: Promise.resolve({ numPages: 2001 }) }, destroy: vi.fn() } as never);
    view.rerender(<AttachmentPdfPreview base64="tooMany" onError={onError} />);
    await waitFor(() => expect(onError).toHaveBeenCalledTimes(2));
  });
  it("shows an actionable error instead of crashing on unavailable worker or rendering failures", async () => {
    const onError = vi.fn();
    vi.mocked(loadAttachmentPdf).mockImplementationOnce(() => { throw new Error("worker unavailable"); });
    const view = render(<AttachmentPdfPreview base64="worker" onError={onError} />);
    expect(onError).toHaveBeenCalledOnce();
    const { document } = pdfDocument(); document.getPage.mockRejectedValue(new Error("corrupt page"));
    vi.mocked(loadAttachmentPdf).mockReturnValueOnce({ task: { promise: Promise.resolve(document) }, destroy: vi.fn() } as never);
    view.rerender(<AttachmentPdfPreview base64="page" onError={onError} />);
    await waitFor(() => expect(onError).toHaveBeenCalledTimes(2));
  });
});
