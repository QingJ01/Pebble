import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ComposeView from "../../../src/features/compose/ComposeView";
import { useMailStore } from "../../../src/stores/mail.store";
import { queryClient } from "../../../src/lib/query-client";
import type { DraftData } from "../../../src/hooks/useComposeDraft";
import {
  cleanupStagedComposeAttachment,
  deleteDraft,
  stageComposeAttachment,
} from "../../../src/lib/api";
import { queueControlledSend, restoreControlledSend } from "../../../src/lib/send-controls-api";

vi.mock("../../../src/features/compose/ScheduledSendsView", () => ({
  useControlledSends: () => ({ data: [], refetch: vi.fn() }),
}));
vi.mock("../../../src/lib/send-controls-api", () => ({
  queueControlledSend: vi.fn(), restoreControlledSend: vi.fn(), deleteControlledSend: vi.fn().mockResolvedValue(undefined),
}));

const mocks = vi.hoisted(() => ({
  mutate: vi.fn(),
  closeCompose: vi.fn(),
  setComposeDirty: vi.fn(),
  addToast: vi.fn(),
  loadDraftFromStorage: vi.fn(),
  cancelPendingDraftSaves: vi.fn(),
  flushDraftSave: vi.fn(),
  resumeDraftSaves: vi.fn(),
  confirmCloseCompose: vi.fn(),
  cancelCloseCompose: vi.fn(),
  showComposeLeaveConfirm: false,
  quotedReplyHtml: "",
  composeDraft: null as DraftData | null,
  accountsQuery: {
    data: [{ id: "account-1", email: "me@example.com", display_name: "Me" }],
    isLoading: false,
    isSuccess: true,
    isError: false,
  },
  recipients: {
    to: ["to@example.com"] as string[],
    cc: [] as string[],
    bcc: [] as string[],
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (_key: string, fallback?: unknown) => typeof fallback === "string" ? fallback : _key, i18n: { language: "en" } }),
}));

vi.mock("../../../src/stores/compose.store", () => ({
  useComposeStore: Object.assign(
    (selector: (state: {
      composeMode: string;
      composeReplyTo: null;
      closeCompose: () => void;
      showComposeLeaveConfirm: boolean;
      confirmCloseCompose: () => void;
      cancelCloseCompose: () => void;
    }) => unknown) =>
      selector({
        composeMode: "new",
        composeDraft: mocks.composeDraft,
        composeReplyTo: null,
        closeCompose: mocks.closeCompose,
        showComposeLeaveConfirm: mocks.showComposeLeaveConfirm,
        confirmCloseCompose: mocks.confirmCloseCompose,
        cancelCloseCompose: mocks.cancelCloseCompose,
      }),
    {
      getState: () => ({ setComposeDirty: mocks.setComposeDirty }),
    },
  ),
}));

vi.mock("../../../src/hooks/queries", () => ({
  useAccountsQuery: () => mocks.accountsQuery,
}));

vi.mock("../../../src/hooks/mutations", () => ({
  useSendEmailMutation: () => ({
    isPending: false,
    mutate: mocks.mutate,
  }),
}));

vi.mock("../../../src/hooks/useComposeRecipients", () => ({
  useComposeRecipients: () => ({
    fromAccountId: "account-1",
    setFromAccountId: vi.fn(),
    to: mocks.recipients.to,
    setTo: vi.fn(),
    cc: mocks.recipients.cc,
    setCc: vi.fn(),
    bcc: mocks.recipients.bcc,
    setBcc: vi.fn(),
    showCc: false,
    setShowCc: vi.fn(),
    showBcc: false,
    setShowBcc: vi.fn(),
  }),
}));

vi.mock("../../../src/hooks/useComposeDraft", () => ({
  useComposeDraft: () => ({
    draftIdRef: { current: "draft-1" },
    draftIdsByAccountRef: { current: { "account-1": "draft-1" } },
    cancelPendingDraftSaves: mocks.cancelPendingDraftSaves,
    flushDraftSave: mocks.flushDraftSave,
    resumeDraftSaves: mocks.resumeDraftSaves,
  }),
  loadDraftFromStorage: mocks.loadDraftFromStorage,
  clearDraftStorage: vi.fn(),
}));

