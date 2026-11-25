import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listThreadMessages } from "@/lib/api";

/**
 * Key readers use to look up the currently open thread's messages by thread
 * id (compose/send-controls). The query itself is additionally keyed by
 * account so two accounts' copies of a shared thread id never share a cache
 * entry; the hook mirrors its result here for those readers.
 */
export const threadMessagesQueryKey = (threadId: string) =>
  ["threadMessages", threadId] as const;

export const accountThreadMessagesQueryKey = (threadId: string, accountId: string | null) =>
  ["threadMessages", threadId, accountId ?? "*"] as const;

export function useThreadMessagesQuery(threadId: string | null, accountId: string | null) {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: accountThreadMessagesQueryKey(threadId ?? "", accountId),
    queryFn: () => listThreadMessages(accountId, threadId!),
    enabled: !!threadId,
  });

  useEffect(() => {
    if (threadId && query.data) {
      queryClient.setQueryData(threadMessagesQueryKey(threadId), query.data);
    }
  }, [queryClient, threadId, query.data]);

  return query;
}
