import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { DndContext, DragEndEvent, KeyboardSensor, PointerSensor, useSensor, useSensors } from "@dnd-kit/core";
import { arrayMove, sortableKeyboardCoordinates } from "@dnd-kit/sortable";
import type { KanbanCard, KanbanColumnType, Message } from "@/lib/api";
import { getMessagesBatch } from "@/lib/api";
import { useKanbanContextNotesQuery, type KanbanContextNotes } from "@/hooks/queries/useKanbanContextNotesQuery";
import { useKanbanCardsQuery } from "@/hooks/queries/useKanbanCardsQuery";
import {
  addKanbanCard,
  useMoveKanbanCardMutation,
  useRemoveKanbanCardMutation,
  useReorderKanbanColumnMutation,
} from "@/hooks/mutations/useKanbanMutations";
import { useToastStore } from "@/stores/toast.store";
import { useUIStore } from "@/stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import KanbanColumn from "./KanbanColumn";
import { KanbanSkeleton } from "@/components/Skeleton";

const COLUMN_IDS: { id: KanbanColumnType; titleKey: string }[] = [
  { id: "todo", titleKey: "kanban.todo" },
  { id: "waiting", titleKey: "kanban.waiting" },
  { id: "done", titleKey: "kanban.done" },
];

const NO_CARDS: KanbanCard[] = [];
const NO_NOTES: KanbanContextNotes = {};

export default function KanbanView() {
  const { t } = useTranslation();
  // Always refetch when the board opens: cards can go stale while messages
  // are deleted or moved elsewhere in the app.
  const { data: cards = NO_CARDS, isPending } = useKanbanCardsQuery({ refetchOnMount: "always" });
  // Notes are optional: a failed load leaves the board usable without them.
  const { data: contextNotes = NO_NOTES } = useKanbanContextNotesQuery({ refetchOnMount: "always" });
  const queryClient = useQueryClient();
  const moveCard = useMoveKanbanCardMutation();
  const reorderColumn = useReorderKanbanColumnMutation();
  const removeCard = useRemoveKanbanCardMutation();
  const [messages, setMessages] = useState<Map<string, Message>>(new Map());

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  // Load message details for all cards (batch)
  useEffect(() => {
    const toLoad = cards.filter((c) => !messages.has(c.message_id));
    if (toLoad.length === 0) return;
    let cancelled = false;
    getMessagesBatch(toLoad.map((c) => c.message_id)).then((msgs) => {
      if (cancelled) return;
      setMessages((current) => {
        const next = new Map(current);
        for (const msg of msgs) {
          next.set(msg.id, msg);
        }
        return next;
      });
    });
    return () => { cancelled = true; };
  }, [cards]); // eslint-disable-line react-hooks/exhaustive-deps

  function handleOpenMessage(messageId: string) {
    useMailStore.getState().setSelectedMessage(messageId);
    useUIStore.getState().setActiveView("inbox");
  }

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    if (!over) return;

    const activeId = active.id as string;
    const overId = over.id as string;

    // Determine target column
    let targetColumn: KanbanColumnType;
    if (COLUMN_IDS.some((c) => c.id === overId)) {
      targetColumn = overId as KanbanColumnType;
    } else {
      const overCard = cards.find((c) => c.message_id === overId);
      if (!overCard) return;
      targetColumn = overCard.column;
    }

    const activeCard = cards.find((c) => c.message_id === activeId);
    if (!activeCard) return;

    if (activeCard.column === targetColumn) {
      // Same-column reorder
      const columnIds = cards
        .filter((c) => c.column === targetColumn)
        .sort((a, b) => a.position - b.position)
        .map((c) => c.message_id);
      const oldIndex = columnIds.indexOf(activeId);
      const newIndex = columnIds.indexOf(overId);
      if (oldIndex !== -1 && newIndex !== -1 && oldIndex !== newIndex) {
        const reordered = arrayMove(columnIds, oldIndex, newIndex);
        reorderColumn.mutate({ column: targetColumn, orderedIds: reordered });
      }
    } else {
      // Cross-column move
      const targetCards = cards.filter((c) => c.column === targetColumn);
      moveCard.mutate({ messageId: activeId, column: targetColumn, position: targetCards.length });
    }
  }

  async function handleRemove(messageId: string) {
    const card = cards.find((c) => c.message_id === messageId);
    const oldColumn = card?.column ?? "todo";
    try {
      await removeCard.mutateAsync({ messageId });
    } catch {
      return; // rolled back; the card is still on the board
    }
    useToastStore.getState().addToast({
      message: t("kanban.removedFromBoard", "Card removed from board"),
      type: "info",
      action: {
        label: t("kanban.undoRemove", "Undo"),
        onClick: () => {
          addKanbanCard(messageId, oldColumn, queryClient).catch(() => {});
        },
      },
    });
  }

  if (isPending) {
    return <KanbanSkeleton />;
  }

  return (
    <div style={{ display: "flex", gap: "8px", padding: "16px", height: "100%", overflow: "hidden" }}>
      <DndContext sensors={sensors} onDragEnd={handleDragEnd}>
        {COLUMN_IDS.map((col) => (
          <KanbanColumn
            key={col.id}
            id={col.id}
            title={t(col.titleKey)}
            cardIds={cards.filter((c) => c.column === col.id).map((c) => c.message_id)}
            contextNotes={contextNotes}
            messages={messages}
            onRemove={handleRemove}
            onOpen={handleOpenMessage}
          />
        ))}
      </DndContext>
    </div>
  );
}
