import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Message } from "@/lib/api";
import IconButton from "@/components/ui/IconButton";

interface Props {
  id: string;
  note?: string;
  message: Message | null;
  onRemove: (id: string) => void;
  onOpen: (messageId: string) => void;
}

const cardStyle: React.CSSProperties = {
  padding: "10px 12px",
  marginBottom: "6px",
  backgroundColor: "var(--color-bg)",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-lg)",
  fontSize: "var(--font-base)",
  textAlign: "left",
  width: "100%",
  boxSizing: "border-box",
  color: "inherit",
  userSelect: "none",
  WebkitUserSelect: "none",
};

/** Card content, shared by the sortable card and the drag overlay preview. */
export function KanbanCardBody({ id, note, message, onRemove }: Omit<Props, "onOpen">) {
  const { t, i18n } = useTranslation();
  if (!message) {
    return <span style={{ color: "var(--color-text-secondary)" }}>{t("common.loading")}</span>;
  }
  const subject = message.subject || t("common.noSubject");
  const sender = message.from_name || message.from_address;
  return (
    <>
      <div
        title={subject}
        style={{ fontWeight: 600, color: "var(--color-text-primary)", marginBottom: "4px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
      >
        {subject}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: "8px", minWidth: 0 }}>
        <span
          title={sender}
          style={{ color: "var(--color-text-secondary)", fontSize: "var(--font-sm)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0 }}
        >
          {sender}
        </span>
        <span style={{ color: "var(--color-text-secondary)", fontSize: "var(--font-xs)", marginRight: "auto", whiteSpace: "nowrap" }}>
          {new Date(message.date * 1000).toLocaleDateString(i18n?.language)}
        </span>
        <IconButton
          label={t("kanban.removeFromBoard")}
          icon={<X size={14} />}
          onClick={(e) => { e.stopPropagation(); onRemove(id); }}
          onPointerDown={(e) => e.stopPropagation()}
          onKeyDown={(e) => e.stopPropagation()}
          style={{ marginRight: "-6px" }}
        />
      </div>
      {note && (
        <div
          style={{
            marginTop: "8px",
            padding: "6px 8px",
            borderRadius: "var(--radius-md)",
            backgroundColor: "var(--color-bg-secondary)",
            color: "var(--color-text-secondary)",
            fontSize: "var(--font-sm)",
            lineHeight: 1.4,
            whiteSpace: "pre-wrap",
            wordBreak: "break-word",
          }}
        >
          {note}
        </div>
      )}
    </>
  );
}

/** Static copy rendered inside dnd-kit's DragOverlay while a card is dragged. */
export function KanbanCardPreview(props: Omit<Props, "onOpen" | "onRemove">) {
  return (
    <div style={{ ...cardStyle, cursor: "grabbing", boxShadow: "var(--shadow-popover)" }}>
      <KanbanCardBody {...props} onRemove={() => {}} />
    </div>
  );
}

export default function KanbanCard({ id, note, message, onRemove, onOpen }: Props) {
  const { t } = useTranslation();
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id });

  const style: React.CSSProperties = {
    ...cardStyle,
    transform: CSS.Transform.toString(transform),
    transition,
    // The overlay follows the pointer; the original stays as a placeholder.
    opacity: isDragging ? 0.4 : 1,
    cursor: "pointer",
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