vi.mock("../../../src/hooks/useComposeEditor", () => ({
  useComposeEditor: () => ({
    editor: {
      getHTML: () => "<p>Hello</p>",
      getText: () => "Hello",
      commands: { setContent: vi.fn() },
    },
    editorMode: "rich",
    rawSource: "",
    setRawSource: vi.fn(),
    richTextHtml: "<p>Hello</p>",
    htmlPreview: false,
    setHtmlPreview: vi.fn(),
    switchMode: vi.fn(),
    textareaRef: { current: null },
    quotedReplyHtml: mocks.quotedReplyHtml,
  }),
  appendReplyQuoteHtml: (bodyHtml: string, quotedReplyHtml: string) =>
    quotedReplyHtml.trim() ? `${bodyHtml}<br/><br/>${quotedReplyHtml.trim()}` : bodyHtml,
}));

vi.mock("../../../src/components/ContactAutocomplete", () => ({
  default: ({
    id,
    name,
    ariaLabelledBy,
    inputValue,
    placeholder,
    onInputValueChange,
  }: {
    id?: string;
    name?: string;
    ariaLabelledBy?: string;
    inputValue?: string;
    placeholder?: string;
    onInputValueChange?: (value: string) => void;
  }) => (
    <input
      id={id}
      name={name}
      role="combobox"
      aria-labelledby={ariaLabelledBy}
      value={inputValue ?? ""}
      placeholder={placeholder}
      onChange={(event) => onInputValueChange?.(event.currentTarget.value)}
    />
  ),
}));

vi.mock("../../../src/features/compose/ComposeToolbar", () => ({
  ModeButton: ({ label, onClick }: { label: string; onClick: () => void }) => (
    <button type="button" onClick={onClick}>{label}</button>
  ),
  EditorToolbar: () => <div />,
  MarkdownToolbar: () => <div />,
  composeStyles: {
    backBtn: {},
    fieldRow: {},
    fieldLabel: {},
    toggleBtn: {},
  },
}));

vi.mock("@tiptap/react", () => ({
  EditorContent: () => <div data-testid="editor" />,
}));

vi.mock("../../../src/lib/templates", () => ({
  listTemplates: () => [],
  saveTemplate: vi.fn(),
  deleteTemplate: vi.fn(),
}));

vi.mock("../../../src/stores/confirm.store", () => ({
  useConfirmStore: { getState: () => ({ confirm: vi.fn().mockResolvedValue(true) }) },
}));

vi.mock("../../../src/stores/toast.store", () => ({
  useToastStore: { getState: () => ({ addToast: mocks.addToast }) },
}));

vi.mock("../../../src/lib/api", () => ({
  cleanupStagedComposeAttachment: vi.fn(),
  deleteDraft: vi.fn(),
  stageComposeAttachment: vi.fn(),
}));

