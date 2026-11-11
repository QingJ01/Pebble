import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ResumeDraftButton from "../../src/components/ResumeDraftButton";

const mocks = vi.hoisted(() => ({ getDraft: vi.fn(), openDraft: vi.fn(), addToast: vi.fn() }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }) }));
vi.mock("../../src/lib/api", () => ({ getDraft: mocks.getDraft }));
vi.mock("../../src/stores/compose.store", () => ({ useComposeStore: { getState: () => ({ openDraft: mocks.openDraft }) } }));
vi.mock("../../src/stores/toast.store", () => ({ useToastStore: { getState: () => ({ addToast: mocks.addToast }) } }));

describe("ResumeDraftButton", () => {
  beforeEach(() => { vi.resetAllMocks(); });
  it.each(["rich", "markdown", "html"])("restores %s mode, recipients, reply headers and staged attachments", async (mode) => {
    mocks.getDraft.mockResolvedValue({
      message: { id: "draft", account_id: "account", to_list: [{ address: "to@example.com" }],
        cc_list: [{ address: "cc@example.com" }], bcc_list: [{ address: "bcc@example.com" }],
        subject: "Saved", body_text: "**Body**", body_html_raw: "<b>Body</b>", in_reply_to: "<parent>" },
      editor_mode: mode,
      attachments: [{ filename: "report.txt", local_path: "/compose_staging/report.txt", size: 12, is_inline: false }],
    });
    render(<ResumeDraftButton messageId="draft" />);
    fireEvent.click(screen.getByRole("button", { name: "Resume draft" }));
    await waitFor(() => expect(mocks.openDraft).toHaveBeenCalledWith(expect.objectContaining({
      id: "draft", accountId: "account", editorMode: mode, to: ["to@example.com"], cc: ["cc@example.com"],
      bcc: ["bcc@example.com"], inReplyTo: "<parent>", rawSource: mode === "html" ? "<b>Body</b>" : "**Body**",
      attachments: [{ name: "report.txt", path: "/compose_staging/report.txt", size: 12 }],
    })));
  });
  it("does not open an incomplete draft after an attachment load error", async () => {
    mocks.getDraft.mockRejectedValue(new Error("Missing attachment"));
    render(<ResumeDraftButton messageId="draft" />);
    fireEvent.click(screen.getByRole("button", { name: "Resume draft" }));
    await waitFor(() => expect(mocks.addToast).toHaveBeenCalled());
    expect(mocks.openDraft).not.toHaveBeenCalled();
  });
});
