import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Folder, MessageSummary } from "../../src/lib/api";
import { useMailStore } from "../../src/stores/mail.store";

const mocks = vi.hoisted(() => ({
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
  useFoldersQuery: () => ({ data: mocks.folders }),
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
  default: ({ message, folderRole }: { message: MessageSummary; folderRole?: string | null }) => (
    <div data-testid={`message-${message.id}`} data-folder-role={folderRole ?? ""}>
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
});