function attachmentFile(name: string, contents = name): File {
  const file = new File([contents], name);
  Object.defineProperty(file, "arrayBuffer", {
    value: vi.fn().mockResolvedValue(new TextEncoder().encode(contents).buffer),
  });
  return file;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

describe("ComposeView", () => {
  it("saves immediately before leaving instead of deleting the saved draft", async () => {
    mocks.showComposeLeaveConfirm = true;
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Save and close" }));
    await waitFor(() => expect(mocks.confirmCloseCompose).toHaveBeenCalledOnce());
    expect(mocks.flushDraftSave).toHaveBeenCalledOnce();
    expect(deleteDraft).not.toHaveBeenCalled();
  });

  it("stays open when saving fails", async () => {
    mocks.showComposeLeaveConfirm = true;
    mocks.flushDraftSave.mockRejectedValue(new Error("Disk full"));
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Save and close" }));
    await waitFor(() => expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({ type: "error" })));
    expect(mocks.confirmCloseCompose).not.toHaveBeenCalled();
    expect(deleteDraft).not.toHaveBeenCalled();
  });

  it("retains the editor and attachment sources when discard cleanup fails", async () => {
    mocks.showComposeLeaveConfirm = true;
    vi.mocked(deleteDraft).mockRejectedValue(new Error("Unavailable"));
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(mocks.resumeDraftSaves).toHaveBeenCalledOnce());
    expect(mocks.confirmCloseCompose).not.toHaveBeenCalled();
    expect(cleanupStagedComposeAttachment).not.toHaveBeenCalled();
  });
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    localStorage.setItem("pebble-undo-send-seconds", "0");
    vi.mocked(queueControlledSend).mockReset();
    vi.mocked(restoreControlledSend).mockReset();
    useMailStore.setState({ activeAccountId: "account-1", selectedMessageId: null,
      selectedThreadId: null, selectedMessageIds: new Set(), batchMode: false });
    queryClient.clear();
    mocks.composeDraft = null;
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    mocks.mutate.mockReset();
    mocks.closeCompose.mockReset();
    mocks.setComposeDirty.mockReset();
    mocks.addToast.mockReset();
    mocks.loadDraftFromStorage.mockReset();
    mocks.cancelPendingDraftSaves.mockReset();
    mocks.flushDraftSave.mockReset().mockResolvedValue("draft-1");
    mocks.resumeDraftSaves.mockReset();
    mocks.cancelPendingDraftSaves.mockResolvedValue({ "account-1": "draft-1" });
    mocks.confirmCloseCompose.mockReset();
    mocks.cancelCloseCompose.mockReset();
    mocks.showComposeLeaveConfirm = false;
    mocks.quotedReplyHtml = "";
    mocks.accountsQuery.data = [{ id: "account-1", email: "me@example.com", display_name: "Me" }];
    mocks.accountsQuery.isLoading = false;
    mocks.accountsQuery.isSuccess = true;
    mocks.accountsQuery.isError = false;
    mocks.recipients.to = ["to@example.com"];
    mocks.recipients.cc = [];
    mocks.recipients.bcc = [];
    mocks.loadDraftFromStorage.mockReturnValue(null);
    vi.mocked(deleteDraft).mockReset();
    vi.mocked(deleteDraft).mockResolvedValue(undefined);
    vi.mocked(cleanupStagedComposeAttachment).mockReset();
    vi.mocked(cleanupStagedComposeAttachment).mockResolvedValue(undefined);
    vi.mocked(stageComposeAttachment).mockReset();
    vi.mocked(stageComposeAttachment).mockImplementation(async (filename) => `staged/${filename}`);
  });

  afterEach(() => {
    warnSpy.mockRestore();
    localStorage.removeItem("pebble-undo-send-seconds");
  });

  it("uses the default pre-delivery Undo queue without invoking the normal sender or deleting drafts", async () => {
    localStorage.removeItem("pebble-undo-send-seconds");
    vi.mocked(queueControlledSend).mockResolvedValue({ id: "job", accountId: "account-1", subject: "Subject", dueAt: Math.floor(Date.now() / 1000) + 10, revision: 0, state: "pending", kind: "undo" });
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(queueControlledSend).toHaveBeenCalledOnce());
    expect(queueControlledSend).toHaveBeenCalledWith(expect.objectContaining({
      accountId: "account-1", to: ["to@example.com"], editorMode: "rich", composeMode: "new", richTextHtml: "<p>Hello</p>",
    }), { undoSeconds: 10 }, expect.any(String));
    await screen.findByRole("button", { name: "sendControls.undo" });
    expect(mocks.mutate).not.toHaveBeenCalled();
    expect(deleteDraft).not.toHaveBeenCalled();
    expect(mocks.flushDraftSave).toHaveBeenCalledOnce();
  });

  it("Undo restores staged attachments and editable compose without deleting the saved draft", async () => {
    localStorage.setItem("pebble-undo-send-seconds", "10");
    vi.mocked(queueControlledSend).mockResolvedValue({ id: "job", accountId: "account-1", subject: "Subject", dueAt: Math.floor(Date.now() / 1000) + 10, revision: 0, state: "pending", kind: "undo" });
    vi.mocked(restoreControlledSend).mockResolvedValue({ accountId: "account-1", to: ["to@example.com"], cc: [], bcc: [], subject: "Subject", bodyText: "Hello", editorMode: "rich", composeMode: "reply-all", rawSource: "", richTextHtml: "<p>Hello</p>", quotedReplyHtml: "", attachments: [{ name: "restored.txt", path: "staged/new.txt", size: 10 }] });
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    fireEvent.click(await screen.findByRole("button", { name: "sendControls.undo" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Send" }).hasAttribute("disabled")).toBe(false));
    expect(screen.getByText("restored.txt")).toBeTruthy();
    expect(deleteDraft).not.toHaveBeenCalled();
    expect(mocks.resumeDraftSaves).toHaveBeenCalledOnce();
    expect(mocks.closeCompose).not.toHaveBeenCalled();
  });

  it("does not claim cancellation succeeded when the worker already claimed the send", async () => {
    localStorage.setItem("pebble-undo-send-seconds", "10");
    vi.mocked(queueControlledSend).mockResolvedValue({ id: "job", accountId: "account-1", subject: "Subject", dueAt: Math.floor(Date.now() / 1000) + 10, revision: 0, state: "pending", kind: "undo" });
    vi.mocked(restoreControlledSend).mockRejectedValue(new Error("already in flight"));
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    fireEvent.click(await screen.findByRole("button", { name: "sendControls.undo" }));
    await waitFor(() => expect(screen.getAllByRole("alert").length).toBeGreaterThan(0));
    expect(mocks.resumeDraftSaves).not.toHaveBeenCalled();
    expect(deleteDraft).not.toHaveBeenCalled();
  });

  it("keeps an IPC-lost queue request id stable rather than creating a duplicate on retry", async () => {
    localStorage.setItem("pebble-undo-send-seconds", "10");
    vi.mocked(queueControlledSend).mockRejectedValue(new Error("IPC reply lost"));
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(mocks.resumeDraftSaves).toHaveBeenCalledOnce());
    const requestId = vi.mocked(queueControlledSend).mock.calls[0][2];
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(queueControlledSend).toHaveBeenCalledTimes(2));
    expect(vi.mocked(queueControlledSend).mock.calls[1][2]).toBe(requestId);
    expect(mocks.mutate).not.toHaveBeenCalled();
  });

  it("persists the chosen local schedule before removing the original draft and closing", async () => {
    const due = new Date(Date.now() + 86400000);
    const value = `${due.getFullYear()}-${String(due.getMonth()+1).padStart(2,"0")}-${String(due.getDate()).padStart(2,"0")}T12:30`;
    const dueAt = new Date(value).getTime() / 1000;
    vi.mocked(queueControlledSend).mockResolvedValue({ id: "job", accountId: "account-1", subject: "Subject", dueAt, revision: 0, state: "pending", kind: "scheduled" });
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "sendControls.schedule" }));
    fireEvent.change(document.querySelector('input[type="datetime-local"]')!, { target: { value } });
    fireEvent.click(screen.getAllByRole("button", { name: "sendControls.schedule" })[1]);
    await waitFor(() => expect(mocks.closeCompose).toHaveBeenCalledOnce());
    expect(queueControlledSend).toHaveBeenCalledWith(expect.objectContaining({ editorMode: "rich", bodyText: "Hello" }), { dueAt }, expect.any(String));
    expect(deleteDraft).toHaveBeenCalledWith("account-1", "draft-1");
    expect(mocks.mutate).not.toHaveBeenCalled();
  });

  it("shows a user-visible error when sent draft cleanup fails", async () => {
    vi.mocked(deleteDraft).mockRejectedValue(new Error("remote draft delete failed"));
    mocks.mutate.mockImplementation((_params, options) => options.onSuccess());

    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(deleteDraft).toHaveBeenCalledWith("account-1", "draft-1"));
    await waitFor(() => expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({
      type: "error",
    })));
  });

  it("prevents duplicate sends while successful-send cleanup is pending", async () => {
    const pendingDeletion = deferred<void>();
    vi.mocked(deleteDraft).mockReturnValue(pendingDeletion.promise);
    mocks.mutate.mockImplementation((_params, options) => options.onSuccess());

    render(<ComposeView />);
    const sendButton = screen.getByRole("button", { name: "Send" }) as HTMLButtonElement;
    fireEvent.click(sendButton);
    await waitFor(() => expect(mocks.mutate).toHaveBeenCalledOnce());
    fireEvent.click(sendButton);

    expect(mocks.mutate).toHaveBeenCalledOnce();
    expect(sendButton.disabled).toBe(true);

    await act(async () => pendingDeletion.resolve());
    await waitFor(() => expect(mocks.closeCompose).toHaveBeenCalledOnce());
  });

  it.each(["send", "discard"])("clears deleted draft detail and singleton thread selection after %s", async (action) => {
    useMailStore.setState({ selectedMessageId: "draft-1", selectedThreadId: "draft-1", selectedMessageIds: new Set(["draft-1", "unrelated"]) });
    mocks.mutate.mockImplementation((_params, options) => options.onSuccess());
    mocks.showComposeLeaveConfirm = action === "discard";
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: action === "send" ? "Send" : "Discard" }));
    await waitFor(() => expect(deleteDraft).toHaveBeenCalledWith("account-1", "draft-1"));
    await waitFor(() => expect(useMailStore.getState().selectedMessageId).toBeNull());
    expect(useMailStore.getState().selectedThreadId).toBeNull();
    expect([...useMailStore.getState().selectedMessageIds]).toEqual(["unrelated"]);
  });

  it("retains selected draft detail when successful delivery cannot delete the draft", async () => {
    useMailStore.setState({ selectedMessageId: "draft-1", selectedThreadId: "draft-1" });
    vi.mocked(deleteDraft).mockRejectedValue(new Error("cleanup failed"));
    mocks.mutate.mockImplementation((_params, options) => options.onSuccess());
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(mocks.closeCompose).toHaveBeenCalledOnce());
    expect(useMailStore.getState().selectedMessageId).toBe("draft-1");
    expect(useMailStore.getState().selectedThreadId).toBe("draft-1");
  });

  it("does not clear a different message selected while draft deletion is pending", async () => {
    useMailStore.setState({ selectedMessageId: "draft-1" });
    const pending = deferred<void>();
    vi.mocked(deleteDraft).mockReturnValue(pending.promise);
    mocks.mutate.mockImplementation((_params, options) => options.onSuccess());
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(deleteDraft).toHaveBeenCalledOnce());
    act(() => useMailStore.setState({ selectedMessageId: "another-message", selectedThreadId: "another-thread" }));
    await act(async () => pending.resolve());
    await waitFor(() => expect(mocks.closeCompose).toHaveBeenCalledOnce());
    expect(useMailStore.getState().selectedMessageId).toBe("another-message");
    expect(useMailStore.getState().selectedThreadId).toBe("another-thread");
  });

  it.each([true, false])("only clears a cached conversation if all its messages were deleted: %s", async (onlyDraft) => {
    useMailStore.setState({ selectedThreadId: "conversation" });
    queryClient.setQueryData(["threadMessages", "conversation"], onlyDraft ? [{ id: "draft-1" }] : [{ id: "draft-1" }, { id: "other" }]);
    mocks.mutate.mockImplementation((_params, options) => options.onSuccess());
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(mocks.closeCompose).toHaveBeenCalledOnce());
    expect(useMailStore.getState().selectedThreadId).toBe(onlyDraft ? null : "conversation");
  });

  it("clears the original resumed draft UUID after cleanup of its promoted remote identity", async () => {
    mocks.composeDraft = { id: "original-uuid", accountId: "account-1", to: [], cc: [], bcc: [], subject: "Resumed", rawSource: "", richTextHtml: "", editorMode: "rich", attachments: [], savedAt: Date.now() };
    useMailStore.setState({ selectedMessageId: "original-uuid" });
    mocks.mutate.mockImplementation((_params, options) => options.onSuccess());
    render(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(mocks.closeCompose).toHaveBeenCalledOnce());
    expect(useMailStore.getState().selectedMessageId).toBeNull();
  });

  it("validates a restored new-message draft against loaded account ids", () => {
    render(<ComposeView />);

    expect(mocks.loadDraftFromStorage).toHaveBeenCalledWith(["account-1"]);
  });

  it("preserves a legacy draft when the accounts query fails", () => {
    mocks.accountsQuery.data = [];
    mocks.accountsQuery.isSuccess = false;
    mocks.accountsQuery.isError = true;

    render(<ComposeView />);

    expect(mocks.loadDraftFromStorage).toHaveBeenCalledWith(undefined);
  });

  it("sends a typed valid recipient without requiring Enter first", async () => {
    mocks.recipients.to = [];

    render(<ComposeView />);

    const sendButton = screen.getByRole("button", { name: "Send" }) as HTMLButtonElement;
    expect(sendButton.disabled).toBe(true);

    fireEvent.change(screen.getByRole("combobox", { name: "To" }), {
      target: { value: "typed@example.com" },
    });

    await waitFor(() => expect(sendButton.disabled).toBe(false));
    fireEvent.click(sendButton);

    await waitFor(() => expect(mocks.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ to: ["typed@example.com"] }),
      expect.any(Object),
    ));
  });

  it("keeps quoted replies collapsed until the user expands them", () => {
    mocks.quotedReplyHtml = "<blockquote><p>Original message body</p></blockquote>";

    render(<ComposeView />);

    expect(screen.getByRole("button", { name: "Show quoted message" })).toBeTruthy();
    expect(screen.queryByText("Original message body")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Show quoted message" }));

    expect(screen.getByText("Original message body")).toBeTruthy();
  });

  it("uses the first attached filename as an empty subject", async () => {
    render(<ComposeView />);

    const first = attachmentFile("季度报告.pdf", "quarterly results");
    const second = attachmentFile("supporting-data.xlsx", "supporting data");

    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    expect(fileInput).not.toBeNull();
    fireEvent.change(fileInput!, { target: { files: [first, second] } });

    await waitFor(() => expect(vi.mocked(stageComposeAttachment)).toHaveBeenCalledTimes(2));
    await waitFor(() => expect((screen.getByLabelText("Subject") as HTMLInputElement).value)
      .toBe("季度报告.pdf"));
  });

  it("does not let a later concurrent attachment refill a subject the user cleared", async () => {
    let resolveFirst!: (path: string) => void;
    let resolveSecond!: (path: string) => void;
    vi.mocked(stageComposeAttachment).mockImplementation((filename) => new Promise((resolve) => {
      if (filename === "first.pdf") resolveFirst = resolve;
      if (filename === "second.pdf") resolveSecond = resolve;
    }));

    render(<ComposeView />);

    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    const subjectInput = screen.getByLabelText("Subject") as HTMLInputElement;
    expect(fileInput).not.toBeNull();

    fireEvent.change(fileInput!, { target: { files: [attachmentFile("first.pdf")] } });
    await waitFor(() => expect(stageComposeAttachment).toHaveBeenCalledWith("first.pdf", expect.any(Array)));
    fireEvent.change(fileInput!, { target: { files: [attachmentFile("second.pdf")] } });
    await waitFor(() => expect(stageComposeAttachment).toHaveBeenCalledWith("second.pdf", expect.any(Array)));

    await act(async () => resolveFirst("staged/first.pdf"));
    await waitFor(() => expect(subjectInput.value).toBe("first.pdf"));
    fireEvent.change(subjectInput, { target: { value: "" } });

    await act(async () => resolveSecond("staged/second.pdf"));
    await waitFor(() => expect(screen.getByText("second.pdf")).toBeTruthy());
    expect(subjectInput.value).toBe("");
  });

  it("keeps successfully staged files when a later file in the batch fails", async () => {
    vi.mocked(stageComposeAttachment)
      .mockResolvedValueOnce("staged/first.pdf")
      .mockRejectedValueOnce(new Error("staging failed"));

    render(<ComposeView />);

    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    expect(fileInput).not.toBeNull();
    fireEvent.change(fileInput!, {
      target: { files: [attachmentFile("first.pdf"), attachmentFile("broken.pdf")] },
    });

    await waitFor(() => expect(screen.getByText("first.pdf")).toBeTruthy());
    expect(screen.queryByText("broken.pdf")).toBeNull();
    expect((screen.getByLabelText("Subject") as HTMLInputElement).value).toBe("first.pdf");
    expect(screen.getByRole("alert").textContent).toContain("Failed to attach file");
  });

  it("disables send until attachment staging finishes and sends the staged path", async () => {
    const pendingStage = deferred<string>();
    vi.mocked(stageComposeAttachment).mockReturnValue(pendingStage.promise);
    render(<ComposeView />);

    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    fireEvent.change(fileInput!, { target: { files: [attachmentFile("slow.pdf")] } });
    await waitFor(() => expect(stageComposeAttachment)
      .toHaveBeenCalledWith("slow.pdf", expect.any(Array)));

    const sendButton = screen.getByRole("button", { name: "Send" }) as HTMLButtonElement;
    expect(sendButton.disabled).toBe(true);
    expect(mocks.mutate).not.toHaveBeenCalled();

    await act(async () => pendingStage.resolve("staged/slow.pdf"));
    await waitFor(() => expect(sendButton.disabled).toBe(false));
    fireEvent.click(sendButton);

    await waitFor(() => expect(mocks.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ attachmentPaths: ["staged/slow.pdf"] }),
      expect.any(Object),
    ));
  });

  it("disables discard until attachment staging finishes, then cleans the staged path", async () => {
    const pendingStage = deferred<string>();
    vi.mocked(stageComposeAttachment).mockReturnValue(pendingStage.promise);
    mocks.showComposeLeaveConfirm = true;
    render(<ComposeView />);

    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    fireEvent.change(fileInput!, { target: { files: [attachmentFile("slow-discard.pdf")] } });
    await waitFor(() => expect(stageComposeAttachment)
      .toHaveBeenCalledWith("slow-discard.pdf", expect.any(Array)));

    const discardButton = screen.getByRole("button", { name: "Discard" }) as HTMLButtonElement;
    expect(discardButton.disabled).toBe(true);
    await act(async () => pendingStage.resolve("staged/slow-discard.pdf"));
    await waitFor(() => expect(discardButton.disabled).toBe(false));
    fireEvent.click(discardButton);

    await waitFor(() => expect(cleanupStagedComposeAttachment)
      .toHaveBeenCalledWith("staged/slow-discard.pdf"));
    await waitFor(() => expect(mocks.confirmCloseCompose).toHaveBeenCalledOnce());
  });

  it("cleans an attachment that finishes staging after the compose view unmounts", async () => {
    const pendingStage = deferred<string>();
    vi.mocked(stageComposeAttachment).mockReturnValue(pendingStage.promise);
    const { unmount } = render(<ComposeView />);

    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    fireEvent.change(fileInput!, { target: { files: [attachmentFile("late.pdf")] } });
    await waitFor(() => expect(stageComposeAttachment)
      .toHaveBeenCalledWith("late.pdf", expect.any(Array)));
    unmount();

    await act(async () => pendingStage.resolve("staged/late.pdf"));
    await waitFor(() => expect(cleanupStagedComposeAttachment)
      .toHaveBeenCalledWith("staged/late.pdf"));
  });

  it("deletes a staged file when the attachment is removed", async () => {
    render(<ComposeView />);
    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    fireEvent.change(fileInput!, { target: { files: [attachmentFile("remove-me.pdf")] } });
    await waitFor(() => expect(screen.getByText("remove-me.pdf")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: /Remove attachment/ }));

    await waitFor(() => expect(cleanupStagedComposeAttachment)
      .toHaveBeenCalledWith("staged/remove-me.pdf"));
    expect(screen.queryByText("remove-me.pdf")).toBeNull();
  });

  it("deletes autosaved drafts and staged files before confirming discard", async () => {
    const { rerender } = render(<ComposeView />);
    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    fireEvent.change(fileInput!, { target: { files: [attachmentFile("discard-me.pdf")] } });
    await waitFor(() => expect(screen.getByText("discard-me.pdf")).toBeTruthy());

    mocks.showComposeLeaveConfirm = true;
    rerender(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));

    await waitFor(() => expect(mocks.cancelPendingDraftSaves).toHaveBeenCalledOnce());
    await waitFor(() => expect(deleteDraft).toHaveBeenCalledWith("account-1", "draft-1"));
    await waitFor(() => expect(cleanupStagedComposeAttachment)
      .toHaveBeenCalledWith("staged/discard-me.pdf"));
    await waitFor(() => expect(mocks.confirmCloseCompose).toHaveBeenCalledOnce());
  });

  it("cleans restored compose-staging attachments when discarding a legacy draft", async () => {
    mocks.loadDraftFromStorage.mockReturnValue({
      accountId: "account-1",
      to: ["to@example.com"],
      cc: [],
      bcc: [],
      subject: "Restored",
      rawSource: "",
      richTextHtml: "<p>Restored</p>",
      editorMode: "rich",
      attachments: [{ name: "restored.pdf", path: "staged/restored.pdf", size: 42 }],
      savedAt: Date.now(),
    });
    mocks.showComposeLeaveConfirm = true;
    render(<ComposeView />);

    expect(screen.getByText("restored.pdf")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));

    await waitFor(() => expect(cleanupStagedComposeAttachment)
      .toHaveBeenCalledWith("staged/restored.pdf"));
  });

  it("does not delete a restored attachment source on ordinary unmount", async () => {
    mocks.loadDraftFromStorage.mockReturnValue({
      accountId: "account-1",
      to: ["to@example.com"],
      cc: [],
      bcc: [],
      subject: "Restored",
      rawSource: "",
      richTextHtml: "<p>Restored</p>",
      editorMode: "rich",
      attachments: [{ name: "restored.pdf", path: "staged/restored.pdf", size: 42 }],
      savedAt: Date.now(),
    });
    const { unmount } = render(<ComposeView />);

    unmount();
    await act(async () => Promise.resolve());

    expect(cleanupStagedComposeAttachment).not.toHaveBeenCalled();
  });

  it("waits for autosave and draft deletion before cleaning staged files", async () => {
    const pendingCancellation = deferred<Record<string, string>>();
    const pendingDeletion = deferred<void>();
    mocks.cancelPendingDraftSaves.mockReturnValue(pendingCancellation.promise);
    vi.mocked(deleteDraft).mockReturnValue(pendingDeletion.promise);

    const { rerender } = render(<ComposeView />);
    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    fireEvent.change(fileInput!, { target: { files: [attachmentFile("in-flight.pdf")] } });
    await waitFor(() => expect(screen.getByText("in-flight.pdf")).toBeTruthy());

    mocks.showComposeLeaveConfirm = true;
    rerender(<ComposeView />);
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));

    expect(cleanupStagedComposeAttachment).not.toHaveBeenCalled();
    await act(async () => pendingCancellation.resolve({ "account-1": "draft-1" }));
    await waitFor(() => expect(deleteDraft).toHaveBeenCalledWith("account-1", "draft-1"));
    expect(cleanupStagedComposeAttachment).not.toHaveBeenCalled();

    await act(async () => pendingDeletion.resolve());
    await waitFor(() => expect(cleanupStagedComposeAttachment)
      .toHaveBeenCalledWith("staged/in-flight.pdf"));
  });

  it("traps focus in the discard dialog and restores the opener after Escape", () => {
    const { rerender } = render(<ComposeView />);
    const backButton = screen.getByRole("button", { name: "Back" });
    backButton.focus();

    mocks.showComposeLeaveConfirm = true;
    rerender(<ComposeView />);

    const keepEditingButton = screen.getByRole("button", { name: "Keep editing" });
    screen.getByRole("button", { name: "Discard" });
    expect(document.activeElement).toBe(keepEditingButton);

    fireEvent.keyDown(document, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Save and close" }));
    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(keepEditingButton);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(mocks.cancelCloseCompose).toHaveBeenCalledOnce();
    mocks.showComposeLeaveConfirm = false;
    rerender(<ComposeView />);
    expect(document.activeElement).toBe(backButton);
  });

  it("ignores repeated discard actions while cleanup is pending", async () => {
    const pendingCancellation = deferred<Record<string, string>>();
    mocks.cancelPendingDraftSaves.mockReturnValue(pendingCancellation.promise);
    mocks.showComposeLeaveConfirm = true;
    render(<ComposeView />);

    const discardButton = screen.getByRole("button", { name: "Discard" });
    fireEvent.click(discardButton);
    fireEvent.click(discardButton);

    expect(mocks.cancelPendingDraftSaves).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Keep editing" }));
    expect(mocks.cancelCloseCompose).not.toHaveBeenCalled();

    await act(async () => {
      pendingCancellation.resolve({ "account-1": "draft-1" });
      await pendingCancellation.promise;
    });
    await waitFor(() => expect(mocks.confirmCloseCompose).toHaveBeenCalledOnce());
  });

  it("cleans staged files after a successful send", async () => {
    mocks.mutate.mockImplementation((_params, options) => options.onSuccess());
    render(<ComposeView />);
    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    fireEvent.change(fileInput!, { target: { files: [attachmentFile("sent.pdf")] } });
    await waitFor(() => expect(screen.getByText("sent.pdf")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(cleanupStagedComposeAttachment)
      .toHaveBeenCalledWith("staged/sent.pdf"));
  });
});
