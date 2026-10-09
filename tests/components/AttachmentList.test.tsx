import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import AttachmentList from "../../src/components/AttachmentList";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(vi.fn())),
}));

vi.mock("../../src/lib/api", () => ({
  listAttachments: vi.fn().mockResolvedValue([
    { id: "a1", message_id: "m1", filename: "photo.png", mime_type: "image/png", size: 2048, is_inline: false },
    { id: "a2", message_id: "m1", filename: "report.xlsx", mime_type: "application/octet-stream", size: 307200, is_inline: false },
    { id: "a3", message_id: "m1", filename: "inline.png", mime_type: "image/png", size: 100, is_inline: true },
  ]),
  downloadAttachment: vi.fn(),
}));

describe("AttachmentList", () => {
  it("renders a color-coded icon tile per attachment and skips inline files", async () => {
    render(<AttachmentList messageId="m1" />);

    const photoName = await screen.findByText("photo.png");
    expect(screen.getByText("report.xlsx")).toBeTruthy();
    expect(screen.queryByText("inline.png")).toBeNull();

    const photoTile = photoName.closest(".attachment-card")?.querySelector(".attachment-icon-tile");
    const reportTile = screen.getByText("report.xlsx").closest(".attachment-card")?.querySelector(".attachment-icon-tile");
    expect(photoTile?.querySelector("svg")?.getAttribute("stroke")).toBe("#22c55e");
    expect(reportTile?.querySelector("svg")?.getAttribute("stroke")).toBe("#10b981");

    expect(screen.getByText("2.0 KB")).toBeTruthy();
    expect(screen.getByText("300.0 KB")).toBeTruthy();
  });

  it("labels download buttons with the file name", async () => {
    render(<AttachmentList messageId="m1" />);

    expect(await screen.findByRole("button", { name: "attachments.download: photo.png" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "attachments.download: report.xlsx" })).toBeTruthy();
  });
});
