import { useQuery, type QueryClient } from "@tanstack/react-query";
import { listKanbanCards } from "@/lib/api";
import type { KanbanCard } from "@/lib/api";

/** Root key for all kanban server data. Invalidate this after message deletes/moves. */
export const kanbanQueryRoot = ["kanban"] as const;
export const kanbanCardsQueryKey = ["kanban", "cards"] as const;

const EMPTY_CARDS: KanbanCard[] = [];
const cardIdSetCache = new WeakMap<KanbanCard[], Set<string>>();

/** O(1) membership lookups shared by every list row rendering the same data. */
export function kanbanCardIdSet(cards: KanbanCard[]): Set<string> {
  let ids = cardIdSetCache.get(cards);
  if (!ids) {
    ids = new Set(cards.map((card) => card.message_id));
    cardIdSetCache.set(cards, ids);
  }
  return ids;
}

export function useKanbanCardsQuery(options: { refetchOnMount?: boolean | "always" } = {}) {
  return useQuery({
    queryKey: kanbanCardsQueryKey,
    queryFn: () => listKanbanCards(),
    refetchOnMount: options.refetchOnMount,
  });
}

/** Whether a message is on the board, from the shared kanban query cache. */
export function useIsInKanban(messageId: string): boolean {
  const { data } = useQuery({
    queryKey: kanbanCardsQueryKey,
    queryFn: () => listKanbanCards(),
    select: (cards: KanbanCard[]) => kanbanCardIdSet(cards).has(messageId),
  });
  return data ?? false;
}

export function getCachedKanbanCards(client: QueryClient): KanbanCard[] {
  return client.getQueryData<KanbanCard[]>(kanbanCardsQueryKey) ?? EMPTY_CARDS;
}

export function isMessageInKanban(messageId: string, client: QueryClient): boolean {
  return kanbanCardIdSet(getCachedKanbanCards(client)).has(messageId);
}

export function invalidateKanbanQueries(client: QueryClient) {
  return client.invalidateQueries({ queryKey: kanbanQueryRoot });
}
