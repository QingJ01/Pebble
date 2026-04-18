import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ComposeView from "../../../src/features/compose/ComposeView";
import { deleteDraft } from "../../../src/lib/api";

const mocks = vi.hoisted(() => ({
  mutate: vi.fn(),
  closeCompose: vi.fn(),
  setComposeDirty: vi.fn(),
  addToast: vi.fn(),
  loadDraftFromStorage: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (_key: string, fallback?: string) => fallback ?? _key }),
}));

vi.mock("../../../src/stores/mail.store", () => ({
  useMailStore: (selector: (state: { activeAccountId: string }) => unknown) =>
    selector({ activeAccountId: "account-1" }),
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
        composeReplyTo: null,
        closeCompose: mocks.closeCompose,
        showComposeLeaveConfirm: false,
        confirmCloseCompose: vi.fn(),
        cancelCloseCompose: vi.fn(),
      }),
    {
      getState: () => ({ setComposeDirty: mocks.setComposeDirty }),
    },
  ),
}));

vi.mock("../../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({
    data: [{ id: "account-1", email: "me@example.com", display_name: "Me" }],
  }),
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
    to: ["to@example.com"],
    setTo: vi.fn(),
    cc: [],
    setCc: vi.fn(),
    bcc: [],
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
  }),
}));

vi.mock("../../../src/components/ContactAutocomplete", () => ({
  default: () => <input aria-label="To" value="to@example.com" readOnly />,
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
  deleteDraft: vi.fn(),
}));

describe("ComposeView", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    mocks.mutate.mockReset();
    mocks.closeCompose.mockReset();
    mocks.setComposeDirty.mockReset();
    mocks.addToast.mockReset();
    mocks.loadDraftFromStorage.mockReset();
    mocks.loadDraftFromStorage.mockReturnValue(null);
    vi.mocked(deleteDraft).mockReset();
  });

  afterEach(() => {
    warnSpy.mockRestore();
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

  it("validates a restored new-message draft against loaded account ids", () => {
    render(<ComposeView />);

    expect(mocks.loadDraftFromStorage).toHaveBeenCalledWith(["account-1"]);
  });
});
