import { memo, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useQueryClient } from "@tanstack/react-query";
import { Star, Paperclip, Mail, MailOpen } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ThreadSummary } from "@/lib/api";
import { updateThreadReadFlagsInFolders } from "@/lib/api";
import { useClickOutside } from "@/hooks/useClickOutside";
import { useToastStore } from "@/stores/toast.store";

interface Props {
  thread: ThreadSummary;
  isSelected: boolean;
  onClick: () => void;
  folderIds: string[];
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

function ThreadItem({ thread, isSelected, onClick, folderIds }: Props) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const hasUnread = thread.unread_count > 0;
  useClickOutside(contextMenuRef, Boolean(contextMenu), () => setContextMenu(null));

  useEffect(() => {
    if (!contextMenu) return;
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") setContextMenu(null);
    }
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [contextMenu]);

  function handleContextMenu(event: React.MouseEvent) {
    event.preventDefault();
    event.stopPropagation();
    setContextMenu({ x: event.clientX, y: event.clientY });
  }

  async function handleToggleRead() {
    const nextReadState = hasUnread;
    setContextMenu(null);
    try {
      await updateThreadReadFlagsInFolders(thread.thread_id, folderIds, nextReadState);
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["threads"] });
      queryClient.invalidateQueries({ queryKey: ["threadMessages", thread.thread_id] });
      queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
    } catch {
      useToastStore.getState().addToast({
        message: t("messageActions.updateReadFailed", "Failed to update read state"),
        type: "error",
      });
    }
  }

  const readToggleLabel = hasUnread
    ? t("messageActions.markThreadRead", "Mark thread as read")
    : t("messageActions.markThreadUnread", "Mark thread as unread");
  const ReadToggleIcon = hasUnread ? MailOpen : Mail;
  const contextMenuLeft = contextMenu ? Math.min(contextMenu.x, window.innerWidth - 210) : 0;
  const contextMenuTop = contextMenu ? Math.min(contextMenu.y, window.innerHeight - 64) : 0;
  const participantText = thread.participants.slice(0, 3).join(", ") +
    (thread.participants.length > 3 ? ` +${thread.participants.length - 3}` : "");

  return (
    <div
      className={`thread-list-row${hasUnread ? " thread-list-row--unread" : ""}`}
      role="option"
      aria-selected={isSelected}
      tabIndex={0}
      onClick={onClick}
      onContextMenu={handleContextMenu}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onClick(); } }}
      style={{
        color: "var(--color-text-primary)",
        cursor: "pointer",
        padding: "10px 14px",
        borderBottom: "1px solid var(--color-border)",
        height: "76px",
        boxSizing: "border-box",
        overflow: "hidden",
        transition: "background-color 0.12s ease",
      }}
    >
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "2px" }}>
        <span
          style={{
            display: "flex",
            alignItems: "center",
            gap: "6px",
            fontSize: "13px",
            fontWeight: "600",
            overflow: "hidden",
            whiteSpace: "nowrap",
            flex: 1,
            marginRight: "8px",
            minWidth: 0,
          }}
        >
          <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
            {participantText}
            {thread.message_count > 1 && (
              <span style={{ color: "var(--color-text-secondary)", fontWeight: "normal", marginLeft: "4px" }}>
                ({thread.message_count})
              </span>
            )}
          </span>
          {hasUnread && (
            <span style={{ width: "6px", height: "6px", borderRadius: "50%", background: "var(--color-accent)", flexShrink: 0 }} />
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
      <div style={{ fontSize: "12.5px", fontWeight: hasUnread ? "600" : "400", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", marginBottom: "2px" }}>
        {thread.subject || t("inbox.noSubject")}
      </div>
      <div style={{ fontSize: "12px", color: "var(--color-text-secondary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontWeight: "normal" }}>
        {thread.snippet}
      </div>
      {contextMenu && createPortal(
        <div
          ref={contextMenuRef}
          role="menu"
          style={{
            position: "fixed",
            left: contextMenuLeft,
            top: contextMenuTop,
            minWidth: "190px",
            padding: "4px",
            borderRadius: "8px",
            border: "1px solid var(--color-border)",
            backgroundColor: "var(--color-bg)",
            boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
            zIndex: 1000,
          }}
          onClick={(event) => event.stopPropagation()}
          onKeyDown={(event) => {
            event.stopPropagation();
            if (event.key === "Escape") setContextMenu(null);
          }}
          onContextMenu={(event) => event.preventDefault()}
        >
          <button
            type="button"
            role="menuitem"
            autoFocus
            onClick={() => void handleToggleRead()}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "8px",
              width: "100%",
              padding: "7px 9px",
              border: "none",
              borderRadius: "6px",
              backgroundColor: "transparent",
              color: "var(--color-text-primary)",
              cursor: "pointer",
              fontSize: "12.5px",
              textAlign: "left",
            }}
          >
            <ReadToggleIcon size={14} />
            <span>{readToggleLabel}</span>
          </button>
        </div>,
        document.body,
      )}
    </div>
  );
}

export default memo(ThreadItem);
