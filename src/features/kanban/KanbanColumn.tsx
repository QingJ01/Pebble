import { useDroppable } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { useTranslation } from "react-i18next";
import type { KanbanColumnType, Message } from "@/lib/api";
import KanbanCardComponent from "./KanbanCard";
import "./kanban.css";

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
    <div className="kanban-column">
      <div className="kanban-column__header">
        <span>{title}</span>
        <span className="kanban-column__count">{cardIds.length}</span>
      </div>
      <div
        ref={setNodeRef}
        className={`scroll-region kanban-column-scroll kanban-column__cards${isOver ? " is-over" : ""}`}
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
          <div className="kanban-column__empty">{t("kanban.dropHere")}</div>
        )}
      </div>
    </div>
  );
}
