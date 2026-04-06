import { useQuery } from "@tanstack/react-query";
import { listThreads } from "@/lib/api";

export const threadsQueryKey = (
  folderId: string,
  limit: number,
  offset: number,
) => ["threads", folderId, limit, offset] as const;

export function useThreadsQuery(
  folderId: string | null,
  limit = 50,
  offset = 0,
) {
  return useQuery({
    queryKey: threadsQueryKey(folderId ?? "", limit, offset),
    queryFn: () => listThreads(folderId!, limit, offset),
    enabled: !!folderId,
    staleTime: 60_000,
  });
}
