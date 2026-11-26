import { useInfiniteQuery, type InfiniteData, type QueryClient } from "@tanstack/react-query";
import { listMessages } from "@/lib/api";
import type { MessageCursor, MessageSummary } from "@/lib/api";

export const MESSAGES_PAGE_SIZE = 50;

export const messagesQueryKey = (folderId: string, folderIds?: string[]) =>
  ["messages", folderId, folderIds] as const;

/**
 * Cursor for the page after `pages`: the `(date, id)` of the oldest message
 * loaded so far. Scans back past pages emptied by optimistic removals.
 */
export function nextMessagesCursor(pages: MessageSummary[][]): MessageCursor | null {
  for (let i = pages.length - 1; i >= 0; i -= 1) {
    const page = pages[i];
    const last = page[page.length - 1];
    if (last) return { date: last.date, id: last.id };
  }
  return null;
}

/**
 * Paginated message list backed by React Query's useInfiniteQuery.
 * Pages are fetched with a keyset cursor (the last loaded message's
 * `(date, id)`), so deep pages cost the same as the first and messages
 * arriving at the top never shift later pages. `data` flattens the pages
 * so render code keeps the flat-array shape it already expects.
 */
export function useMessagesQuery(
  folderId: string | null,
  folderIds?: string[],
) {
  const query = useInfiniteQuery({
    queryKey: messagesQueryKey(folderId ?? "", folderIds),
    queryFn: ({ pageParam }) =>
      listMessages(folderId!, MESSAGES_PAGE_SIZE, 0, folderIds, pageParam),
    initialPageParam: null as MessageCursor | null,
    getNextPageParam: (lastPage, allPages) =>
      lastPage.length < MESSAGES_PAGE_SIZE ? undefined : nextMessagesCursor(allPages) ?? undefined,
    enabled: !!folderId,
    staleTime: 60_000,
  });

  const data: MessageSummary[] = query.data?.pages.flat() ?? [];
  return {
    data,
    isLoading: query.isLoading,
    hasNextPage: query.hasNextPage,
    isFetchingNextPage: query.isFetchingNextPage,
    fetchNextPage: query.fetchNextPage,
  };
}

export type MessagesInfiniteData = InfiniteData<MessageSummary[], MessageCursor | null>;

/**
 * Patch every cached ["messages", ...] infinite-query entry with a transform
 * applied to each page's flat array. Used by optimistic flag/delete/move
 * updates so they don't have to know about the page shape.
 */
export function patchMessagesCache(
  queryClient: QueryClient,
  transform: (messages: MessageSummary[]) => MessageSummary[],
) {
  queryClient.setQueriesData<MessagesInfiniteData>(
    { queryKey: ["messages"] },
    (old) => {
      if (!old) return old;
      return { ...old, pages: old.pages.map(transform) };
    },
  );
}

/** Find the first cached message matching `predicate` across all pages. */
export function findCachedMessage(
  queryClient: QueryClient,
  predicate: (m: MessageSummary) => boolean,
): MessageSummary | undefined {
  const entries = queryClient.getQueriesData<MessagesInfiniteData>({ queryKey: ["messages"] });
  for (const [, data] of entries) {
    if (!data) continue;
    for (const page of data.pages) {
      const hit = page.find(predicate);
      if (hit) return hit;
    }
  }
  return undefined;
}

/** Read a flat snapshot of the first non-empty cached message list. */
export function readFirstCachedMessages(queryClient: QueryClient): MessageSummary[] {
  const entries = queryClient.getQueriesData<MessagesInfiniteData>({ queryKey: ["messages"] });
  for (const [, data] of entries) {
    const flat = data?.pages.flat() ?? [];
    if (flat.length > 0) return flat;
  }
  return [];
}

/**
 * Snapshot every ["messages", ...] cache entry so it can be restored on
 * optimistic-update rollback. Returns an array of [queryKey, data] pairs.
 */
export function snapshotMessagesCache(queryClient: QueryClient) {
  return queryClient.getQueriesData<MessagesInfiniteData>({ queryKey: ["messages"] });
}

export function restoreMessagesCache(
  queryClient: QueryClient,
  snapshot: ReturnType<typeof snapshotMessagesCache>,
) {
  for (const [key, data] of snapshot) {
    queryClient.setQueryData(key, data);
  }
}
