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
  toastError: vi.fn(),
  runMailAction: vi.fn(),
  getMessage: vi.fn(),
  restoreMessage: vi.fn(),
  openCompose: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "messageActions.archive": "Archive",
        "messageActions.unarchive": "Unarchive",
        "messageActions.addToKanban": "Add to kanban",
        "messageActions.reportSpam": "Report spam",
        "messageActions.star": "Star",
        "messageActions.unstar": "Unstar",
        "messageActions.markUnread": "Mark as unread",
        "messageActions.markRead": "Mark as read",
        "messageActions.updateReadFailed": "Failed to update read state",
        "messageActions.contextMenu": "Message actions",
        "messageActions.reply": "Reply",
        "messageActions.delete": "Delete",
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
  getMessage: mocks.getMessage,
  restoreMessage: mocks.restoreMessage,
}));

vi.mock("../../src/hooks/mutations/mailActions", () => ({
  runMailAction: mocks.runMailAction,
  trashConfirmCopy: () => ({ title: "t", message: "m", confirmLabel: "c" }),
}));

vi.mock("../../src/stores/compose.store", () => ({
  useComposeStore: { getState: () => ({ openCompose: mocks.openCompose }) },
}));

vi.mock("../../src/hooks/queries/useKanbanCardsQuery", () => ({
  useIsInKanban: () => false,
}));

