import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { listKanbanCards, moveToKanban, removeFromKanban, type KanbanCard } from "@/lib/api";
import {
  invalidateKanbanQueries,
  isMessageInKanban,
  kanbanCardsQueryKey,
  useIsInKanban,
  useKanbanCardsQuery,
} from "@/hooks/queries/useKanbanCardsQuery";
import {
  addKanbanCard,
  reorderColumnInList,
  useMoveKanbanCardMutation,
  useRemoveKanbanCardMutation,
  useReorderKanbanColumnMutation,
} from "@/hooks/mutations/useKanbanMutations";

vi.mock("@/lib/api", () => ({
  listKanbanCards: vi.fn(),
  moveToKanban: vi.fn(() => Promise.resolve()),
  removeFromKanban: vi.fn(() => Promise.resolve()),
}));

const mockList = vi.mocked(listKanbanCards);
const mockMove = vi.mocked(moveToKanban);
const mockRemove = vi.mocked(removeFromKanban);

function card(id: string, column: KanbanCard["column"], position: number): KanbanCard {
  return { message_id: id, column, position, created_at: 1, updated_at: 1 };
}

function setup(initial: KanbanCard[]) {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false, staleTime: Infinity } },
  });
  queryClient.setQueryData(kanbanCardsQueryKey, initial);
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  const cached = () => queryClient.getQueryData<KanbanCard[]>(kanbanCardsQueryKey);
  return { queryClient, wrapper, cached };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("kanban react-query data", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("loads cards through the shared query and answers membership", async () => {
    mockList.mockResolvedValue([card("m1", "todo", 0)]);
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(
      () => ({ query: useKanbanCardsQuery(), inKanban: useIsInKanban("m1"), other: useIsInKanban("m2") }),
      { wrapper },
    );

    await waitFor(() => expect(result.current.query.isSuccess).toBe(true));
    expect(result.current.inKanban).toBe(true);
    expect(result.current.other).toBe(false);
    expect(isMessageInKanban("m1", queryClient)).toBe(true);
    expect(mockList).toHaveBeenCalledTimes(1);
  });

  it("moves a card optimistically and keeps it after success", async () => {
    const { wrapper, cached } = setup([card("m1", "todo", 0)]);
    const pending = deferred<void>();
    mockMove.mockReturnValueOnce(pending.promise);
    mockList.mockResolvedValue([card("m1", "done", 0)]);

    const { result } = renderHook(() => useMoveKanbanCardMutation(), { wrapper });
    act(() => {
      result.current.mutate({ messageId: "m1", column: "done", position: 0 });
    });

    await waitFor(() => expect(cached()?.[0].column).toBe("done"));
    expect(mockMove).toHaveBeenCalledWith("m1", "done", 0);
    pending.resolve();
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(cached()?.[0].column).toBe("done");
  });

  it("rolls a failed move back to the previous cards", async () => {
    const { wrapper, cached } = setup([card("m1", "todo", 0)]);
    mockMove.mockRejectedValueOnce(new Error("fail"));
    mockList.mockResolvedValue([card("m1", "todo", 0)]);

    const { result } = renderHook(() => useMoveKanbanCardMutation(), { wrapper });
    act(() => {
      result.current.mutate({ messageId: "m1", column: "done", position: 0 });
    });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(cached()?.[0].column).toBe("todo");
  });

  it("removes optimistically and restores the card on failure", async () => {
    const { wrapper, cached } = setup([card("m1", "todo", 0), card("m2", "todo", 1)]);
    const failing = deferred<void>();
    mockRemove.mockReturnValueOnce(failing.promise);
    mockList.mockResolvedValue([card("m1", "todo", 0), card("m2", "todo", 1)]);

    const { result } = renderHook(() => useRemoveKanbanCardMutation(), { wrapper });
    act(() => {
      result.current.mutate({ messageId: "m1" });
    });
    await waitFor(() => expect(cached()?.map((c) => c.message_id)).toEqual(["m2"]));

    failing.reject(new Error("fail"));
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(cached()?.map((c) => c.message_id)).toEqual(["m1", "m2"]);
  });

  it("reorders a column and rolls the whole column back if any write fails", async () => {
    const initial = [card("a", "todo", 0), card("b", "todo", 1), card("x", "done", 0)];
    expect(reorderColumnInList(initial, "todo", ["b", "a"]).filter((c) => c.column === "todo"))
      .toEqual([card("b", "todo", 0), card("a", "todo", 1)]);

    const { wrapper, cached } = setup(initial);
    mockMove.mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("fail"));
    mockList.mockResolvedValue(initial);

    const { result } = renderHook(() => useReorderKanbanColumnMutation(), { wrapper });
    act(() => {
      result.current.mutate({ column: "todo", orderedIds: ["b", "a"] });
    });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(cached()).toEqual(initial);
  });

  it("refetches after adding a card and after message deletes invalidate the board", async () => {
    const { queryClient } = setup([]);
    const cachedQuery = queryClient.getQueryCache().find({ queryKey: kanbanCardsQueryKey });
    expect(cachedQuery).toBeDefined();
    mockList.mockResolvedValue([card("m1", "todo", 0)]);
    // Mount an observer so invalidation triggers a refetch.
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
    renderHook(() => useKanbanCardsQuery(), { wrapper });

    await addKanbanCard("m1", "todo", queryClient);
    expect(mockMove).toHaveBeenCalledWith("m1", "todo");
    await waitFor(() => expect(isMessageInKanban("m1", queryClient)).toBe(true));

    // The message is deleted elsewhere; the backend no longer lists its card.
    mockList.mockResolvedValue([]);
    await invalidateKanbanQueries(queryClient);
    await waitFor(() => expect(isMessageInKanban("m1", queryClient)).toBe(false));
  });
});
