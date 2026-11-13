import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createInstance } from "i18next";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MailArchiveView from "../../../src/features/mail-archive/MailArchiveView";
import en from "../../../src/locales/mail-archive.en.json";
import zh from "../../../src/locales/mail-archive.zh.json";

const mocks = vi.hoisted(() => ({
  listMailArchives: vi.fn(), createMailArchive: vi.fn(), listArchiveFolders: vi.fn(), createArchiveFolder: vi.fn(),
  listArchiveMessages: vi.fn(), readArchiveMessage: vi.fn(), updateArchiveMessage: vi.fn(), deleteArchiveMessage: vi.fn(),
  chooseArchiveFiles: vi.fn(), importMailArchive: vi.fn(), cancelArchiveJob: vi.fn(), exportMailArchive: vi.fn(), saveArchiveAttachment: vi.fn(), confirm: vi.fn(),
  listener: null as null | ((event: { payload: unknown }) => void),
}));
vi.mock("../../../src/lib/mail-archive-api", () => mocks);
vi.mock("@tauri-apps/api/event", () => ({ listen: (_name: string, listener: (event: { payload: unknown }) => void) => { mocks.listener = listener; return Promise.resolve(() => {}); } }));
vi.mock("../../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({ data: [{ id: "account", email: "owner@example.com" }] }),
  useFoldersQuery: () => ({ data: [{ id: "inbox", name: "Inbox" }] }),
}));
vi.mock("../../../src/stores/mail.store", () => ({ useMailStore: (selector: (s: object) => unknown) => selector({ activeAccountId: "account", selectedMessageId: null, selectedMessageIds: new Set(["remote-selected"]) }) }));
vi.mock("../../../src/stores/confirm.store", () => ({ useConfirmStore: { getState: () => ({ confirm: mocks.confirm }) } }));

async function renderView(language = "en") {
  const i18n = createInstance();
  await i18n.init({ lng: language, fallbackLng: "en", resources: { en: { translation: en }, zh: { translation: zh } }, interpolation: { escapeValue: false } });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  render(<I18nextProvider i18n={i18n}><QueryClientProvider client={client}><MailArchiveView /></QueryClientProvider></I18nextProvider>);
  await screen.findByRole("button", { name: /Subject A — sender@example.com/ });
  return client;
}