vi.mock("../../src/hooks/mutations/useKanbanMutations", () => ({
  addKanbanCard: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("../../src/stores/toast.store", () => ({
  useToastStore: {
    getState: () => ({ addToast: mocks.addToast }),
  },
  toastError: mocks.toastError,
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
    mocks.runMailAction.mockResolvedValue({ ok: true, count: 1 });
    mocks.getMessage.mockResolvedValue({ id: "message-1", account_id: "account-1" });
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

  it("archives through the shared undoable mail action", async () => {
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient,
      "archive",
      [expect.objectContaining({ id: "message-1" })],
      expect.objectContaining({ onRemoved: expect.any(Function), confirm: expect.any(Function) }),
    ));
  });

  it("reports spam through the shared undoable mail action with the spam folder", async () => {
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
        spamFolderId="folder-spam"
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Report spam" }));

    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient,
      "spam",
      [expect.objectContaining({ id: "message-1" })],
      expect.objectContaining({ spamFolderId: "folder-spam" }),
    ));
  });

  it("does not open the message when Enter or Space is pressed on a row action button", () => {
    const onClick = vi.fn();
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={onClick}
      />,
    );

    const archive = screen.getByRole("button", { name: "Archive" });
    fireEvent.keyDown(archive, { key: "Enter" });
    fireEvent.keyDown(archive, { key: " " });
    expect(onClick).not.toHaveBeenCalled();

    fireEvent.keyDown(screen.getByRole("option"), { key: "Enter" });
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("toggles batch selection with Space and opens with Enter in batch mode", () => {
    const onOpen = vi.fn();
    const onToggleBatchSelect = vi.fn();
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
        onOpen={onOpen}
        batchMode
        onToggleBatchSelect={onToggleBatchSelect}
      />,
    );

    fireEvent.keyDown(screen.getByRole("option"), { key: " " });
    expect(onToggleBatchSelect).toHaveBeenCalledWith("message-1");
    fireEvent.keyDown(screen.getByRole("option"), { key: "Enter" });
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("offers reply, archive and delete in the context menu and opens it with Shift+F10", async () => {
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    fireEvent.keyDown(screen.getByRole("option"), { key: "F10", shiftKey: true });
    const menu = screen.getByRole("menu", { name: "Message actions" });
    expect(menu).toBeTruthy();
    expect(document.activeElement?.getAttribute("role")).toBe("menuitem");

    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(document.activeElement?.textContent).toContain("Reply All");

    fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient, "trash", [expect.objectContaining({ id: "message-1" })], expect.anything(),
    ));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("closes the context menu on Escape and returns focus to the row", () => {
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );
    const row = screen.getByRole("option");
    row.focus();
    fireEvent.contextMenu(row, { clientX: 10, clientY: 10 });
    expect(screen.getByRole("menu")).toBeTruthy();

    fireEvent.keyDown(document.activeElement!, { key: "Escape" });

    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(row);
  });

  it("offers Move to… in the context menu, disabled for drafts", () => {
    const { unmount } = render(<MessageItem message={makeMessage()} isSelected={false} onClick={vi.fn()} />);
    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 10, clientY: 10 });
    const item = screen.getByRole("menuitem", { name: /Move to/ });
    expect(item.getAttribute("aria-disabled")).toBeNull();
    unmount();

    render(<MessageItem message={{ ...makeMessage(), is_draft: true }} isSelected={false} onClick={vi.fn()} />);
    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 10, clientY: 10 });
    expect(screen.getByRole("menuitem", { name: /Move to/ }).getAttribute("aria-disabled")).toBe("true");
  });

  it("closes the context menu when the page scrolls", () => {
    render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );
    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 10, clientY: 10 });
    expect(screen.getByRole("menu")).toBeTruthy();

    fireEvent.scroll(document);

    expect(screen.queryByRole("menu")).toBeNull();
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

  it("rolls back optimistic star state and shows an error when persistence fails", async () => {
    const snapshot = { messages: "before-star" };
    const onToggleStar = vi.fn();
    mocks.snapshotMessagesCache.mockReturnValueOnce(snapshot);
    mocks.updateMessageFlags.mockRejectedValueOnce(new Error("star failed"));

    render(
      <MessageItem
        message={makeMessage({ is_starred: false })}
        isSelected={false}
        onClick={vi.fn()}
        onToggleStar={onToggleStar}
      />,
    );

    fireEvent.mouseEnter(screen.getByRole("option"));
    fireEvent.click(screen.getByRole("button", { name: "Star" }));

    expect(onToggleStar).toHaveBeenCalledWith("message-1", true);
    await waitFor(() => expect(onToggleStar).toHaveBeenLastCalledWith("message-1", false));
    expect(mocks.restoreMessagesCache).toHaveBeenCalledWith(mocks.queryClient, snapshot);
    expect(mocks.toastError).toHaveBeenCalledWith(expect.any(Error), "messageActions.starFailed");
  });


  it("marks a read message unread from the row context menu", async () => {
    render(
      <MessageItem
        message={makeMessage({ is_read: true })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 120, clientY: 80 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark as unread" }));

    await waitFor(() => expect(mocks.updateMessageFlags).toHaveBeenCalledWith("message-1", false));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["starred-messages"] });
    expect(screen.queryByRole("menu", { name: "Message actions" })).toBeNull();
  });

  it("does not let keyboard activation bubble from the portal menu to the row", () => {
    const onClick = vi.fn();
    render(
      <MessageItem
        message={makeMessage({ is_read: true })}
        isSelected={false}
        onClick={onClick}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 120, clientY: 80 });
    fireEvent.keyDown(screen.getByRole("menuitem", { name: "Mark as unread" }), { key: "Enter" });

    expect(onClick).not.toHaveBeenCalled();
    expect(screen.getByRole("menu", { name: "Message actions" })).toBeTruthy();
  });

  it("marks an unread message read from the row context menu", async () => {
    render(
      <MessageItem
        message={makeMessage({ is_read: false })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("option"), { clientX: 120, clientY: 80 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Mark as read" }));

    await waitFor(() => expect(mocks.updateMessageFlags).toHaveBeenCalledWith("message-1", true));
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

  it("shows the account badge right before the date, named once for screen readers", () => {
    const { container } = render(
      <MessageItem
        message={makeMessage()}
        isSelected={false}
        onClick={vi.fn()}
        account={{
          id: "work",
          email: "work@example.com",
          display_name: "Work",
          color: "#10b981",
          account_label: "Work",
        }}
      />,
    );

    const badge = screen.getByRole("img", { name: "Work" });
    expect(badge.className).toContain("account-badge--xs");
    expect(badge.getAttribute("title")).toBe("Work");
    expect(badge.textContent).toBe("W");
    // Badge sits right before the date in the meta area.
    expect(badge.nextElementSibling?.className).toContain("mail-row__date");
    // No leftover colour strip, and the account is not named twice.
    expect(container.querySelectorAll('[aria-label="Work"]')).toHaveLength(1);
    expect(container.querySelector('[style*="width: 3px"]')).toBeNull();
  });

  it("renders no account badge without an account", () => {
    const { container } = render(
      <MessageItem message={makeMessage()} isSelected={false} onClick={vi.fn()} />,
    );

    expect(container.querySelector(".account-badge")).toBeNull();
  });

  it("marks unread rows with a row class", () => {
    render(
      <MessageItem
        message={makeMessage({ is_read: false })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    expect(screen.getByRole("option").className).toContain("message-list-row--unread");
  });

  it("puts read and unread weight on the row class with titles for truncated text", () => {
    render(
      <MessageItem
        message={makeMessage({ is_read: true })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    expect(screen.getByText("Sender").closest(".mail-row__primary")).toBeTruthy();
    expect(screen.getByText("Sender").getAttribute("title")).toBe("Sender");
    expect(screen.getByText("Archived message").className).toContain("mail-row__subject");
    expect(screen.getByText("Archived message").getAttribute("title")).toBe("Archived message");
    expect(screen.getByRole("option").className).not.toContain("message-list-row--unread");
  });

  it("shows recipients as the primary contact in the sent folder", () => {
    render(
      <MessageItem
        message={makeMessage({
          from_name: "Current Account",
          from_address: "current@example.com",
          to_list: [{ name: "Destination", address: "destination@example.com" }],
        })}
        isSelected={false}
        onClick={vi.fn()}
        folderRole="sent"
      />,
    );

    expect(screen.getByText("Destination")).toBeTruthy();
    expect(screen.queryByText("Current Account")).toBeNull();
  });

  it("does not add unread row treatment to read rows", () => {
    render(
      <MessageItem
        message={makeMessage({ is_read: true })}
        isSelected={false}
        onClick={vi.fn()}
      />,
    );

    expect(screen.getByRole("option").className).not.toContain("message-list-row--unread");
  });
});
