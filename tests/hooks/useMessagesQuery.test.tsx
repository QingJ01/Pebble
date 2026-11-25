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
  MESSAGES_PAGE_SIZE,
  nextMessagesCursor,
  useMessagesQuery,
} from "../../src/hooks/queries/useMessagesQuery";

const mockInvoke = vi.mocked(invoke);

function makeMessage(id: string, date: number): MessageSummary {
  return {
    id,
    account_id: "account-1",
    remote_id: `remote-${id}`,
    thread_id: null,
    subject: `Subject ${id}`,
    snippet: "",
    from_address: "sender@example.com",
    from_name: "Sender",
    to_list: [],
    cc_list: [],
    bcc_list: [],
    has_attachments: false,
    is_read: true,
    is_starred: false,
    is_draft: false,
    date,
    remote_version: null,
    is_deleted: false,
    deleted_at: null,
    created_at: date,
    updated_at: date,
  } as MessageSummary;
}

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe("useMessagesQuery keyset pagination", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("derives the next cursor from the oldest loaded message, skipping emptied pages", () => {
    expect(nextMessagesCursor([])).toBeNull();
    expect(
      nextMessagesCursor([[makeMessage("a", 30), makeMessage("b", 20)], []]),
    ).toEqual({ date: 20, id: "b" });
  });

  it("requests the next page with the last message's (date, id)", async () => {
    const firstPage = Array.from({ length: MESSAGES_PAGE_SIZE }, (_, i) =>
      makeMessage(`m${i}`, 1_000 - i),
    );
    mockInvoke.mockResolvedValueOnce(firstPage).mockResolvedValueOnce([makeMessage("old", 1)]);

    const { result } = renderHook(() => useMessagesQuery("inbox"), { wrapper });
    await waitFor(() => expect(result.current.data).toHaveLength(MESSAGES_PAGE_SIZE));
    expect(mockInvoke).toHaveBeenLastCalledWith("list_messages", {
      folderId: "inbox",
      folderIds: undefined,
      limit: MESSAGES_PAGE_SIZE,
      offset: 0,
      before: null,
    });

    await act(async () => {
      await result.current.fetchNextPage();
    });
    await waitFor(() => expect(result.current.data).toHaveLength(MESSAGES_PAGE_SIZE + 1));
    expect(mockInvoke).toHaveBeenLastCalledWith("list_messages", {
      folderId: "inbox",
      folderIds: undefined,
      limit: MESSAGES_PAGE_SIZE,
      offset: 0,
      before: { date: 1_000 - (MESSAGES_PAGE_SIZE - 1), id: `m${MESSAGES_PAGE_SIZE - 1}` },
    });
    expect(result.current.hasNextPage).toBe(false);
  });
});
