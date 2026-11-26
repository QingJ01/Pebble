import { useMemo } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import { getFolderUnreadCounts } from "@/lib/api";
import { useUIStore } from "@/stores/ui.store";

export function useFolderUnreadCounts(accountId: string | null) {
  const enabled = useUIStore((s) => s.showFolderUnreadCount);
  return useQuery({
    queryKey: ["folder-unread-counts", accountId],
    queryFn: () => getFolderUnreadCounts(accountId!),
    enabled: enabled && !!accountId,
    staleTime: 30_000,
    refetchInterval: 30_000,
  });
}

/**
 * Unread counts by folder id for several accounts. Follows the "show folder
 * unread counts" setting unless `always` (the unified inbox account filter,
 * whose chips always carry their count).
 */
export function useFolderUnreadCountsForAccounts(accountIds: string[], options: { always?: boolean } = {}) {
  const showCounts = useUIStore((s) => s.showFolderUnreadCount);
  const enabled = showCounts || options.always === true;
  const queries = useQueries({
    queries: accountIds.map((accountId) => ({
      queryKey: ["folder-unread-counts", accountId],
      queryFn: () => getFolderUnreadCounts(accountId),
      enabled: enabled && !!accountId,
      staleTime: 30_000,
      refetchInterval: 30_000,
    })),
  });

  const data = useMemo(
    () => Object.assign({}, ...queries.map((query) => query.data ?? {})),
    [queries],
  );

  return {
    data,
    isLoading: queries.some((query) => query.isLoading),
  };
}
