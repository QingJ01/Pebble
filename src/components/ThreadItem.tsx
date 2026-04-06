import { memo } from "react";
import { Star, Paperclip } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ThreadSummary } from "@/lib/api";

interface Props {
  thread: ThreadSummary;
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

function ThreadItem({ thread, isSelected, onClick }: Props) {
  const { t } = useTranslation();
  const hasUnread = thread.unread_count > 0;
  const fontWeight = hasUnread ? "600" : "normal";
  const participantText = thread.participants.slice(0, 3).join(", ") +
    (thread.participants.length > 3 ? ` +${thread.participants.length - 3}` : "");

  return (
    <div
      role="option"
      aria-selected={isSelected}
      tabIndex={0}
      onClick={onClick}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onClick(); } }}
      style={{
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
        if (!isSelected) e.currentTarget.style.backgroundColor = "var(--color-bg-hover)";
      }}
      onMouseLeave={(e) => {
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
          {participantText}
          {thread.message_count > 1 && (
            <span style={{ color: "var(--color-text-secondary)", fontWeight: "normal", marginLeft: "4px" }}>
              ({thread.message_count})
            </span>
          )}
        </span>
        <div style={{ display: "flex", alignItems: "center", gap: "4px", flexShrink: 0 }}>
          {thread.is_starred && <Star size={13} fill="#f59e0b" color="#f59e0b" />}
          {thread.has_attachments && <Paperclip size={13} color="var(--color-text-secondary)" />}
          <span style={{ fontSize: "11px", color: "var(--color-text-secondary)", fontWeight: "normal" }}>
            {formatDate(thread.last_date)}
          </span>
        </div>
      </div>
      <div style={{ fontSize: "12.5px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", marginBottom: "2px" }}>
        {thread.subject || t("inbox.noSubject")}
      </div>
      <div style={{ fontSize: "12px", color: "var(--color-text-secondary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontWeight: "normal" }}>
        {thread.snippet}
      </div>
    </div>
  );
}

export default memo(ThreadItem);
