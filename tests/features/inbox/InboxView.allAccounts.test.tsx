import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Account, Folder } from "../../../src/lib/api";

const mocks = vi.hoisted(() => ({
  mailState: {
    activeAccountId: null as string | null,
    activeFolderId: "all:inbox" as string | null,
    selectedMessageId: null as string | null,
    selectedThreadId: null as string | null,
    threadView: false,
    unreadOnly: false,
    selectedThreadAccountId: null as string | null,
    unifiedAccountFilter: null as string | null,
    setUnifiedAccountFilter: vi.fn(),
    selectThread: vi.fn(),
    setSelectedMessage: vi.fn(),
    setSelectedThreadId: vi.fn(),
    toggleThreadView: vi.fn(),
  },
  accounts: [
    { id: "account-1", email: "one@example.com" },
    { id: "account-2", email: "two@example.com" },
  ] as Account[],
  folders: [
    { id: "a1-inbox", account_id: "account-1", role: "inbox", sort_order: 0 },
    { id: "a2-inbox", account_id: "account-2", role: "inbox", sort_order: 0 },
  ] as Folder[],
  useFoldersForAccountsQuery: vi.fn(),
  useMessagesQuery: vi.fn(),
  useThreadsQuery: vi.fn(),
  unreadCounts: { "a1-inbox": 3, "a2-inbox": 5 } as Record<string, number>,
}));

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({
    invalidateQueries: vi.fn(),
  }),
}));

vi.mock("../../../src/stores/mail.store", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../src/stores/mail.store")>()),
  useMailStore: (selector: (state: typeof mocks.mailState) => unknown) => selector(mocks.mailState),
}));

vi.mock("../../../src/stores/ui.store", () => ({
  useUIStore: (selector: (state: { setActiveView: () => void; showFolderUnreadCount: boolean }) => unknown) =>
    selector({ setActiveView: vi.fn(), showFolderUnreadCount: true }),
}));

vi.mock("../../../src/hooks/queries/useFolderUnreadCounts", () => ({
  useFolderUnreadCountsForAccounts: () => ({ data: mocks.unreadCounts, isLoading: false }),
}));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 72,
    getVirtualItems: () => Array.from({ length: count }, (_, index) => ({ index, key: index, start: index * 72 })),
    measureElement: vi.fn(),
    scrollToIndex: vi.fn(),
  }),
}));

vi.mock("../../../src/stores/toast.store", () => ({
  useToastStore: (selector: (state: { addToast: () => void }) => unknown) =>
    selector({ addToast: vi.fn() }),
}));

vi.mock("../../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({ data: mocks.accounts }),
  useFoldersQuery: () => ({
    data: mocks.folders.slice(0, 1),
    isFetched: true,
  }),
  useFoldersForAccountsQuery: mocks.useFoldersForAccountsQuery,
  useMessagesQuery: mocks.useMessagesQuery,
  useThreadsQuery: mocks.useThreadsQuery,
  patchMessagesCache: vi.fn(),
  onMessagesCachePatched: () => () => {},
}));

vi.mock("../../../src/components/SearchBar", () => ({
  default: () => <div>Search bar</div>,
}));

vi.mock("../../../src/components/MessageList", () => ({
  default: ({ spansAccounts }: { spansAccounts?: boolean }) => (
    <div data-testid="message-list" data-spans-accounts={String(Boolean(spansAccounts))}>Message list</div>
  ),
}));

vi.mock("../../../src/components/MessageDetail", () => ({
  default: () => <div>Message detail</div>,
}));

vi.mock("../../../src/features/inbox/ThreadView", () => ({
  default: () => <div>Thread detail</div>,
}));

vi.mock("../../../src/components/ConfirmDialog", () => ({
  default: () => <div>Confirm dialog</div>,
}));

vi.mock("../../../src/lib/api", () => ({
  emptyTrash: vi.fn(),
}));

import InboxView from "../../../src/features/inbox/InboxView";

