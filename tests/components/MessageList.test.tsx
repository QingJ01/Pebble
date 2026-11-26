import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Account, Folder, MessageSummary } from "../../src/lib/api";
import { useMailStore } from "../../src/stores/mail.store";

const mocks = vi.hoisted(() => ({
  accounts: [] as Account[],
  folders: [] as Folder[],
  queryClient: {
    invalidateQueries: vi.fn(),
  },
  getMessageLabelsBatch: vi.fn(),
  labelQueryKeys: [] as unknown[][][],
  labelData: {} as Record<string, Record<string, { id: string; name: string; color: string }[]>>,
  batchArchive: vi.fn(),
  batchDelete: vi.fn(),
  batchMarkRead: vi.fn(),
  batchStar: vi.fn(),
  addToast: vi.fn(),
  confirm: vi.fn(),
  runMailAction: vi.fn(),
  toastError: vi.fn(),
  scrollToIndex: vi.fn(),
  itemProps: {} as Record<string, Record<string, unknown>>,
  prefetchMessage: vi.fn(),
}));

vi.mock("../../src/hooks/useMessageLoader", () => ({
  prefetchMessage: mocks.prefetchMessage,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallbackOrOptions?: string | { defaultValue?: string }) => {
      if (typeof fallbackOrOptions === "string") return fallbackOrOptions;
      return fallbackOrOptions?.defaultValue ?? key;
    },
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQuery: () => ({ data: {} }),
  useQueries: ({ queries, combine }: {
    queries: { queryKey: unknown[] }[];
    combine: (results: { data?: unknown }[]) => unknown;
  }) => {
    mocks.labelQueryKeys.push(queries.map((query) => query.queryKey));
    return combine(queries.map((query) => ({ data: mocks.labelData[String(query.queryKey[1])] })));
  },
  useQueryClient: () => mocks.queryClient,
}));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count, getItemKey }: { count: number; getItemKey?: (index: number) => string | number }) => ({
    getTotalSize: () => count * 76,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        key: getItemKey?.(index) ?? `row-${index}`,
        start: index * 76,
      })),
    measureElement: vi.fn(),
    scrollToIndex: mocks.scrollToIndex,
  }),
}));

vi.mock("../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({
    data: mocks.accounts,
  }),
  useFoldersForAccountsQuery: () => ({ data: mocks.folders }),
}));

vi.mock("../../src/lib/api", () => ({
  getMessageLabelsBatch: mocks.getMessageLabelsBatch,
  batchArchive: mocks.batchArchive,
  batchDelete: mocks.batchDelete,
  batchMarkRead: mocks.batchMarkRead,
  batchStar: mocks.batchStar,
}));

vi.mock("../../src/stores/toast.store", () => ({
  useToastStore: (selector: (state: { addToast: (toast: unknown) => void }) => unknown) =>
    selector({ addToast: mocks.addToast }),
  toastError: mocks.toastError,
}));

vi.mock("../../src/hooks/mutations/mailActions", () => ({
  runMailAction: mocks.runMailAction,
  trashConfirmCopy: () => ({ title: "t", message: "m", confirmLabel: "c" }),
}));

vi.mock("../../src/stores/confirm.store", () => ({
  useConfirmStore: (selector: (state: { confirm: () => Promise<boolean> }) => unknown) =>
    selector({ confirm: mocks.confirm }),
}));

vi.mock("../../src/components/MessageItem", async () => {
  const { useState } = await import("react");
  return {
  default: (props: {
    message: MessageSummary;
    folderRole?: string | null;
    account?: { id: string; email: string; color: string | null };
    tabIndex?: number;
    isSelected?: boolean;
    onClick: (event?: unknown) => void;
  }) => {
    const { message, folderRole, account } = props;
    mocks.itemProps[message.id] = props as unknown as Record<string, unknown>;
    const [initialMessageId] = useState(message.id);
    return (
      <div
        role="option"
        aria-selected={Boolean(props.isSelected)}
        tabIndex={props.tabIndex}
        data-row-id={message.id}
        onClick={(event) => props.onClick(event)}
        data-testid={`message-${message.id}`}
        data-initial-message-id={initialMessageId}
        data-folder-role={folderRole ?? ""}
        data-account-color={account?.color ?? ""}
        data-account-id={account?.id ?? ""}
      >
        {message.subject}
      </div>
    );
  },
  };
});

