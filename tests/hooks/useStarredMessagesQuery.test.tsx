import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageSummary } from "../../src/lib/api";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import {
  STARRED_MESSAGES_PAGE_SIZE,
  starredMessagesQueryKey,
  useStarredMessagesQuery,
} from "../../src/hooks/queries/useStarredMessagesQuery";

const mockInvoke = vi.mocked(invoke);

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

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });

  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

describe("useStarredMessagesQuery", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("uses a stable starred query key", () => {
    expect(starredMessagesQueryKey("account-1")).toEqual(["starred-messages", "account-1"]);
  });

  it("fetches starred messages with page-size offsets", async () => {
    const firstPage = Array.from({ length: STARRED_MESSAGES_PAGE_SIZE }, (_, index) =>
      makeMessage(`m-${index + 1}`),
    );
    const secondPage = [makeMessage("m-51")];
    mockInvoke.mockResolvedValueOnce(firstPage).mockResolvedValueOnce(secondPage);

    const { result } = renderHook(() => useStarredMessagesQuery("account-1"), {
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.data).toHaveLength(STARRED_MESSAGES_PAGE_SIZE));
    expect(mockInvoke).toHaveBeenCalledWith("list_starred_messages", {
      accountId: "account-1",
      limit: STARRED_MESSAGES_PAGE_SIZE,
      offset: 0,
    });
    expect(result.current.hasNextPage).toBe(true);

    await act(async () => {
      await result.current.fetchNextPage();
    });

    await waitFor(() => expect(result.current.data).toHaveLength(STARRED_MESSAGES_PAGE_SIZE + 1));
    expect(mockInvoke).toHaveBeenLastCalledWith("list_starred_messages", {
      accountId: "account-1",
      limit: STARRED_MESSAGES_PAGE_SIZE,
      offset: STARRED_MESSAGES_PAGE_SIZE,
    });
    expect(result.current.hasNextPage).toBe(false);
  });

  it("subtracts locally removed starred rows from the next offset", async () => {
    const firstPage = Array.from({ length: STARRED_MESSAGES_PAGE_SIZE }, (_, index) =>
      makeMessage(`m-${index + 1}`),
    );
    const secondPage = [makeMessage("m-50"), makeMessage("m-51")];
    mockInvoke.mockResolvedValueOnce(firstPage).mockResolvedValueOnce(secondPage);

    const { result } = renderHook(() => useStarredMessagesQuery("account-1", 1), {
      wrapper: createWrapper(),
    });

    await waitFor(() => expect(result.current.data).toHaveLength(STARRED_MESSAGES_PAGE_SIZE));

    await act(async () => {
      await result.current.fetchNextPage();
    });

    expect(mockInvoke).toHaveBeenLastCalledWith("list_starred_messages", {
      accountId: "account-1",
      limit: STARRED_MESSAGES_PAGE_SIZE,
      offset: STARRED_MESSAGES_PAGE_SIZE - 1,
    });
  });
});