describe("InboxView all accounts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.mailState.activeAccountId = null;
    mocks.mailState.activeFolderId = "all:inbox";
    mocks.mailState.selectedMessageId = null;
    mocks.mailState.selectedThreadId = null;
    mocks.mailState.threadView = false;
    mocks.mailState.unifiedAccountFilter = null;
    mocks.useFoldersForAccountsQuery.mockReturnValue({
      data: mocks.folders,
      isFetched: true,
    });
    mocks.useMessagesQuery.mockReturnValue({
      data: [],
      isLoading: false,
      hasNextPage: false,
      isFetchingNextPage: false,
      fetchNextPage: vi.fn(),
    });
    mocks.useThreadsQuery.mockReturnValue({
      data: [],
      isLoading: false,
    });
  });

  it("queries all matching inbox folders when all accounts are selected", () => {
    render(<InboxView />);

    expect(mocks.useFoldersForAccountsQuery).toHaveBeenCalledWith(["account-1", "account-2"]);
    expect(mocks.useMessagesQuery).toHaveBeenCalledWith("a1-inbox", ["a1-inbox", "a2-inbox"], false);
  });

  it("shows the account filter with unread counts in the unified view", () => {
    render(<InboxView />);

    const group = screen.getByRole("radiogroup", { name: "Filter by account" });
    const radios = screen.getAllByRole("radio");
    expect(group).toBeTruthy();
    expect(radios.map((radio) => radio.textContent)).toEqual(["All8", "Oone@example.com3", "Ttwo@example.com5"]);
    expect(radios[0].getAttribute("aria-checked")).toBe("true");
    expect(radios[0].tabIndex).toBe(0);
    expect(radios[1].tabIndex).toBe(-1);
    expect(screen.getByTestId("message-list").getAttribute("data-spans-accounts")).toBe("true");
  });

  it("restricts the unified query to the chosen account's folders", () => {
    mocks.mailState.unifiedAccountFilter = "account-2";

    render(<InboxView />);

    expect(mocks.useMessagesQuery).toHaveBeenLastCalledWith("a2-inbox", undefined, false);
    expect(screen.getAllByRole("radio")[2].getAttribute("aria-checked")).toBe("true");
    expect(screen.getByTestId("message-list").getAttribute("data-spans-accounts")).toBe("false");
  });

  it("ignores a filter for an account that no longer exists", () => {
    mocks.mailState.unifiedAccountFilter = "gone";

    render(<InboxView />);

    expect(mocks.useMessagesQuery).toHaveBeenLastCalledWith("a1-inbox", ["a1-inbox", "a2-inbox"], false);
  });

  it("selects an account by click and moves with arrow keys", () => {
    render(<InboxView />);
    const radios = screen.getAllByRole("radio");

    fireEvent.click(radios[1]);
    expect(mocks.mailState.setUnifiedAccountFilter).toHaveBeenLastCalledWith("account-1");

    fireEvent.keyDown(radios[0], { key: "ArrowRight" });
    expect(mocks.mailState.setUnifiedAccountFilter).toHaveBeenLastCalledWith("account-1");
    fireEvent.keyDown(radios[0], { key: "End" });
    expect(mocks.mailState.setUnifiedAccountFilter).toHaveBeenLastCalledWith("account-2");
    fireEvent.keyDown(radios[0], { key: "ArrowLeft" });
    expect(mocks.mailState.setUnifiedAccountFilter).toHaveBeenLastCalledWith("account-2");
  });

  it("hides the account filter outside the unified view", () => {
    mocks.mailState.activeAccountId = "account-1";
    mocks.mailState.activeFolderId = "a1-inbox";

    render(<InboxView />);

    expect(screen.queryByRole("radiogroup")).toBeNull();
    expect(screen.getByTestId("message-list").getAttribute("data-spans-accounts")).toBe("false");
  });

  it("badges unified thread rows by account", () => {
    mocks.mailState.threadView = true;
    mocks.useThreadsQuery.mockReturnValue({
      data: [
        { thread_id: "t-1", account_id: "account-1", subject: "One", snippet: "", last_date: 1, message_count: 1, unread_count: 0, is_starred: false, participants: ["a"], has_attachments: false },
        { thread_id: "t-2", account_id: "account-2", subject: "Two", snippet: "", last_date: 1, message_count: 1, unread_count: 0, is_starred: false, participants: ["b"], has_attachments: false },
      ],
      isLoading: false,
    });

    render(<InboxView />);

    expect(screen.getByRole("img", { name: "one@example.com" })).toBeTruthy();
    expect(screen.getByRole("img", { name: "two@example.com" })).toBeTruthy();
  });
});
