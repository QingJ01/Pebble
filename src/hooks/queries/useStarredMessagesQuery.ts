import { useInfiniteQuery } from "@tanstack/react-query";
import { listStarredMessages } from "@/lib/api";
import type { MessageSummary } from "@/lib/api";

export const STARRED_MESSAGES_PAGE_SIZE = 50;

export const starredMessagesQueryKey = (accountId: string) =>
  ["starred-messages", accountId] as const;

export function useStarredMessagesQuery(accountId: string | null, offsetAdjustment = 0) {
  const query = useInfiniteQuery({
    queryKey: starredMessagesQueryKey(accountId ?? ""),
    queryFn: ({ pageParam }) =>
      listStarredMessages(accountId!, STARRED_MESSAGES_PAGE_SIZE, pageParam as number),
    initialPageParam: 0,
    getNextPageParam: (lastPage, allPages) => {
      if (lastPage.length < STARRED_MESSAGES_PAGE_SIZE) return undefined;
      const loadedCount = allPages.reduce((total, page) => total + page.length, 0);
      return Math.max(0, loadedCount - offsetAdjustment);
    },
    enabled: !!accountId,
    staleTime: 60_000,
  });

  return {
    data: query.data?.pages.flat() ?? [],
    loading: query.isLoading,
    error: query.error,
    hasNextPage: query.hasNextPage,
    isFetchingNextPage: query.isFetchingNextPage,
    fetchNextPage: query.fetchNextPage,
    refetch: query.refetch,
  };
}

export type StarredMessagesData = MessageSummary[];
