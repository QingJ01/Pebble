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
  mutate: vi.fn(),
}));

vi.mock("react-i18next", () => ({
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

vi.mock("../../src/stores/kanban.store", () => {
  const useKanbanStore = Object.assign(
    (selector: (state: { cardIdSet: Set<string> }) => unknown) =>
      selector({ cardIdSet: new Set() }),
    { getState: () => ({ addCard: mocks.addCard }) },
  );
  return { useKanbanStore };
});

vi.mock("../../src/stores/toast.store", () => ({
  useToastStore: {
    getState: () => ({ addToast: mocks.addToast }),
  },
}));

vi.mock("../../src/components/ConfirmDialog", () => ({
  default: ({ onConfirm }: { onConfirm: () => void }) => (
    <button onClick={onConfirm}>Confirm delete</button>
  ),
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
    mocks.archiveMessage.mockResolvedValue("archived");
    mocks.deleteMessage.mockResolvedValue(undefined);
    mocks.restoreMessage.mockResolvedValue(undefined);
  });

  it("refreshes folder unread counts after archive from the detail toolbar", async () => {
    render(
      <MessageActionToolbar
        message={makeMessage()}
        onBack={vi.fn()}
        onMessageUpdate={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    await waitFor(() => expect(mocks.archiveMessage).toHaveBeenCalledWith("message-1"));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
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

  it("refreshes folder unread counts after delete from the detail toolbar", async () => {
    render(
      <MessageActionToolbar
        message={makeMessage()}
        onBack={vi.fn()}
        onMessageUpdate={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    fireEvent.click(screen.getByRole("button", { name: "Confirm delete" }));

    await waitFor(() => expect(mocks.deleteMessage).toHaveBeenCalledWith("message-1"));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });
});
