import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMailNewCoalescer } from "../../src/components/StatusBar";

function mailNew(accountId: string, folderIds: string[] = ["inbox-1"]) {
  return { account_id: accountId, message_id: `m-${Math.random()}`, folder_ids: folderIds };
}

function pages(count: number) {
  return {
    pages: Array.from({ length: count }, (_, index) => [{ id: `page-${index}` }]),
    pageParams: Array.from({ length: count }, (_, index) => index * 50),
  };
}

describe("createMailNewCoalescer", () => {
  let queryClient: QueryClient;
  let invalidate: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.useFakeTimers();
    queryClient = new QueryClient();
    invalidate = vi.spyOn(queryClient, "invalidateQueries");
  });

  afterEach(() => {
    queryClient.clear();
    vi.useRealTimers();
  });

  function invalidatedKeys() {
    return invalidate.mock.calls.map(([filters]) => (filters as { queryKey: unknown[] }).queryKey);
  }

  it("coalesces a burst of events into one trailing invalidation per account", () => {
    const coalescer = createMailNewCoalescer(queryClient);
    for (let i = 0; i < 200; i += 1) {
      coalescer.push(mailNew(i % 2 ? "account-1" : "account-2"));
    }

    vi.advanceTimersByTime(499);
    expect(invalidate).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);

    const keys = invalidatedKeys();
    expect(keys.filter((key) => key[0] === "messages")).toHaveLength(1);
    expect(keys.filter((key) => key[0] === "threads")).toHaveLength(1);
    expect(keys).toContainEqual(["folders", "account-1"]);
    expect(keys).toContainEqual(["folders", "account-2"]);
    expect(keys).toContainEqual(["folder-unread-counts", "account-1"]);
    expect(keys).toContainEqual(["folder-unread-counts", "account-2"]);
    expect(keys).not.toContainEqual(["folders"]);
  });

  it("flushes at the max wait during a continuous stream of events", () => {
    const coalescer = createMailNewCoalescer(queryClient);
    for (let elapsed = 0; elapsed < 2_000; elapsed += 100) {
      coalescer.push(mailNew("account-1"));
      vi.advanceTimersByTime(100);
    }

    expect(invalidatedKeys()).toContainEqual(["folders", "account-1"]);
  });

  it("only invalidates mail lists for folders that received mail", () => {
    queryClient.setQueryData(["messages", "inbox-1", undefined], pages(1));
    queryClient.setQueryData(["messages", "archive-1", undefined], pages(1));
    queryClient.setQueryData(["messages", "unified-inbox", ["inbox-1", "inbox-2"]], pages(1));
    queryClient.setQueryData(["threads", "archive-1", undefined, 50, 0], []);

    const coalescer = createMailNewCoalescer(queryClient);
    coalescer.push(mailNew("account-1", ["inbox-1"]));
    vi.advanceTimersByTime(500);

    const cache = queryClient.getQueryCache();
    expect(cache.find({ queryKey: ["messages", "inbox-1", undefined] })?.state.isInvalidated).toBe(true);
    expect(cache.find({ queryKey: ["messages", "unified-inbox", ["inbox-1", "inbox-2"]] })?.state.isInvalidated).toBe(true);
    expect(cache.find({ queryKey: ["messages", "archive-1", undefined] })?.state.isInvalidated).toBe(false);
    expect(cache.find({ queryKey: ["threads", "archive-1", undefined, 50, 0] })?.state.isInvalidated).toBe(false);
  });

  it("falls back to refreshing every mail list when an event has no folder ids", () => {
    queryClient.setQueryData(["messages", "archive-1", undefined], pages(1));

    const coalescer = createMailNewCoalescer(queryClient);
    coalescer.push({ account_id: "account-1" });
    vi.advanceTimersByTime(500);

    expect(queryClient.getQueryCache().find({ queryKey: ["messages", "archive-1", undefined] })?.state.isInvalidated)
      .toBe(true);
  });

  it("trims inactive infinite lists to their first page so they do not refetch every page", () => {
    queryClient.setQueryData(["messages", "inbox-1", undefined], pages(6));

    const coalescer = createMailNewCoalescer(queryClient);
    coalescer.push(mailNew("account-1", ["inbox-1"]));
    vi.advanceTimersByTime(500);

    const query = queryClient.getQueryCache().find({ queryKey: ["messages", "inbox-1", undefined] });
    const data = query?.state.data as ReturnType<typeof pages>;
    expect(data.pages).toHaveLength(1);
    expect(data.pageParams).toEqual([0]);
    expect(query?.state.isInvalidated).toBe(true);
  });
});
