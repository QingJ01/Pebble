import { useDroppable } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { useTranslation } from "react-i18next";
import type { KanbanColumnType, Message } from "@/lib/api";
import KanbanCardComponent from "./KanbanCard";

interface Props {
  id: KanbanColumnType;
  title: string;
  cardIds: string[];
  contextNotes: Record<string, string>;
  messages: Map<string, Message>;
  onRemove: (id: string) => void;
  onOpen: (messageId: string) => void;
}

export default function KanbanColumn({ id, title, cardIds, contextNotes, messages, onRemove, onOpen }: Props) {
  const { t } = useTranslation();
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
          fontWeight: "var(--font-weight-title)" as React.CSSProperties["fontWeight"],
          fontSize: "var(--font-md)",
          userSelect: "none",
          WebkitUserSelect: "none",
          color: "var(--color-text-primary)",
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
        }}
      >
        <span>{title}</span>
        <span
          style={{
            fontSize: "var(--font-sm)",
            color: "var(--color-text-secondary)",
            backgroundColor: "var(--color-bg-secondary)",
            padding: "2px 8px",
            borderRadius: "var(--radius-full)",
          }}
        >
          {cardIds.length}
        </span>
      </div>
      <div
        ref={setNodeRef}
        className="scroll-region kanban-column-scroll"
        style={{
          flex: 1,
          padding: "0 8px 8px",
          overflowY: "auto",
          backgroundColor: isOver ? "var(--color-bg-hover)" : "transparent",
          borderRadius: "var(--radius-lg)",
          transition: "background-color var(--dur-fast) var(--ease-out)",
          minHeight: "100px",
        }}
      >
        <SortableContext items={cardIds} strategy={verticalListSortingStrategy}>
          {cardIds.map((cardId) => (
            <KanbanCardComponent
              key={cardId}
              id={cardId}
              note={contextNotes[cardId]}
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
              fontSize: "var(--font-base)",
              padding: "24px 0",
              userSelect: "none",
            }}
          >
            {t("kanban.dropHere")}
          </div>
        )}
      </div>
    </div>
  );
}
