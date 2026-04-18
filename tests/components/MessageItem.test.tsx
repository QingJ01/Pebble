import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageSummary } from "../../src/lib/api";

const mocks = vi.hoisted(() => ({
  queryClient: {
    invalidateQueries: vi.fn(),
  },
  patchMessagesCache: vi.fn(),
  snapshotMessagesCache: vi.fn(),
  restoreMessagesCache: vi.fn(),
  updateMessageFlags: vi.fn(),
  archiveMessage: vi.fn(),
  moveToFolder: vi.fn(),
  addToast: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "messageActions.archive": "Archive",
        "messageActions.unarchive": "Unarchive",
        "messageActions.addToKanban": "Add to kanban",
        "messageActions.reportSpam": "Report spam",
        "messageActions.star": "Star",
        "messageActions.unstar": "Unstar",
      };
      return labels[key] ?? fallback ?? key;
    },
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => mocks.queryClient,
}));

vi.mock("../../src/hooks/queries", () => ({
  patchMessagesCache: mocks.patchMessagesCache,
  snapshotMessagesCache: mocks.snapshotMessagesCache,
  restoreMessagesCache: mocks.restoreMessagesCache,
}));

vi.mock("../../src/lib/api", () => ({
  updateMessageFlags: mocks.updateMessageFlags,
  archiveMessage: mocks.archiveMessage,
  moveToFolder: mocks.moveToFolder,
}));

vi.mock("../../src/stores/kanban.store", () => ({
  useKanbanStore: (selector: (state: { cardIdSet: Set<string> }) => unknown) =>
    selector({ cardIdSet: new Set() }),
}));

vi.mock("../../src/stores/toast.store", () => ({
  useToastStore: {
    getState: () => ({ addToast: mocks.addToast }),
  },
}));

import MessageItem from "../../src/components/MessageItem";

function makeMessage(): MessageSummary {
  return {
    id: "message-1",
    account_id: "account-1",
    remote_id: "remote-message-1",
    message_id_header: null,
    in_reply_to: null,
    references_header: null,
    thread_id: null,
    subject: "Archived message",
    snippet: "Snippet",
    from_address: "sender@example.com",
    from_name: "Sender",
    to_list: [],
    cc_list: [],
    bcc_list: [],
    has_attachments: false,
    is_read: true,
    is_starred: false,
    is_draft: false,
    date: 1_700_000_000,
    remote_version: null,
    is_deleted: false,
    deleted_at: null,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
  };
}

describe("MessageItem", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.snapshotMessagesCache.mockReturnValue({ messages: "snapshot" });
    mocks.updateMessageFlags.mockResolvedValue(undefined);
    mocks.archiveMessage.mockResolvedValue("archived");
    mocks.moveToFolder.mockResolvedValue(undefined);
  });

  it("labels the archive action as unarchive in the archive folder", () => {
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
        {...({ folderRole: "archive" } as Record<string, unknown>)}
      />,
    );

    fireEvent.mouseEnter(screen.getByRole("option"));

    expect(screen.getByRole("button", { name: "Unarchive" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Archive" })).toBeNull();
  });

  it("restores message lists when archive optimistic update fails", async () => {
    const snapshot = { messages: "before-archive" };
    mocks.snapshotMessagesCache.mockReturnValueOnce(snapshot);
    mocks.archiveMessage.mockRejectedValueOnce(new Error("archive failed"));

    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    fireEvent.mouseEnter(screen.getByRole("option"));
    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    expect(mocks.snapshotMessagesCache).toHaveBeenCalledWith(mocks.queryClient);
    expect(mocks.patchMessagesCache).toHaveBeenCalledWith(mocks.queryClient, expect.any(Function));
    await waitFor(() => expect(mocks.restoreMessagesCache).toHaveBeenCalledWith(mocks.queryClient, snapshot));
  });

  it("restores message lists when spam optimistic update fails", async () => {
    const snapshot = { messages: "before-spam" };
    mocks.snapshotMessagesCache.mockReturnValueOnce(snapshot);
    mocks.moveToFolder.mockRejectedValueOnce(new Error("spam failed"));

    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
        spamFolderId="folder-spam"
      />,
    );

    fireEvent.mouseEnter(screen.getByRole("option"));
    fireEvent.click(screen.getByRole("button", { name: "Report spam" }));

    expect(mocks.snapshotMessagesCache).toHaveBeenCalledWith(mocks.queryClient);
    expect(mocks.patchMessagesCache).toHaveBeenCalledWith(mocks.queryClient, expect.any(Function));
    await waitFor(() => expect(mocks.restoreMessagesCache).toHaveBeenCalledWith(mocks.queryClient, snapshot));
  });

  it("refreshes derived queries after starring from row actions", async () => {
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    fireEvent.mouseEnter(screen.getByRole("option"));
    fireEvent.click(screen.getByRole("button", { name: "Star" }));

    await waitFor(() => expect(mocks.updateMessageFlags).toHaveBeenCalledWith("message-1", undefined, true));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["messages"] });
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["threads"] });
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["starred-messages"] });
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["message", "message-1"] });
  });
});
