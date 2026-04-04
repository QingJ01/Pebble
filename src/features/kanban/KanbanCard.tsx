import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { Message } from "@/lib/api";

interface Props {
  id: string;
  message: Message | null;
  onRemove: (id: string) => void;
  onOpen: (messageId: string) => void;
}

export default function KanbanCard({ id, message, onRemove, onOpen }: Props) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
    padding: "10px 12px",
    marginBottom: "6px",
    backgroundColor: "var(--color-bg)",
    border: "1px solid var(--color-border)",
    borderRadius: "8px",
    cursor: "grab",
    fontSize: "13px",
  };

  if (!message) {
    return (
      <div ref={setNodeRef} style={style} {...attributes} {...listeners}>
        <span style={{ color: "var(--color-text-secondary)" }}>Loading...</span>
      </div>
    );
  }

  return (
    <div ref={setNodeRef} style={style} {...attributes} {...listeners} onDoubleClick={() => { if (message) onOpen(id); }}>
      <div style={{ fontWeight: 600, color: "var(--color-text-primary)", marginBottom: "4px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {message.subject || "(No subject)"}
      </div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <span style={{ color: "var(--color-text-secondary)", fontSize: "12px" }}>
          {message.from_name || message.from_address}
        </span>
        <span style={{ color: "var(--color-text-secondary)", fontSize: "11px", marginRight: "auto", marginLeft: "8px" }}>
          {new Date(message.date * 1000).toLocaleDateString()}
        </span>
        <button
          onClick={(e) => { e.stopPropagation(); onRemove(id); }}
          onPointerDown={(e) => e.stopPropagation()}
          style={{
            background: "none",
            border: "none",
            cursor: "pointer",
            color: "var(--color-text-secondary)",
            fontSize: "14px",
            padding: "0 4px",
          }}
          title="Remove from board"
        >
          ×
        </button>
      </div>
    </div>
  );
}
