import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { Check, Paperclip, RotateCcw, Star, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { KanbanColumnType, Message } from "@/lib/api";
import IconButton from "@/components/ui/IconButton";
import AccountBadge from "@/components/ui/AccountBadge";
import type { BadgeAccount } from "@/hooks/useAccountBadges";
import { formatListDate } from "@/lib/formatDate";
import "./kanban.css";

interface Props {
  id: string;
  column: KanbanColumnType;
  note?: string;
  message: Message | null;
  /** Shown when the board spans several accounts. */
  account?: BadgeAccount | null;
  onRemove: (id: string) => void;
  /** Done cards reopen into To do; others move to Done. */
  onToggleDone: (id: string) => void;
  onOpen: (messageId: string) => void;
}

type BodyProps = Omit<Props, "onOpen">;

// Card buttons must not start a drag or reach the card's own key handling.
const keepFromCard = {
  onPointerDown: (e: React.PointerEvent) => e.stopPropagation(),
  onKeyDown: (e: React.KeyboardEvent) => e.stopPropagation(),
};

/** Card content, shared by the sortable card and the drag overlay preview. */
export function KanbanCardBody({ id, column, note, message, account, onRemove, onToggleDone }: BodyProps) {
  const { t, i18n } = useTranslation();
  if (!message) {
    return <span className="kanban-card__loading">{t("common.loading")}</span>;
  }
  const subject = message.subject || t("common.noSubject");
  const sender = message.from_name || message.from_address;
  const done = column === "done";
  return (
    <>
      <div className="kanban-card__top">
        {account && <AccountBadge account={account} title={account.account_label || account.email} />}
        <span title={sender} className="kanban-card__sender">{sender}</span>
        {message.is_starred && (
          <Star className="kanban-card__flag" size={12} fill="var(--color-warning)" color="var(--color-warning)" aria-hidden="true" />
        )}
        {message.has_attachments && <Paperclip className="kanban-card__flag" size={12} aria-hidden="true" />}
        <span className="kanban-card__date">{formatListDate(message.date, i18n?.language)}</span>
      </div>
      <div title={subject} className="kanban-card__subject">{subject}</div>
      {message.snippet && <div className="kanban-card__snippet">{message.snippet}</div>}
      {note && <div className="kanban-card__note">{note}</div>}
      <div className="kanban-card__actions">
        <IconButton
          size="sm"
          variant="secondary"
          label={done ? t("kanban.reopen", "Move back to To do") : t("kanban.markDone", "Mark done")}
          icon={done ? <RotateCcw size={14} /> : <Check size={14} />}
          onClick={(e) => { e.stopPropagation(); onToggleDone(id); }}
          {...keepFromCard}
        />
        <IconButton
          size="sm"
          variant="secondary"
          label={t("kanban.removeFromBoard")}
          icon={<X size={14} />}
          onClick={(e) => { e.stopPropagation(); onRemove(id); }}
          {...keepFromCard}
        />
      </div>
    </>
  );
}

/** Static copy rendered inside dnd-kit's DragOverlay while a card is dragged. */
export function KanbanCardPreview(props: Omit<Props, "onOpen" | "onRemove" | "onToggleDone">) {
  return (
    <div className={`kanban-card kanban-card--overlay kanban-card--${props.column}`}>
      <KanbanCardBody {...props} onRemove={() => {}} onToggleDone={() => {}} />
    </div>
  );
}

export default function KanbanCard({ id, column, note, message, account, onRemove, onToggleDone, onOpen }: Props) {
  const { t } = useTranslation();
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  // Enter opens the message; Space (and arrows while dragging) go to dnd-kit.
  function handleKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (e.key === "Enter" && !isDragging && e.target === e.currentTarget) {
      e.preventDefault();
      onOpen(id);
      return;
    }
    listeners?.onKeyDown?.(e);
  }

  const subject = message ? message.subject || t("common.noSubject") : t("common.loading");
  const sender = message ? message.from_name || message.from_address : "";

  return (
    <div
      ref={setNodeRef}
      // The overlay follows the pointer; the original stays as a placeholder.
      className={`kanban-card kanban-card--${column}${isDragging ? " is-dragging" : ""}`}
      style={style}
      {...attributes}
      {...listeners}
      onKeyDown={handleKeyDown}
      onClick={() => {
        if (message) onOpen(id);
      }}
      aria-label={message
        ? t("kanban.cardAriaLabel", "{{subject}} from {{sender}}. Enter to open, Space to drag.", { subject, sender })
        : t("common.loading")}
    >
      <KanbanCardBody
        id={id}
        column={column}
        note={note}
        message={message}
        account={account}
        onRemove={onRemove}
        onToggleDone={onToggleDone}
      />
    </div>
  );
}
