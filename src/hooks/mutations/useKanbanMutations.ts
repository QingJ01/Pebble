import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { moveToKanban, removeFromKanban, setKanbanContextNote } from "@/lib/api";
import type { KanbanCard, KanbanColumnType } from "@/lib/api";
import {
  invalidateKanbanQueries,
  kanbanCardsQueryKey,
} from "@/hooks/queries/useKanbanCardsQuery";
import { saveKanbanContextNote } from "@/hooks/queries/useKanbanContextNotesQuery";
import { toastError } from "@/stores/toast.store";

interface KanbanMutationContext {
  previous: KanbanCard[] | undefined;
}

/** Cancel in-flight refetches, snapshot the cache, then apply `update`. */
async function applyOptimistic(
  client: QueryClient,
  update: (cards: KanbanCard[]) => KanbanCard[],
): Promise<KanbanMutationContext> {
  await client.cancelQueries({ queryKey: kanbanCardsQueryKey });
  const previous = client.getQueryData<KanbanCard[]>(kanbanCardsQueryKey);
  if (previous) {
    client.setQueryData<KanbanCard[]>(kanbanCardsQueryKey, update(previous));
  }
  return { previous };
}

function rollback(client: QueryClient, context: KanbanMutationContext | undefined) {
  if (context?.previous) {
    client.setQueryData(kanbanCardsQueryKey, context.previous);
  }
}

export function moveCardInList(
  cards: KanbanCard[],
  messageId: string,
  column: KanbanColumnType,
  position: number,
): KanbanCard[] {
  return cards.map((card) =>
    card.message_id === messageId ? { ...card, column, position } : card,
  );
}

/**
 * Give `column` exactly the order `orderedIds`. A card listed there that
 * currently sits in another column moves into `column` (cross-column drop at
 * a specific position).
 */
export function reorderColumnInList(
  cards: KanbanCard[],
  column: KanbanColumnType,
  orderedIds: string[],
): KanbanCard[] {
  const listed = new Set(orderedIds);
  const others = cards.filter((card) => card.column !== column && !listed.has(card.message_id));
  const reordered = orderedIds
    .map((id, index) => {
      const card = cards.find((candidate) => candidate.message_id === id);
      return card ? { ...card, column, position: index } : null;
    })
    .filter((card): card is KanbanCard => card !== null);
  return [...others, ...reordered];
}

export function useMoveKanbanCardMutation() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ messageId, column, position }: {
      messageId: string;
      column: KanbanColumnType;
      position: number;
    }) => moveToKanban(messageId, column, position),
    onMutate: ({ messageId, column, position }) =>
      applyOptimistic(client, (cards) => moveCardInList(cards, messageId, column, position)),
    onError: (error, _vars, context) => {
      rollback(client, context);
      toastError(error, "kanban.moveFailed");
    },
    onSettled: () => invalidateKanbanQueries(client),
  });
}

export function useReorderKanbanColumnMutation() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async ({ column, orderedIds }: {
      column: KanbanColumnType;
      orderedIds: string[];
    }) => {
      await Promise.all(
        orderedIds.map((messageId, position) => moveToKanban(messageId, column, position)),
      );
    },
    onMutate: ({ column, orderedIds }) =>
      applyOptimistic(client, (cards) => reorderColumnInList(cards, column, orderedIds)),
    // Any failed write rolls the whole column back; the refetch then shows
    // whatever subset the backend actually persisted.
    onError: (error, _vars, context) => {
      rollback(client, context);
      toastError(error, "kanban.moveFailed");
    },
    onSettled: () => invalidateKanbanQueries(client),
  });
}

export function useRemoveKanbanCardMutation() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ messageId }: { messageId: string }) => removeFromKanban(messageId),
    onMutate: ({ messageId }) =>
      applyOptimistic(client, (cards) => cards.filter((card) => card.message_id !== messageId)),
    onError: (error, _vars, context) => {
      rollback(client, context);
      toastError(error, "kanban.removeFailed");
    },
    onSettled: () => invalidateKanbanQueries(client),
  });
}

/**
 * Add (or move) a message onto the board from outside the board view, e.g.
 * toolbars and the command palette. The new card's position comes from the
 * backend, so this refetches instead of guessing.
 */
export async function addKanbanCard(
  messageId: string,
  column: KanbanColumnType,
  client: QueryClient,
): Promise<void> {
  await moveToKanban(messageId, column);
  await invalidateKanbanQueries(client);
}

/** Persist a kanban context note (optimistic; see `saveKanbanContextNote`). */
export function setKanbanContextNoteInCache(client: QueryClient, messageId: string, note: string) {
  return saveKanbanContextNote(client, messageId, note, setKanbanContextNote);
}

export function useSetKanbanContextNoteMutation() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ messageId, note }: { messageId: string; note: string }) =>
      setKanbanContextNoteInCache(client, messageId, note),
  });
}
