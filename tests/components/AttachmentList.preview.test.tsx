import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import AttachmentList from "../../src/components/AttachmentList";
import { downloadAttachment, listAttachments } from "../../src/lib/api";
import { previewAttachment, saveAllAttachments } from "../../src/lib/attachments-api";
import { useToastStore } from "../../src/stores/toast.store";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string, options?: Record<string, unknown>) => options ? key + JSON.stringify(options) : key }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(vi.fn())) }));
vi.mock("@tauri-apps/api/path", () => ({ downloadDir: vi.fn(() => Promise.resolve("/downloads")) }));
vi.mock("../../src/lib/api", () => ({ listAttachments: vi.fn(), downloadAttachment: vi.fn() }));
vi.mock("../../src/lib/attachments-api", async (original) => ({ ...await original<typeof import("../../src/lib/attachments-api")>(), previewAttachment: vi.fn(), saveAllAttachments: vi.fn() }));

const attachments = [
  { id: "png", message_id: "m1", filename: "photo.png", mime_type: "image/png", size: 10, is_inline: false },
  { id: "generic", message_id: "m1", filename: "unknown.dat", mime_type: "application/octet-stream", size: 20, is_inline: false },
  { id: "svg", message_id: "m1", filename: "unsafe.svg", mime_type: "image/svg+xml", size: 30, is_inline: false },
  { id: "cid", message_id: "m1", filename: "inline.png", mime_type: "image/png", size: 40, is_inline: true },
];
beforeEach(() => { vi.mocked(listAttachments).mockResolvedValue(attachments as never); vi.mocked(previewAttachment).mockResolvedValue({ mime_type: "image/png", base64: "ok" }); });
afterEach(() => { cleanup(); vi.clearAllMocks(); useToastStore.setState({ toasts: [] }); });

describe("AttachmentList preview and Save All", () => {
  it("opens ordinary previewable/generic files without exposing active MIME files or CID editor paths", async () => {
    render(<AttachmentList messageId="m1" />);
    fireEvent.click(await screen.findByRole("button", { name: "attachmentViewer.preview: unknown.dat" }));
    expect(await screen.findByRole("dialog")).toBeTruthy();
    expect(previewAttachment).toHaveBeenCalledWith("generic");
    expect(screen.queryByRole("button", { name: "attachmentViewer.preview: unsafe.svg" })).toBeNull();
    expect(screen.queryByText("inline.png")).toBeNull();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("reports every success/failure from Save All, disables competing saves and handles cancellation", async () => {
    let resolve!: (value: unknown) => void;
    vi.mocked(saveAllAttachments).mockImplementationOnce(() => new Promise((r) => { resolve = r; }) as never).mockResolvedValueOnce(null);
    render(<AttachmentList messageId="m1" />);
    const save = await screen.findByRole("button", { name: "attachmentViewer.saveAll" });
    fireEvent.click(save);
    expect(screen.getByRole("button", { name: "attachmentViewer.savingAll" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "attachments.download: photo.png" }).hasAttribute("disabled")).toBe(true);
    await act(async () => resolve([
      { attachment_id: "png", filename: "photo.png", saved_path: "/picked/photo (1).png", error: null },
      { attachment_id: "generic", filename: "unknown.dat", saved_path: null, error: "Source missing; sync and retry" },
    ]));
    expect(screen.getByText("photo.png: /picked/photo (1).png")).toBeTruthy();
    expect(screen.getByText("unknown.dat: Source missing; sync and retry")).toBeTruthy();
    expect(screen.getByText('attachmentViewer.saveSummary{"saved":1,"failed":1}')).toBeTruthy();
    expect(saveAllAttachments).toHaveBeenCalledWith("m1");
    fireEvent.click(screen.getByRole("button", { name: "attachmentViewer.saveAll" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "attachmentViewer.saveAll" }).hasAttribute("disabled")).toBe(false));
    expect(screen.queryByText(/Source missing/)).toBeNull();
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it("preserves individual safe download and shows list/save errors with retry", async () => {
    vi.mocked(downloadAttachment).mockResolvedValue("/downloads/photo (1).png");
    vi.mocked(saveAllAttachments).mockRejectedValue({ message: "Permission denied" });
    const view = render(<AttachmentList messageId="m1" />);
    fireEvent.click(await screen.findByRole("button", { name: "attachments.download: photo.png" }));
    await waitFor(() => expect(downloadAttachment).toHaveBeenCalledWith("png", "/downloads/photo.png"));
    await waitFor(() => expect(screen.getByRole("button", { name: "attachments.download: photo.png" }).getAttribute("title")).toBe("/downloads/photo (1).png"));
    fireEvent.click(screen.getByRole("button", { name: "attachmentViewer.saveAll" }));
    await waitFor(() => expect(useToastStore.getState().toasts[0]?.message).toContain("Permission denied"));
    vi.mocked(listAttachments).mockRejectedValueOnce(new Error("offline"));
    view.rerender(<AttachmentList messageId="other" />);
    expect(await screen.findByRole("alert")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "attachmentViewer.retry" }));
    expect(await screen.findByText("photo.png")).toBeTruthy();
  });

  it("closes previews immediately when selection changes and ignores late Save All results", async () => {
    let resolve!: (value: unknown) => void;
    vi.mocked(saveAllAttachments).mockImplementationOnce(() => new Promise((r) => { resolve = r; }) as never);
    const view = render(<AttachmentList messageId="m1" />);
    fireEvent.click(await screen.findByRole("button", { name: "attachmentViewer.saveAll" }));
    fireEvent.click(screen.getByRole("button", { name: "attachmentViewer.preview: photo.png" }));
    expect(await screen.findByRole("dialog")).toBeTruthy();
    vi.mocked(listAttachments).mockResolvedValueOnce([]);
    view.rerender(<AttachmentList messageId="m2" />);
    expect(screen.queryByRole("dialog")).toBeNull();
    await act(async () => resolve([{ attachment_id: "png", filename: "photo.png", saved_path: "/old/selection.png", error: null }]));
    expect(screen.queryByText(/selection.png/)).toBeNull();
  });
});