describe("Local mail archive browser", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.listMailArchives.mockResolvedValue([{ id: "archive", account_id: "account", account_label: "owner@example.com", name: "Imported mail" }, { id: "other", account_id: "account", account_label: "owner@example.com", name: "Other archive" }]);
    mocks.listArchiveFolders.mockResolvedValue([{ id: "local", archive_id: "archive", name: "Imported" }, { id: "local-other", archive_id: "archive", name: "Other local folder" }]);
    mocks.listArchiveMessages.mockResolvedValue([{ id: "message", folder_id: "local", subject: "Subject A", sender: "sender@example.com", date: 1, is_read: false, is_starred: false }]);
    mocks.readArchiveMessage.mockResolvedValue({ id: "message", folder_id: "local", fingerprint: "a".repeat(64), is_read: false, is_starred: false, metadata: { subject: "Subject A", from_name: "Sender", from_address: "sender@example.com", to_list: [{ address: "to@example.com" }], cc_list: [], bcc_list: [{ address: "hidden@example.com" }], date: 1, has_html: true, body_text: '<img src="https://tracker.invalid/pixel" onerror="evil()"><script>evil()</script>', attachments: [{ filename: "../report.html", size: 4, is_inline: true, content_id: "cid", sha256: "b".repeat(64) }] } });
    mocks.chooseArchiveFiles.mockResolvedValue({ id: "selection", files: [{ name: "mail.mbox", bytes: 1024 }] });
    mocks.importMailArchive.mockResolvedValue({ processed: 3, imported: 1, duplicates: 1, failed: 1, cancelled: false, errors: ["File 1, message 2: invalid MIME"] });
    mocks.exportMailArchive.mockResolvedValue({ processed: 1, imported: 1, duplicates: 0, failed: 0, cancelled: false, errors: [] });
    mocks.confirm.mockResolvedValue(true);
    mocks.cancelArchiveJob.mockResolvedValue(undefined);
  });
  afterEach(() => cleanup());

  it("states local-only/search boundaries and renders untrusted mail strictly as text", async () => {
    await renderView();
    expect(screen.getByText(en.mailArchive.localOnly)).toBeTruthy();
    expect(screen.getByText(en.mailArchive.searchScope)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Subject A — sender@example.com/ }));
    const message = await screen.findByRole("article", { name: "Archived message" });
    expect(message.querySelector("img,script,iframe")).toBeNull();
    expect(within(message).getByText(/<img src=/)).toBeTruthy();
    expect(within(message).getByText(/Bcc: hidden@example.com/)).toBeTruthy();
    expect(within(message).getByText(`SHA256: ${"b".repeat(64)}`)).toBeTruthy();
    fireEvent.click(within(message).getByRole("button", { name: "Save attachment…" }));
    await waitFor(() => expect(mocks.saveArchiveAttachment).toHaveBeenCalledWith("archive", "message", 0));
  });

  it("requires validation and explicit confirmation before a local import, reporting duplicates and failures", async () => {
    await renderView();
    const importButton = screen.getByRole("button", { name: "Import into local archive" }) as HTMLButtonElement;
    expect(importButton.disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Choose EML / MBOX files…" }));
    await screen.findByText("mail.mbox (1.0 KiB)");
    fireEvent.click(screen.getByRole("button", { name: "Validate and summarize" }));
    await screen.findByRole("heading", { name: "Validation only — nothing imported" });
    expect(mocks.importMailArchive).toHaveBeenCalledWith(expect.any(String), "selection", "archive", "local", true);
    expect(mocks.confirm).not.toHaveBeenCalled();
    expect(screen.getByText("File 1, message 2: invalid MIME")).toBeTruthy();
    mocks.confirm.mockResolvedValueOnce(false);
    fireEvent.click(importButton);
    await waitFor(() => expect(mocks.confirm).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(importButton.disabled).toBe(false));
    expect(mocks.importMailArchive).toHaveBeenCalledTimes(1);
    fireEvent.click(importButton);
    await screen.findByRole("heading", { name: "Import summary" });
    expect(mocks.importMailArchive).toHaveBeenLastCalledWith(expect.any(String), "selection", "archive", "local", false);
  });

  it("clears validation when destination changes and scopes archive exports separately from cached mailbox exports", async () => {
    await renderView();
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Subject A" }));
    fireEvent.click(screen.getByRole("button", { name: "Export selected archive messages…" }));
    await waitFor(() => expect(mocks.exportMailArchive).toHaveBeenCalledWith(expect.any(String), { kind: "archive", archive_id: "archive", folder_id: null, message_ids: ["message"] }, "eml"));
    fireEvent.change(screen.getByLabelText("Export format"), { target: { value: "mbox" } });
    fireEvent.click(screen.getByRole("button", { name: "Export entire archive folder…" }));
    await waitFor(() => expect(mocks.exportMailArchive).toHaveBeenLastCalledWith(expect.any(String), { kind: "archive", archive_id: "archive", folder_id: "local", message_ids: [] }, "mbox"));
    fireEvent.click(screen.getByRole("button", { name: "Export mailbox selection (1) as mbox…" }));
    await waitFor(() => expect(mocks.exportMailArchive).toHaveBeenLastCalledWith(expect.any(String), { kind: "mailbox", account_id: "account", folder_id: null, message_ids: ["remote-selected"] }, "mbox"));
    fireEvent.change(screen.getByLabelText("Local archive"), { target: { value: "other" } });
    expect((screen.getByRole("button", { name: "Export selected archive messages…" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("performs local flags/moves, confirms deletion, and clears deleted message selection", async () => {
    await renderView();
    fireEvent.click(screen.getByRole("button", { name: /Subject A — sender@example.com/ }));
    await screen.findByRole("article", { name: "Archived message" });
    fireEvent.change(screen.getByLabelText("Move within this local archive"), { target: { value: "local-other" } });
    await waitFor(() => expect(mocks.updateArchiveMessage).toHaveBeenCalledWith("archive", "message", "local-other", false, false));
    await waitFor(() => expect(screen.queryByRole("article")).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: /Subject A — sender@example.com/ }));
    await screen.findByRole("article", { name: "Archived message" });
    fireEvent.click(screen.getByRole("button", { name: "Delete archived message" }));
    await waitFor(() => expect(mocks.deleteArchiveMessage).toHaveBeenCalledWith("archive", "message"));
    await waitFor(() => expect(screen.queryByRole("article")).toBeNull());
    expect(mocks.confirm).toHaveBeenCalledWith(expect.objectContaining({ destructive: true, message: en.mailArchive.deleteWarning }));
  });

  it("shows translated Chinese scope, errors and progress, and cancels only its active job", async () => {
    await renderView("zh");
    expect(screen.getByText(zh.mailArchive.localOnly)).toBeTruthy();
    expect(screen.getByText(zh.mailArchive.searchScope)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "选择 EML / MBOX 文件…" }));
    await screen.findByText("mail.mbox (1.0 KiB)");
    let finish!: (report: object) => void;
    mocks.importMailArchive.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    fireEvent.click(screen.getByRole("button", { name: "验证并查看摘要" }));
    await waitFor(() => expect(mocks.importMailArchive).toHaveBeenCalled());
    const jobId = mocks.importMailArchive.mock.calls[0][0];
    act(() => mocks.listener?.({ payload: { job_id: jobId, processed: 37, imported: 35, duplicates: 2, failed: 0 } }));
    await screen.findByText(/正在处理邮件… 37/);
    fireEvent.click(screen.getByRole("button", { name: "当前邮件完成后取消" }));
    expect(mocks.cancelArchiveJob).toHaveBeenCalledWith(jobId);
    await act(async () => finish({ processed: 37, imported: 35, duplicates: 2, failed: 0, cancelled: true, errors: [] }));
    await screen.findByText(zh.mailArchive.cancelled);
    expect((screen.getByRole("button", { name: "导入本地归档" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("has matching EN/ZH namespaces without missing or untranslated keys", () => {
    const keys = (value: object, prefix = ""): string[] => Object.entries(value).flatMap(([key, item]) => typeof item === "object" ? keys(item, `${prefix}${key}.`) : [`${prefix}${key}`]).sort();
    expect(keys(en)).toEqual(keys(zh));
    expect(Object.keys(en)).toEqual(["mailArchive"]);
  });
});
