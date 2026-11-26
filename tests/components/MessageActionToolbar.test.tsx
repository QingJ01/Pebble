import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Message } from "../../src/lib/api";

const mocks = vi.hoisted(() => ({
  queryClient: {
    invalidateQueries: vi.fn(),
  },
  archiveMessage: vi.fn(),
  deleteMessage: vi.fn(),
  restoreMessage: vi.fn(),
  patchMessagesCache: vi.fn(),
  snapshotMessagesCache: vi.fn(),
  restoreMessagesCache: vi.fn(),
  addToast: vi.fn(),
  openCompose: vi.fn(),
  addCard: vi.fn(),
  invalidateKanbanQueries: vi.fn(),
  mutate: vi.fn(),
  runMailAction: vi.fn(),
  toastError: vi.fn(),
  confirm: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "messageActions.reply": "Reply",
        "messageActions.replyAll": "Reply all",
        "messageActions.forward": "Forward",
        "messageActions.star": "Star",
        "messageActions.unstar": "Unstar",
        "messageActions.archive": "Archive",
        "messageActions.unarchive": "Unarchive",
        "messageActions.restore": "Restore",
        "messageActions.delete": "Delete",
        "messageActions.addToKanban": "Add to kanban",
        "common.delete": "Delete",
        "messageActions.deletePermanently": "Delete permanently",
      };
      return labels[key] ?? fallback ?? key;
    },
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => mocks.queryClient,
}));

vi.mock("../../src/hooks/mutations/useUpdateFlagsMutation", () => ({
  useUpdateFlagsMutation: () => ({ mutate: mocks.mutate }),
}));

vi.mock("../../src/hooks/queries", () => ({
  patchMessagesCache: mocks.patchMessagesCache,
  snapshotMessagesCache: mocks.snapshotMessagesCache,
  restoreMessagesCache: mocks.restoreMessagesCache,
}));

vi.mock("../../src/lib/api", () => ({
  archiveMessage: mocks.archiveMessage,
  deleteMessage: mocks.deleteMessage,
  restoreMessage: mocks.restoreMessage,
}));

vi.mock("../../src/stores/compose.store", () => ({
  useComposeStore: (selector: (state: { openCompose: typeof mocks.openCompose }) => unknown) =>
    selector({ openCompose: mocks.openCompose }),
}));

vi.mock("../../src/hooks/queries/useKanbanCardsQuery", () => ({
  useIsInKanban: () => false,
  invalidateKanbanQueries: mocks.invalidateKanbanQueries,
}));

vi.mock("../../src/hooks/mutations/useKanbanMutations", () => ({
  addKanbanCard: mocks.addCard,
}));

vi.mock("../../src/stores/toast.store", () => ({
  useToastStore: {
    getState: () => ({ addToast: mocks.addToast }),
  },
  toastError: mocks.toastError,
}));

vi.mock("../../src/stores/confirm.store", () => ({
  useConfirmStore: { getState: () => ({ confirm: mocks.confirm }) },
}));

vi.mock("../../src/hooks/mutations/mailActions", () => ({
  runMailAction: mocks.runMailAction,
  trashConfirmCopy: () => ({ title: "Delete", message: "m", confirmLabel: "Delete" }),
}));

import MessageActionToolbar from "../../src/components/MessageActionToolbar";

function makeMessage(): Message {
  return {
    id: "message-1",
    account_id: "account-1",
    remote_id: "remote-message-1",
    message_id_header: null,
    in_reply_to: null,
    references_header: null,
    thread_id: null,
    subject: "Subject",
    snippet: "Snippet",
    from_address: "sender@example.com",
    from_name: "Sender",
    to_list: [],
    cc_list: [],
    bcc_list: [],
    has_attachments: false,
    is_read: false,
    is_starred: false,
    is_draft: false,
    date: 1_700_000_000,
    remote_version: null,
    is_deleted: false,
    deleted_at: null,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
    body_text: "Body",
    body_html_raw: "",
  };
}

describe("MessageActionToolbar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.snapshotMessagesCache.mockReturnValue({ messages: "snapshot" });
    mocks.restoreMessage.mockResolvedValue(undefined);
    mocks.runMailAction.mockImplementation(async (_client, _kind, _messages, options) => {
      options?.onRemoved?.(["message-1"]);
      return { ok: true, count: 1 };
    });
  });

  it("archives through the shared undoable action and leaves the detail view", async () => {
    const onBack = vi.fn();
    render(
      <MessageActionToolbar
        message={makeMessage()}
        onBack={onBack}
        onMessageUpdate={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient, "archive", [expect.objectContaining({ id: "message-1" })], expect.anything(),
    ));
    expect(onBack).toHaveBeenCalled();
  });

  it("moves to trash without its own confirm dialog; confirmation is left to non-undoable plans", async () => {
    render(
      <MessageActionToolbar
        message={makeMessage()}
        onBack={vi.fn()}
        onMessageUpdate={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient, "trash", [expect.objectContaining({ id: "message-1" })],
      expect.objectContaining({ confirm: expect.any(Function) }),
    ));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(mocks.confirm).not.toHaveBeenCalled();
  });

  it("labels delete as permanent in Trash and separates it from safe actions", () => {
    const { container } = render(
      <MessageActionToolbar
        message={makeMessage()}
        folderRole="trash"
        onBack={vi.fn()}
        onMessageUpdate={vi.fn()}
      />,
    );

    const del = screen.getByRole("button", { name: "Delete permanently" });
    expect(del.previousElementSibling?.className).toContain("toolbar-separator");
    expect(container.querySelector('[role="toolbar"]')).toBeTruthy();
  });

  it("exposes the star toggle state", () => {
    render(
      <MessageActionToolbar
        message={{ ...makeMessage(), is_starred: true }}
        onBack={vi.fn()}
        onMessageUpdate={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Unstar" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("refreshes folder unread counts after restore from the detail toolbar", async () => {
    render(
      <MessageActionToolbar
        message={makeMessage()}
        folderRole="trash"
        onBack={vi.fn()}
        onMessageUpdate={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Restore" }));

    await waitFor(() => expect(mocks.restoreMessage).toHaveBeenCalledWith("message-1"));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });

  it("opens the kanban column picker as a keyboard menu", () => {
    render(
      <MessageActionToolbar
        message={makeMessage()}
        onBack={vi.fn()}
        onMessageUpdate={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Add to kanban" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Waiting" }));

    expect(mocks.addCard).toHaveBeenCalledWith("message-1", "waiting", mocks.queryClient);
  });
});
