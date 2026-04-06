import { useQuery } from "@tanstack/react-query";
import { listMessages } from "@/lib/api";

export const messagesQueryKey = (
  folderId: string,
  limit: number,
  offset: number,
  folderIds?: string[],
) => ["messages", folderId, limit, offset, folderIds] as const;

export function useMessagesQuery(
  folderId: string | null,
  limit = 50,
  offset = 0,
  folderIds?: string[],
) {
  return useQuery({
    queryKey: messagesQueryKey(folderId ?? "", limit, offset, folderIds),
    queryFn: () => listMessages(folderId!, limit, offset, folderIds),
    enabled: !!folderId,
    staleTime: 60_000, // Messages rarely change mid-session; avoid unnecessary refetches
  });
}