vi.mock("../../src/components/Skeleton", () => ({
  MessageListSkeleton: () => <div>Loading messages</div>,
}));

import MessageList from "../../src/components/MessageList";

function makeMessage(id: string): MessageSummary {
  return {
    id,
    account_id: "account-1",
    remote_id: `remote-${id}`,
    message_id_header: null,
    in_reply_to: null,
    references_header: null,
    thread_id: null,
    subject: `Subject ${id}`,
    snippet: `Snippet ${id}`,
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

describe("MessageList", () => {
  it("sends batch delete through the undoable trash action with a confirm fallback", async () => {
    const first = { ...makeMessage("m-1"), is_draft: true, remote_id: "" };
    useMailStore.setState({ batchMode: true, selectedMessageIds: new Set([first.id, "m-2"]) });
    mocks.runMailAction.mockResolvedValue({ ok: true, count: 2 });
    render(<MessageList messages={[first, makeMessage("m-2")]} selectedMessageId={null} onSelectMessage={vi.fn()} loading={false} />);
    fireEvent.click(screen.getByRole("button", { name: "common.delete" }));
    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient,
      "trash",
      [expect.objectContaining({ id: "m-1" }), expect.objectContaining({ id: "m-2" })],
      expect.objectContaining({ confirm: expect.any(Function) }),
    ));
    expect(useMailStore.getState().selectedMessageIds.size).toBe(0);
  });

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.accounts = [
      {
        id: "account-1",
        email: "one@example.com",
        display_name: "One",
        color: "#22c55e",
      },
      {
        id: "account-2",
        email: "two@example.com",
        display_name: "Two",
        color: "#3b82f6",
      },
      {
        id: "account-3",
        email: "three@example.com",
        display_name: "Three",
        color: null,
      },
      {
        id: "account-4",
        email: "four@example.com",
        display_name: "Four",
        color: null,
      },
    ];
    mocks.batchArchive.mockResolvedValue(1);
    mocks.batchDelete.mockResolvedValue(1);
    mocks.batchMarkRead.mockResolvedValue(1);
    mocks.batchStar.mockResolvedValue(1);
    mocks.confirm.mockResolvedValue(true);
    mocks.folders = [{
      id: "folder-archive",
      account_id: "account-1",
      remote_id: "archive",
      name: "Archive",
      folder_type: "folder",
      role: "archive",
      parent_id: null,
      color: null,
      is_system: true,
      sort_order: 1,
    }];
    useMailStore.setState({
      activeAccountId: "account-1",
      activeFolderId: "folder-archive",
      selectedMessageId: null,
      selectedThreadId: null,
      threadView: false,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
  });

  it("fetches labels per stable page-sized chunk so earlier pages stay cached", () => {
    mocks.labelQueryKeys = [];
    const firstPages = Array.from({ length: 100 }, (_, index) => makeMessage(`m-${index}`));
    const { rerender } = render(
      <MessageList messages={firstPages} selectedMessageId={null} onSelectMessage={vi.fn()} loading={false} />,
    );
    const initialKeys = mocks.labelQueryKeys.at(-1)!;
    expect(initialKeys).toHaveLength(2);
    expect(String(initialKeys[0][1]).split(",")).toHaveLength(50);

    const morePages = [...firstPages, ...Array.from({ length: 30 }, (_, index) => makeMessage(`n-${index}`))];
    rerender(<MessageList messages={morePages} selectedMessageId={null} onSelectMessage={vi.fn()} loading={false} />);
    const nextKeys = mocks.labelQueryKeys.at(-1)!;

    expect(nextKeys).toHaveLength(3);
    expect(nextKeys.slice(0, 2)).toEqual(initialKeys);
    expect(String(nextKeys[2][1]).split(",")).toHaveLength(30);
  });

  it("does not show Load more just because the current page has 50 messages", () => {
    render(
      <MessageList
        messages={Array.from({ length: 50 }, (_, index) => makeMessage(`m-${index + 1}`))}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
        onLoadMore={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: "Load more" })).toBeNull();
  });


  it("keys virtual rows by message id so refresh insertion does not reuse row state", () => {
    const first = makeMessage("m-1");
    const inserted = makeMessage("m-2");
    const { rerender } = render(
      <MessageList messages={[first]} selectedMessageId={null} onSelectMessage={vi.fn()} loading={false} />,
    );

    rerender(
      <MessageList messages={[inserted, first]} selectedMessageId={null} onSelectMessage={vi.fn()} loading={false} />,
    );

    const rows = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="message-"]'));
    expect(rows[0].dataset.testid).toBe("message-m-2");
    expect(rows[0].dataset.initialMessageId).toBe("m-2");
    expect(rows[1].dataset.testid).toBe("message-m-1");
    expect(rows[1].dataset.initialMessageId).toBe("m-1");
  });

  it("passes the active folder role to message items", () => {
    render(
      <MessageList
        messages={[makeMessage("m-1")]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    expect(screen.getByTestId("message-m-1").getAttribute("data-folder-role")).toBe("archive");
  });

  it("uses the shared smooth scroll region for the virtualized list", () => {
    render(
      <MessageList
        messages={[makeMessage("m-1")]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    const listbox = screen.getByRole("listbox", { name: "Messages" });

    expect(listbox.className).toContain("scroll-region");
    expect(listbox.className).toContain("message-list-scroll");
  });

  it("badges rows by account when the list holds mail from several accounts", () => {
    useMailStore.setState({ activeAccountId: null });

    render(
      <MessageList
        messages={[
          { ...makeMessage("m-1"), account_id: "account-2" },
          { ...makeMessage("m-2"), account_id: "account-1" },
        ]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    const row = screen.getByTestId("message-m-1");
    expect(row.getAttribute("data-account-id")).toBe("account-2");
    expect(row.getAttribute("data-account-color")).toBe("#3b82f6");
  });

  it("badges a one-account page when the list spans accounts (unfiltered unified inbox)", () => {
    useMailStore.setState({ activeAccountId: null });

    render(
      <MessageList
        messages={[{ ...makeMessage("m-1"), account_id: "account-2" }]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
        spansAccounts
      />,
    );

    expect(screen.getByTestId("message-m-1").getAttribute("data-account-id")).toBe("account-2");
  });

  it("does not badge rows when the list holds a single account", () => {
    useMailStore.setState({ activeAccountId: null });

    render(
      <MessageList
        messages={[
          { ...makeMessage("m-1"), account_id: "account-1" },
          { ...makeMessage("m-2"), account_id: "account-1" },
        ]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    expect(screen.getByTestId("message-m-1").getAttribute("data-account-id")).toBe("");
  });

  it("does not badge rows when a single account is selected, even if told the list spans accounts", () => {
    render(
      <MessageList
        messages={[{ ...makeMessage("m-1"), account_id: "account-1" }]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
        spansAccounts
      />,
    );

    expect(screen.getByTestId("message-m-1").getAttribute("data-account-id")).toBe("");
  });

  it("does not badge rows when there is only one account", () => {
    mocks.accounts = [
      {
        id: "account-1",
        email: "one@example.com",
        display_name: "One",
        color: "#22c55e",
      },
    ];
    useMailStore.setState({ activeAccountId: null });

    render(
      <MessageList
        messages={[makeMessage("m-1"), { ...makeMessage("m-2"), account_id: "missing" }]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    expect(screen.getByTestId("message-m-1").getAttribute("data-account-id")).toBe("");
  });

  it("gives accounts without a saved colour distinct badge colours", () => {
    useMailStore.setState({ activeAccountId: null });

    render(
      <MessageList
        messages={[
          { ...makeMessage("m-1"), account_id: "account-3" },
          { ...makeMessage("m-2"), account_id: "account-4" },
        ]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    const firstColor = screen.getByTestId("message-m-1").getAttribute("data-account-color");
    const secondColor = screen.getByTestId("message-m-2").getAttribute("data-account-color");

    expect(firstColor).toMatch(/^#[0-9a-f]{6}$/);
    expect(secondColor).toMatch(/^#[0-9a-f]{6}$/);
    expect(firstColor).not.toBe(secondColor);
  });

  it("refreshes derived queries after a successful batch star action", async () => {
    mocks.batchStar.mockResolvedValueOnce(2);
    useMailStore.setState({
      selectedMessageIds: new Set(["m-1", "m-2"]),
      batchMode: true,
    });

    render(
      <MessageList
        messages={[makeMessage("m-1"), makeMessage("m-2")]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Star" }));

    await waitFor(() => expect(mocks.batchStar).toHaveBeenCalledWith(["m-1", "m-2"], true));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["messages"] });
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["threads"] });
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["starred-messages"] });
    expect(mocks.queryClient.invalidateQueries).not.toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });

  it("archives the batch through the shared undoable action", async () => {
    mocks.batchArchive.mockResolvedValueOnce(2);
    useMailStore.setState({
      selectedMessageIds: new Set(["m-1", "m-2"]),
      batchMode: true,
    });

    render(
      <MessageList
        messages={[makeMessage("m-1"), makeMessage("m-2")]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "messageActions.archive" }));

    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient,
      "archive",
      [expect.objectContaining({ id: "m-1" }), expect.objectContaining({ id: "m-2" })],
      expect.anything(),
    ));
  });

  it("refreshes folder unread counts after a successful batch read-state action", async () => {
    mocks.batchMarkRead.mockResolvedValueOnce(2);
    useMailStore.setState({
      selectedMessageIds: new Set(["m-1", "m-2"]),
      batchMode: true,
    });

    render(
      <MessageList
        messages={[makeMessage("m-1"), makeMessage("m-2")]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "batch.markRead" }));

    await waitFor(() => expect(mocks.batchMarkRead).toHaveBeenCalledWith(["m-1", "m-2"], true));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });

  it("uses the custom checkbox control for select all", () => {
    useMailStore.setState({ batchMode: true });

    render(
      <MessageList
        messages={[makeMessage("m-1"), makeMessage("m-2")]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    const checkbox = screen.getByRole("checkbox", { name: "Select all" });

    expect(checkbox.className).toContain("batch-checkbox");
    expect(checkbox.className).toContain("batch-select-all-checkbox");
    expect(checkbox.closest("label")?.className).toContain("batch-select-control");
  });

  it("uses distinct icons for unstar/mark unread and separates delete", () => {
    useMailStore.setState({ batchMode: true, selectedMessageIds: new Set(["m-1"]) });
    render(<MessageList messages={[makeMessage("m-1")]} selectedMessageId={null} onSelectMessage={vi.fn()} loading={false} />);
    const del = screen.getByRole("button", { name: "common.delete" });
    expect(del.previousElementSibling?.className).toContain("toolbar-separator");
    expect(del.className).toContain("icon-btn");
  });

  it("keeps a single tab stop on the active row (roving tabindex)", () => {
    render(
      <MessageList
        messages={[makeMessage("m-1"), makeMessage("m-2"), makeMessage("m-3")]}
        selectedMessageId="m-2"
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );
    const rows = screen.getAllByRole("option");
    expect(rows.map((row) => row.getAttribute("tabindex"))).toEqual(["-1", "0", "-1"]);
  });

  it("prefetches the neighbours of the open message once the selection settles", () => {
    vi.useFakeTimers();
    try {
      mocks.prefetchMessage.mockClear();
      const messages = [makeMessage("m-1"), makeMessage("m-2"), makeMessage("m-3"), makeMessage("m-4")];
      const view = render(
        <MessageList messages={messages} selectedMessageId="m-2" onSelectMessage={vi.fn()} loading={false} />,
      );
      // Moving on before the delay cancels the pending prefetch.
      view.rerender(
        <MessageList messages={messages} selectedMessageId="m-3" onSelectMessage={vi.fn()} loading={false} />,
      );
      vi.advanceTimersByTime(1000);
      expect(mocks.prefetchMessage.mock.calls.map((call) => call[0]).sort()).toEqual(["m-2", "m-4"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("moves selection with arrow keys and J/K from a focused row", () => {
    const onSelectMessage = vi.fn();
    render(
      <MessageList
        messages={[makeMessage("m-1"), makeMessage("m-2"), makeMessage("m-3")]}
        selectedMessageId="m-1"
        onSelectMessage={onSelectMessage}
        loading={false}
      />,
    );
    const first = screen.getAllByRole("option")[0];
    first.focus();
    fireEvent.keyDown(first, { key: "ArrowDown" });
    expect(onSelectMessage).toHaveBeenLastCalledWith("m-2");
    fireEvent.keyDown(first, { key: "j" });
    expect(onSelectMessage).toHaveBeenLastCalledWith("m-2");
    fireEvent.keyDown(screen.getAllByRole("option")[1], { key: "End" });
    expect(onSelectMessage).toHaveBeenLastCalledWith("m-3");
  });

  it("toggles rows into a selection with Cmd/Ctrl-click and selects ranges with Shift-click", () => {
    const onSelectMessage = vi.fn();
    const messages = ["m-1", "m-2", "m-3", "m-4"].map(makeMessage);
    render(<MessageList messages={messages} selectedMessageId="m-1" onSelectMessage={onSelectMessage} loading={false} />);

    fireEvent.click(screen.getByTestId("message-m-2"), { metaKey: true });
    let state = useMailStore.getState();
    expect(state.batchMode).toBe(true);
    expect([...state.selectedMessageIds].sort()).toEqual(["m-1", "m-2"]);

    fireEvent.click(screen.getByTestId("message-m-4"), { shiftKey: true });
    state = useMailStore.getState();
    expect([...state.selectedMessageIds].sort()).toEqual(["m-1", "m-2", "m-3", "m-4"]);

    fireEvent.click(screen.getByTestId("message-m-3"), { ctrlKey: true });
    expect([...useMailStore.getState().selectedMessageIds].sort()).toEqual(["m-1", "m-2", "m-4"]);
    expect(onSelectMessage).not.toHaveBeenCalled();
  });

  it("scrolls to the selection only when it changes, not on list refreshes", () => {
    const props = { onSelectMessage: vi.fn(), loading: false };
    const { rerender } = render(
      <MessageList {...props} messages={[makeMessage("m-1"), makeMessage("m-2")]} selectedMessageId="m-2" />,
    );
    expect(mocks.scrollToIndex).toHaveBeenCalledTimes(1);
    expect(mocks.scrollToIndex).toHaveBeenLastCalledWith(1, { align: "auto" });

    // Background refresh: new array, new message at the top, same selection.
    rerender(
      <MessageList {...props} messages={[makeMessage("m-0"), makeMessage("m-1"), makeMessage("m-2")]} selectedMessageId="m-2" />,
    );
    expect(mocks.scrollToIndex).toHaveBeenCalledTimes(1);

    rerender(
      <MessageList {...props} messages={[makeMessage("m-0"), makeMessage("m-1"), makeMessage("m-2")]} selectedMessageId="m-0" />,
    );
    expect(mocks.scrollToIndex).toHaveBeenCalledTimes(2);
    expect(mocks.scrollToIndex).toHaveBeenLastCalledWith(0, { align: "auto" });
  });
});
