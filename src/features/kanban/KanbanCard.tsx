import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Message } from "@/lib/api";
import IconButton from "@/components/ui/IconButton";
import { formatListDate } from "@/lib/formatDate";
import "./kanban.css";

interface Props {
  id: string;
  note?: string;
  message: Message | null;
  onRemove: (id: string) => void;
  onOpen: (messageId: string) => void;
}

/** Card content, shared by the sortable card and the drag overlay preview. */
export function KanbanCardBody({ id, note, message, onRemove }: Omit<Props, "onOpen">) {
  const { t, i18n } = useTranslation();
  if (!message) {
    return <span className="kanban-card__loading">{t("common.loading")}</span>;
  }
  const subject = message.subject || t("common.noSubject");
  const sender = message.from_name || message.from_address;
  return (
    <>
      <div title={subject} className="kanban-card__subject">
        {subject}
      </div>
      <div className="kanban-card__meta">
        <span title={sender} className="kanban-card__sender">
          {sender}
        </span>
        <span className="kanban-card__date">
          {formatListDate(message.date, i18n?.language)}
        </span>
        <IconButton
          label={t("kanban.removeFromBoard")}
          icon={<X size={14} />}
          onClick={(e) => { e.stopPropagation(); onRemove(id); }}
          onPointerDown={(e) => e.stopPropagation()}
          onKeyDown={(e) => e.stopPropagation()}
        />
      </div>
      {note && <div className="kanban-card__note">{note}</div>}
    </>
  );
}

/** Static copy rendered inside dnd-kit's DragOverlay while a card is dragged. */
export function KanbanCardPreview(props: Omit<Props, "onOpen" | "onRemove">) {
  return (
    <div className="kanban-card kanban-card--overlay">
      <KanbanCardBody {...props} onRemove={() => {}} />
    </div>
  );
}

export default function KanbanCard({ id, note, message, onRemove, onOpen }: Props) {
  const { t } = useTranslation();
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    // The overlay follows the pointer; the original stays as a placeholder.
    opacity: isDragging ? 0.4 : 1,
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
      className="kanban-card"
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
      <KanbanCardBody id={id} note={note} message={message} onRemove={onRemove} />
    </div>
  );
}
