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

function makeMessage(overrides: Partial<MessageSummary> = {}): MessageSummary {
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
    ...overrides,
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

  it("refreshes folder unread counts after a successful archive action", async () => {
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    fireEvent.mouseEnter(screen.getByRole("option"));
    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    await waitFor(() => expect(mocks.archiveMessage).toHaveBeenCalledWith("message-1"));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });

  it("refreshes folder unread counts after a successful spam action", async () => {
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

    await waitFor(() => expect(mocks.moveToFolder).toHaveBeenCalledWith("message-1", "folder-spam"));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
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

  it("uses the custom batch checkbox control for row selection", () => {
    const onToggleBatchSelect = vi.fn();

    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
        batchMode
        batchSelected={false}
        onToggleBatchSelect={onToggleBatchSelect}
      />,
    );

    const checkbox = screen.getByRole("checkbox", { name: "Select message" });

    expect(checkbox.className).toContain("batch-checkbox");
    expect(checkbox.className).toContain("message-row-checkbox");

    fireEvent.click(checkbox);

    expect(onToggleBatchSelect).toHaveBeenCalledWith("message-1");
  });

  it("shows the source account color marker when an account color is provided", () => {
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
        {...({
          accountColor: "#22c55e",
          accountLabel: "Work <work@example.com>",
        } as Record<string, unknown>)}
      />,
    );

    const marker = screen.getByTitle("Work <work@example.com>");

    expect(marker.getAttribute("aria-label")).toBe("Work <work@example.com>");
    expect(marker.style.backgroundColor).toBe("rgb(34, 197, 94)");
  });

  it("marks unread rows with a row class and unread dot", () => {
    const { container } = render(
      <MessageItem
        message={makeMessage({ is_read: false })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    expect(screen.getByRole("option").className).toContain("message-list-row--unread");
    expect(container.querySelector(".message-list-row__unread-dot")).not.toBeNull();
  });

  it("does not add unread row treatment to read rows", () => {
    const { container } = render(
      <MessageItem
        message={makeMessage({ is_read: true })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    expect(screen.getByRole("option").className).not.toContain("message-list-row--unread");
    expect(container.querySelector(".message-list-row__unread-dot")).toBeNull();
  });
});
