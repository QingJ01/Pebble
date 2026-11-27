import { useQuery } from "@tanstack/react-query";
import { listThreads } from "@/lib/api";

export const threadsQueryKey = (
  folderId: string,
  limit: number,
  offset: number,
  folderIds?: string[],
  unreadOnly = false,
) => ["threads", folderId, folderIds, limit, offset, unreadOnly ? "unread" : "all"] as const;

export function useThreadsQuery(
  folderId: string | null,
  limit = 50,
  offset = 0,
  folderIds?: string[],
  unreadOnly = false,
) {
  return useQuery({
    queryKey: threadsQueryKey(folderId ?? "", limit, offset, folderIds, unreadOnly),
    queryFn: () => listThreads(folderId!, limit, offset, folderIds, unreadOnly),
    enabled: !!folderId,
    staleTime: 60_000,
  });
}
