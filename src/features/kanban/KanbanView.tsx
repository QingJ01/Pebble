import { useEffect, useState } from "react";
import { DndContext, DragEndEvent, PointerSensor, useSensor, useSensors } from "@dnd-kit/core";
import type { KanbanColumnType, Message } from "@/lib/api";
import { getMessage } from "@/lib/api";
import { useKanbanStore } from "@/stores/kanban.store";
import { useUIStore } from "@/stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import KanbanColumn from "./KanbanColumn";
import { KanbanSkeleton } from "@/components/Skeleton";

const COLUMNS: { id: KanbanColumnType; title: string }[] = [
  { id: "todo", title: "To Do" },
  { id: "waiting", title: "Waiting" },
  { id: "done", title: "Done" },
];

export default function KanbanView() {
  const { cards, loading, fetchCards, moveCard, removeCard } = useKanbanStore();
  const [messages, setMessages] = useState<Map<string, Message>>(new Map());

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  );

  useEffect(() => {
    fetchCards();
  }, [fetchCards]);

  // Load message details for all cards
  useEffect(() => {
    async function loadMessages() {
      setMessages((prev) => {
        const toLoad = cards.filter((c) => !prev.has(c.message_id));
        if (toLoad.length === 0) return prev;
        // Load in background, then update
        Promise.all(
          toLoad.map((card) => getMessage(card.message_id).then((msg) => [card.message_id, msg] as const)),
        ).then((results) => {
          setMessages((current) => {
            const next = new Map(current);
            for (const [id, msg] of results) {
              if (msg) next.set(id, msg);
            }
            return next;
          });
        });
        return prev;
      });
    }
    loadMessages();
  }, [cards]);

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
    if (COLUMNS.some((c) => c.id === overId)) {
      targetColumn = overId as KanbanColumnType;
    } else {
      // Dropped on another card — find that card's column
      const overCard = cards.find((c) => c.message_id === overId);
      if (!overCard) return;
      targetColumn = overCard.column;
    }

    const activeCard = cards.find((c) => c.message_id === activeId);
    if (!activeCard || activeCard.column === targetColumn) return;

    const targetCards = cards.filter((c) => c.column === targetColumn);
    moveCard(activeId, targetColumn, targetCards.length);
  }

  if (loading && cards.length === 0) {
    return <KanbanSkeleton />;
  }

  return (
    <div style={{ display: "flex", gap: "8px", padding: "16px", height: "100%", overflow: "hidden" }}>
      <DndContext sensors={sensors} onDragEnd={handleDragEnd}>
        {COLUMNS.map((col) => (
          <KanbanColumn
            key={col.id}
            id={col.id}
            title={col.title}
            cardIds={cards.filter((c) => c.column === col.id).map((c) => c.message_id)}
            messages={messages}
            onRemove={removeCard}
            onOpen={handleOpenMessage}
          />
        ))}
      </DndContext>
    </div>
  );
}
