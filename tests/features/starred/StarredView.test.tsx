import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageSummary } from "../../../src/lib/api";

const fetchNextPage = vi.fn();
const setSelectedMessage = vi.fn((messageId: string | null) => {
  mockMailState.selectedMessageId = messageId;
});
const useStarredMessagesQuery = vi.fn();
const listStarredMessages = vi.fn();

const mockMailState = {
  activeAccountId: "account-1",
  selectedMessageId: null as string | null,
  setSelectedMessage,
};

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
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
  }),
}));

vi.mock("../../../src/hooks/queries/useStarredMessagesQuery", () => ({
  useStarredMessagesQuery: (...args: unknown[]) => useStarredMessagesQuery(...args),
}));

vi.mock("../../../src/lib/api", () => ({
  listStarredMessages: (...args: unknown[]) => listStarredMessages(...args),
}));

vi.mock("../../../src/stores/mail.store", () => ({
  useMailStore: (selector: (state: typeof mockMailState) => unknown) => selector(mockMailState),
}));

vi.mock("../../../src/components/MessageItem", () => ({
  default: ({
    message,
    onClick,
    onToggleStar,
  }: {
    message: MessageSummary;
    onClick: () => void;
    onToggleStar?: (messageId: string, newStarred: boolean) => void;
  }) => (
    <div data-testid={`message-${message.id}`}>
      <button onClick={onClick}>{message.subject}</button>
      <button onClick={() => onToggleStar?.(message.id, false)}>unstar</button>
    </div>
  ),
}));

vi.mock("../../../src/components/MessageDetail", () => ({
  default: ({ messageId }: { messageId: string }) => <div data-testid="message-detail">{messageId}</div>,
}));

import StarredView from "../../../src/features/starred/StarredView";

function makeMessage(id: string): MessageSummary {
  return {
    id,
    account_id: "account-1",
    remote_id: `remote-${id}`,
    thread_id: `thread-${id}`,
    subject: `Subject ${id}`,
    snippet: `Snippet ${id}`,
    from_address: "sender@example.com",
    from_name: "Sender",
    to_list: [],
    cc_list: [],
    bcc_list: [],
    has_attachments: false,
    is_read: true,
    is_starred: true,
    is_draft: false,
    date: 1_700_000_000,
    remote_version: null,
    is_deleted: false,
    deleted_at: null,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
  };
}

describe("StarredView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockMailState.activeAccountId = "account-1";
    mockMailState.selectedMessageId = null;
    mockMailState.setSelectedMessage = setSelectedMessage;
  });

  it("shows Load More for a full first page and fetches the next page when clicked", () => {
    const messages = Array.from({ length: 50 }, (_, index) => makeMessage(`m-${index + 1}`));
    useStarredMessagesQuery.mockReturnValue({
      data: messages,
      loading: false,
      error: null,
      hasNextPage: true,
      isFetchingNextPage: false,
      fetchNextPage,
    });

    render(<StarredView />);

    expect(useStarredMessagesQuery).toHaveBeenCalledWith("account-1", 0);
    expect(screen.getByRole("button", { name: "Load More" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Load More" }));

    expect(fetchNextPage).toHaveBeenCalledTimes(1);
  });

  it("clears the selected detail when the starred message is unstarred", () => {
    const messages = [makeMessage("m-1")];
    useStarredMessagesQuery.mockReturnValue({
      data: messages,
      loading: false,
      error: null,
      hasNextPage: false,
      isFetchingNextPage: false,
      fetchNextPage,
    });

    render(<StarredView />);

    fireEvent.click(screen.getByRole("button", { name: "Subject m-1" }));

    expect(screen.getByTestId("message-detail").textContent).toBe("m-1");

    fireEvent.click(screen.getByRole("button", { name: "unstar" }));

    expect(screen.queryByTestId("message-m-1")).toBeNull();
    expect(screen.queryByTestId("message-detail")).toBeNull();
  });

  it("does not open a stale globally selected message when entering starred view", () => {
    mockMailState.selectedMessageId = "stale-inbox-message";
    useStarredMessagesQuery.mockReturnValue({
      data: [makeMessage("m-1")],
      loading: false,
      error: null,
      hasNextPage: false,
      isFetchingNextPage: false,
      fetchNextPage,
    });

    render(<StarredView />);

    expect(screen.queryByTestId("message-detail")).toBeNull();
  });

  it("keeps Load More available when all currently loaded starred messages are unstarred", () => {
    useStarredMessagesQuery.mockReturnValue({
      data: [makeMessage("m-1")],
      loading: false,
      error: null,
      hasNextPage: true,
      isFetchingNextPage: false,
      fetchNextPage,
    });

    render(<StarredView />);

    fireEvent.click(screen.getByRole("button", { name: "unstar" }));

    expect(screen.queryByText("No starred messages")).toBeNull();
    expect(screen.getByRole("button", { name: "Load More" })).toBeTruthy();
  });

  it("resets local selection and removal state when the active account changes", () => {
    useStarredMessagesQuery.mockReturnValue({
      data: [makeMessage("m-1")],
      loading: false,
      error: null,
      hasNextPage: true,
      isFetchingNextPage: false,
      fetchNextPage,
    });
    const { rerender } = render(<StarredView />);
    fireEvent.click(screen.getByRole("button", { name: "Subject m-1" }));
    fireEvent.click(screen.getByRole("button", { name: "unstar" }));

    mockMailState.activeAccountId = "account-2";
    rerender(<StarredView />);

    expect(screen.queryByTestId("message-detail")).toBeNull();
    expect(useStarredMessagesQuery).toHaveBeenLastCalledWith("account-2", 0);
  });

  it("keeps loaded messages visible when fetching another starred page fails", () => {
    useStarredMessagesQuery.mockReturnValue({
      data: [makeMessage("m-1")],
      loading: false,
      error: new Error("next page failed"),
      hasNextPage: true,
      isFetchingNextPage: false,
      fetchNextPage,
    });

    render(<StarredView />);

    expect(screen.getByTestId("message-m-1")).toBeTruthy();
    expect(screen.queryByText("Failed to load starred messages")).toBeNull();
  });
});
