import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import AttachmentPreview from "../../src/components/AttachmentPreview";
import type { Attachment } from "../../src/lib/api";
import { previewAttachment } from "../../src/lib/attachments-api";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("../../src/lib/attachments-api", () => ({ previewAttachment: vi.fn() }));
vi.mock("../../src/components/AttachmentPdfPreview", () => ({ default: ({ onError }: { onError: () => void }) => <button onClick={onError}>PDF failure</button> }));

const attachments = ["first.png", "second.pdf"].map((filename, index) => ({ id: `a${index}`, filename, mime_type: index ? "application/pdf" : "image/png", message_id: "m1", size: 20, local_path: null, is_inline: false, content_id: null } satisfies Attachment));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

function Viewer({ onClose = vi.fn(), onDownload = vi.fn() }) {
  const [index, setIndex] = useState(0);
  return <AttachmentPreview attachments={attachments} index={index} onNavigate={setIndex} onClose={onClose} onDownload={onDownload} downloading={false} />;
}

describe("AttachmentPreview", () => {
  it("loads only by ID, traps focus, downloads, navigates with arrows and restores focus on close", async () => {
    vi.mocked(previewAttachment).mockResolvedValue({ mime_type: "image/png", base64: "safePNG" });
    const opener = document.createElement("button");
    document.body.append(opener); opener.focus();
    const onClose = vi.fn(); const onDownload = vi.fn();
    const rendered = render(<Viewer onClose={onClose} onDownload={onDownload} />);
    expect(screen.getByRole("dialog").getAttribute("aria-modal")).toBe("true");
    const close = screen.getByRole("button", { name: "attachmentViewer.close" });
    expect(document.activeElement).toBe(close);
    expect(screen.getByRole("status")).toBeTruthy();
    const image = await screen.findByAltText("first.png");
    expect(image.getAttribute("src")).toBe("data:image/png;base64,safePNG");
    fireEvent.load(image);
    expect(image.hasAttribute("hidden")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "attachments.download: first.png" }));
    expect(onDownload).toHaveBeenCalledWith(attachments[0]);
    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "attachmentViewer.next" }));
    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "attachments.download: first.png" }));
    fireEvent.keyDown(document, { key: "ArrowRight" });
    expect(await screen.findByRole("heading", { name: "second.pdf" })).toBeTruthy();
    expect(previewAttachment).toHaveBeenCalledWith("a1");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
    rendered.unmount(); expect(document.activeElement).toBe(opener); opener.remove();
  });

  it("shows actionable native errors and retries without accepting paths", async () => {
    vi.mocked(previewAttachment).mockRejectedValueOnce({ kind: "Validation", message: "File is too large" }).mockResolvedValueOnce({ mime_type: "image/png", base64: "ok" });
    render(<Viewer />);
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByText("File is too large")).toBeTruthy();
    expect(screen.getByText("attachmentViewer.fallback")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "attachmentViewer.retry" }));
    expect(await screen.findByAltText("first.png")).toBeTruthy();
    expect(previewAttachment).toHaveBeenNthCalledWith(2, "a0");
  });

  it("does not show bytes from an earlier attachment when its request resolves late", async () => {
    let resolveFirst!: (value: { mime_type: "image/png"; base64: string }) => void;
    vi.mocked(previewAttachment).mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; })).mockResolvedValueOnce({ mime_type: "application/pdf", base64: "pdf" });
    render(<Viewer />);
    fireEvent.keyDown(document, { key: "ArrowRight" });
    await screen.findByRole("button", { name: "PDF failure" });
    await act(async () => resolveFirst({ mime_type: "image/png", base64: "stale" }));
    expect(screen.queryByAltText("first.png")).toBeNull();
    expect(screen.queryByAltText("second.pdf")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "PDF failure" }));
    await waitFor(() => expect(screen.getByRole("alert")).toBeTruthy());
    expect(screen.queryByRole("button", { name: "PDF failure" })).toBeNull();
  });

  it("does not leak mail shortcuts to the background", async () => {
    vi.mocked(previewAttachment).mockResolvedValue({ mime_type: "image/png", base64: "ok" });
    const background = vi.fn(); document.addEventListener("keydown", background);
    render(<Viewer />);
    await screen.findByAltText("first.png");
    fireEvent.keyDown(document, { key: "Delete" });
    expect(background).not.toHaveBeenCalled();
    document.removeEventListener("keydown", background);
  });
});
