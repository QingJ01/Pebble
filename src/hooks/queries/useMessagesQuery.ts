import { useInfiniteQuery, type InfiniteData, type QueryClient } from "@tanstack/react-query";
import { listMessages } from "@/lib/api";
import type { MessageCursor, MessageSummary } from "@/lib/api";

export const MESSAGES_PAGE_SIZE = 50;

export const messagesQueryKey = (folderId: string, folderIds?: string[], unreadOnly = false) =>
  ["messages", folderId, folderIds, unreadOnly ? "unread" : "all"] as const;

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
  unreadOnly = false,
) {
  const query = useInfiniteQuery({
    queryKey: messagesQueryKey(folderId ?? "", folderIds, unreadOnly),
    queryFn: ({ pageParam }) =>
      listMessages(folderId!, MESSAGES_PAGE_SIZE, 0, folderIds, pageParam, unreadOnly),
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
    /** Changes whenever the list data changes (for effects keyed on it). */
    dataUpdatedAt: query.dataUpdatedAt,
  };
}

export type MessagesInfiniteData = InfiniteData<MessageSummary[], MessageCursor | null>;

/**
 * Patch every cached ["messages", ...] infinite-query entry with a transform
 * applied to each page's flat array. Used by optimistic flag/delete/move
 * updates so they don't have to know about the page shape.
 */
type MessagesPatch = (messages: MessageSummary[]) => MessageSummary[];
const patchListeners = new Set<(transform: MessagesPatch) => void>();

/**
 * Hear every optimistic patch applied to the message lists (flags, stars,
 * removals), e.g. to keep a row shown outside the cache in step.
 */
export function onMessagesCachePatched(listener: (transform: MessagesPatch) => void): () => void {
  patchListeners.add(listener);
  return () => patchListeners.delete(listener);
}

export function patchMessagesCache(
  queryClient: QueryClient,
  transform: MessagesPatch,
) {
  for (const listener of patchListeners) listener(transform);
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

/**
 * Roll back an optimistic removal for `ids` only: put those messages back into
 * each cached list (from `snapshot`) without touching anything else, so a
 * concurrent action that already removed other messages isn't undone. Each
 * message goes back right after the neighbour it followed in the snapshot.
 */
export function restoreMessagesInCache(
  queryClient: QueryClient,
  snapshot: ReturnType<typeof snapshotMessagesCache>,
  ids: Iterable<string>,
) {
  const idSet = new Set(ids);
  for (const [key, before] of snapshot) {
    if (!before) continue;
    const beforeFlat = before.pages.flat();
    if (!beforeFlat.some((m) => idSet.has(m.id))) continue;
    queryClient.setQueryData<MessagesInfiniteData>(key, (current) => {
      if (!current) return current;
      const merged = current.pages.flat();
      const present = new Set(merged.map((m) => m.id));
      let changed = false;
      beforeFlat.forEach((message, index) => {
        if (!idSet.has(message.id) || present.has(message.id)) return;
        // Insert after the closest earlier snapshot neighbour still listed.
        let at = 0;
        for (let i = index - 1; i >= 0; i -= 1) {
          const pos = merged.findIndex((m) => m.id === beforeFlat[i].id);
          if (pos !== -1) { at = pos + 1; break; }
        }
        merged.splice(at, 0, message);
        present.add(message.id);
        changed = true;
      });
      if (!changed) return current;
      // Keep the page structure: same page sizes, the last page takes the rest.
      const pages: MessageSummary[][] = [];
      let offset = 0;
      current.pages.forEach((page, index) => {
        const size = index === current.pages.length - 1 ? merged.length - offset : page.length;
        pages.push(merged.slice(offset, offset + size));
        offset += size;
      });
      return { ...current, pages };
    });
  }
}
