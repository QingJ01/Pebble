import { useState } from "react";
import { Star, Paperclip, Archive } from "lucide-react";
import type { Message } from "@/lib/api";
import { updateMessageFlags } from "@/lib/api";

interface Props {
  message: Message;
  isSelected: boolean;
  onClick: () => void;
}

function formatDate(timestamp: number): string {
  const date = new Date(timestamp * 1000);
  const now = new Date();
  const isToday =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();

  if (isToday) {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}

export default function MessageItem({ message, isSelected, onClick }: Props) {
  const [hovered, setHovered] = useState(false);
  const fontWeight = message.is_read ? "normal" : "600";

  return (
    <div
      onClick={onClick}
      style={{
        position: "relative",
        backgroundColor: isSelected ? "var(--color-sidebar-active)" : "transparent",
        color: "var(--color-text-primary)",
        fontWeight,
        cursor: "pointer",
        padding: "10px 14px",
        borderBottom: "1px solid var(--color-border)",
        height: "76px",
        boxSizing: "border-box",
        overflow: "hidden",
        transition: "background-color 0.12s ease",
      }}
      onMouseEnter={(e) => {
        setHovered(true);
        if (!isSelected) e.currentTarget.style.backgroundColor = "var(--color-bg-hover)";
      }}
      onMouseLeave={(e) => {
        setHovered(false);
        if (!isSelected) e.currentTarget.style.backgroundColor = "transparent";
      }}
    >
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "2px" }}>
        <span
          style={{
            fontSize: "13px",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            flex: 1,
            marginRight: "8px",
          }}
        >
          {message.from_name || message.from_address}
        </span>
        <div style={{ display: "flex", alignItems: "center", gap: "4px", flexShrink: 0 }}>
          {message.is_starred && (
            <Star size={13} fill="#f59e0b" color="#f59e0b" />
          )}
          {message.has_attachments && (
            <Paperclip size={13} color="var(--color-text-secondary)" />
          )}
          <span
            style={{
              fontSize: "11px",
              color: "var(--color-text-secondary)",
              fontWeight: "normal",
            }}
          >
            {formatDate(message.date)}
          </span>
        </div>
      </div>
      <div
        style={{
          fontSize: "12.5px",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          marginBottom: "2px",
        }}
      >
        {message.subject || "(no subject)"}
      </div>
      <div
        style={{
          fontSize: "12px",
          color: "var(--color-text-secondary)",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          fontWeight: "normal",
        }}
      >
        {message.snippet}
      </div>
      {hovered && (
        <div
          onClick={(e) => e.stopPropagation()}
          style={{
            position: "absolute",
            right: "8px",
            top: "50%",
            transform: "translateY(-50%)",
            display: "flex",
            alignItems: "center",
            gap: "2px",
            backgroundColor: "var(--color-bg)",
            border: "1px solid var(--color-border)",
            borderRadius: "6px",
            padding: "2px",
            boxShadow: "0 1px 4px rgba(0,0,0,0.08)",
          }}
        >
          <button
            onClick={(e) => {
              e.stopPropagation();
              updateMessageFlags(message.id, undefined, !message.is_starred);
            }}
            style={{
              padding: "4px",
              border: "none",
              background: "transparent",
              borderRadius: "4px",
              cursor: "pointer",
              display: "flex",
              alignItems: "center",
              color: message.is_starred ? "#f59e0b" : "var(--color-text-secondary)",
            }}
          >
            <Star
              size={14}
              fill={message.is_starred ? "#f59e0b" : "none"}
              color={message.is_starred ? "#f59e0b" : "currentColor"}
            />
          </button>
          <button
            onClick={(e) => {
              e.stopPropagation();
            }}
            style={{
              padding: "4px",
              border: "none",
              background: "transparent",
              borderRadius: "4px",
              cursor: "pointer",
              display: "flex",
              alignItems: "center",
              color: "var(--color-text-secondary)",
            }}
          >
            <Archive size={14} />
          </button>
        </div>
      )}
    </div>
  );
}
