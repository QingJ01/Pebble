import { useMutation, useQueryClient } from "@tanstack/react-query";
import { updateMessageFlags } from "@/lib/api";
import type { Message } from "@/lib/api";
import {
  patchMessagesCache,
  snapshotMessagesCache,
  restoreMessagesCache,
} from "@/hooks/queries";

interface UpdateFlagsParams {
  messageId: string;
  isRead?: boolean;
  isStarred?: boolean;
}

interface MutationContext {
  previousMessage: Message | null | undefined;
  previousLists: ReturnType<typeof snapshotMessagesCache>;
}

export function useUpdateFlagsMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (params: UpdateFlagsParams) =>
      updateMessageFlags(params.messageId, params.isRead, params.isStarred),
    onMutate: async (params): Promise<MutationContext> => {
      await queryClient.cancelQueries({ queryKey: ["messages"] });
      await queryClient.cancelQueries({
        queryKey: ["message", params.messageId],
      });

      const previousMessage = queryClient.getQueryData<Message | null>([
        "message",
        params.messageId,
      ]);

      const previousLists = snapshotMessagesCache(queryClient);

      if (previousMessage) {
        queryClient.setQueryData<Message | null>(
          ["message", params.messageId],
          {
            ...previousMessage,
            ...(params.isRead !== undefined && { is_read: params.isRead }),
            ...(params.isStarred !== undefined && {
              is_starred: params.isStarred,
            }),
          },
        );
      }

      patchMessagesCache(queryClient, (page) =>
        page.map((m) =>
          m.id === params.messageId
            ? {
                ...m,
                ...(params.isRead !== undefined && { is_read: params.isRead }),
                ...(params.isStarred !== undefined && { is_starred: params.isStarred }),
              }
            : m,
        ),
      );

      return { previousMessage, previousLists };
    },
    onError: (_err, params, context) => {
      if (context?.previousMessage) {
        queryClient.setQueryData(
          ["message", params.messageId],
          context.previousMessage,
        );
      }
      if (context?.previousLists) {
        restoreMessagesCache(queryClient, context.previousLists);
      }
    },
    onSettled: (_data, _err, params) => {
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["threads"] });
      queryClient.invalidateQueries({ queryKey: ["starred-messages"] });
      queryClient.invalidateQueries({ queryKey: ["message", params.messageId] });
    },
  });
}
