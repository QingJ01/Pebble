import { useDroppable } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { useTranslation } from "react-i18next";
import type { KanbanColumnType, Message } from "@/lib/api";
import type { BadgeAccount } from "@/hooks/useAccountBadges";
import KanbanCardComponent from "./KanbanCard";
import "./kanban.css";

interface Props {
  id: KanbanColumnType;
  title: string;
  /** One line under "drop here" in an empty column. */
  hint: string;
  cardIds: string[];
  contextNotes: Record<string, string>;
  messages: Map<string, Message>;
  badgeAccounts: Map<string, BadgeAccount> | null;
  onRemove: (id: string) => void;
  onToggleDone: (id: string) => void;
  onOpen: (messageId: string) => void;
}

export default function KanbanColumn({
  id, title, hint, cardIds, contextNotes, messages, badgeAccounts, onRemove, onToggleDone, onOpen,
}: Props) {
  const { t } = useTranslation();
  const { setNodeRef, isOver } = useDroppable({ id });

  return (
    <section className={`kanban-column kanban-column--${id}`} aria-label={title}>
      <header className="kanban-column__header">
        <span className="kanban-column__dot" aria-hidden="true" />
        <h2 className="kanban-column__title">{title}</h2>
        <span className="kanban-column__count">{cardIds.length}</span>
      </header>
      <div
        ref={setNodeRef}
        className={`scroll-region kanban-column-scroll kanban-column__cards${isOver ? " is-over" : ""}`}
      >
        <SortableContext items={cardIds} strategy={verticalListSortingStrategy}>
          {cardIds.map((cardId) => {
            const message = messages.get(cardId) ?? null;
            return (
              <KanbanCardComponent
                key={cardId}
                id={cardId}
                column={id}
                note={contextNotes[cardId]}
                message={message}
                account={message && badgeAccounts ? badgeAccounts.get(message.account_id) ?? null : null}
                onRemove={onRemove}
                onToggleDone={onToggleDone}
                onOpen={onOpen}
              />
            );
          })}
        </SortableContext>
        {cardIds.length === 0 && (
          <div className="kanban-column__empty">
            <span>{t("kanban.dropHere")}</span>
            <span className="kanban-column__empty-hint">{hint}</span>
          </div>
        )}
      </div>
    </section>
  );
}
