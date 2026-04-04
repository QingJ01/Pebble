import { useDroppable } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import type { KanbanColumnType, Message } from "@/lib/api";
import KanbanCardComponent from "./KanbanCard";

interface Props {
  id: KanbanColumnType;
  title: string;
  cardIds: string[];
  messages: Map<string, Message>;
  onRemove: (id: string) => void;
  onOpen: (messageId: string) => void;
}

export default function KanbanColumn({ id, title, cardIds, messages, onRemove, onOpen }: Props) {
  const { setNodeRef, isOver } = useDroppable({ id });

  return (
    <div
      style={{
        flex: 1,
        minWidth: "240px",
        maxWidth: "380px",
        display: "flex",
        flexDirection: "column",
        height: "100%",
      }}
    >
      <div
        style={{
          padding: "12px 16px",
          fontWeight: 600,
          fontSize: "14px",
          color: "var(--color-text-primary)",
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
        }}
      >
        <span>{title}</span>
        <span
          style={{
            fontSize: "12px",
            color: "var(--color-text-secondary)",
            backgroundColor: "var(--color-bg-secondary, rgba(0,0,0,0.06))",
            padding: "2px 8px",
            borderRadius: "10px",
          }}
        >
          {cardIds.length}
        </span>
      </div>
      <div
        ref={setNodeRef}
        style={{
          flex: 1,
          padding: "0 8px 8px",
          overflowY: "auto",
          backgroundColor: isOver ? "var(--color-bg-hover, rgba(0,0,0,0.03))" : "transparent",
          borderRadius: "8px",
          transition: "background-color 0.15s ease",
          minHeight: "100px",
        }}
      >
        <SortableContext items={cardIds} strategy={verticalListSortingStrategy}>
          {cardIds.map((cardId) => (
            <KanbanCardComponent
              key={cardId}
              id={cardId}
              message={messages.get(cardId) || null}
              onRemove={onRemove}
              onOpen={onOpen}
            />
          ))}
        </SortableContext>
        {cardIds.length === 0 && (
          <div
            style={{
              textAlign: "center",
              color: "var(--color-text-secondary)",
              fontSize: "13px",
              padding: "24px 0",
            }}
          >
            Drop messages here
          </div>
        )}
      </div>
    </div>
  );
}
