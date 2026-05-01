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
  batchArchive: vi.fn(),
  batchDelete: vi.fn(),
  batchMarkRead: vi.fn(),
  batchStar: vi.fn(),
  addToast: vi.fn(),
  confirm: vi.fn(),
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
  useQueryClient: () => mocks.queryClient,
}));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 76,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        key: `row-${index}`,
        start: index * 76,
      })),
    measureElement: vi.fn(),
    scrollToIndex: vi.fn(),
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
}));

vi.mock("../../src/stores/confirm.store", () => ({
  useConfirmStore: (selector: (state: { confirm: () => Promise<boolean> }) => unknown) =>
    selector({ confirm: mocks.confirm }),
}));

vi.mock("../../src/components/MessageItem", () => ({
  default: ({ message, folderRole, accountColor, accountLabel }: {
    message: MessageSummary;
    folderRole?: string | null;
    accountColor?: string;
    accountLabel?: string;
  }) => (
    <div
      data-testid={`message-${message.id}`}
      data-folder-role={folderRole ?? ""}
      data-account-color={accountColor ?? ""}
      data-account-label={accountLabel ?? ""}
    >
      {message.subject}
    </div>
  ),
}));

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

  it("passes account color metadata to message items", () => {
    const message = { ...makeMessage("m-1"), account_id: "account-2" };
    useMailStore.setState({ activeAccountId: null });

    render(
      <MessageList
        messages={[message]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    const row = screen.getByTestId("message-m-1");

    expect(row.getAttribute("data-account-color")).toBe("#3b82f6");
    expect(row.getAttribute("data-account-label")).toBe("Two <two@example.com>");
  });

  it("does not pass account color metadata when a single account is selected", () => {
    const message = { ...makeMessage("m-1"), account_id: "account-1" };

    render(
      <MessageList
        messages={[message]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    const row = screen.getByTestId("message-m-1");

    expect(row.getAttribute("data-account-color")).toBe("");
  });

  it("does not pass account color metadata when there is only one account", () => {
    mocks.accounts = [
      {
        id: "account-1",
        email: "one@example.com",
        display_name: "One",
        color: "#22c55e",
      },
    ];

    render(
      <MessageList
        messages={[makeMessage("m-1")]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    const row = screen.getByTestId("message-m-1");

    expect(row.getAttribute("data-account-color")).toBe("");
  });

  it("derives a stable account color when the account has no saved color", () => {
    const message = { ...makeMessage("m-1"), account_id: "missing-account" };
    useMailStore.setState({ activeAccountId: null });

    render(
      <MessageList
        messages={[message]}
        selectedMessageId={null}
        onSelectMessage={vi.fn()}
        loading={false}
      />,
    );

    const color = screen.getByTestId("message-m-1").getAttribute("data-account-color");

    expect(color).toMatch(/^#[0-9a-f]{6}$/);
  });

  it("uses different default colors for known accounts without saved colors", () => {
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
});
