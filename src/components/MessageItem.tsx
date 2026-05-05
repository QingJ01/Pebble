import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { Star, Paperclip, Archive, LayoutGrid, ShieldAlert, RotateCcw } from "lucide-react";
import type { Folder, Label, MessageSummary } from "@/lib/api";
import { updateMessageFlags, archiveMessage, moveToFolder } from "@/lib/api";
import { useKanbanStore } from "@/stores/kanban.store";
import { useToastStore } from "@/stores/toast.store";
import { patchMessagesCache, restoreMessagesCache, snapshotMessagesCache } from "@/hooks/queries";

interface Props {
  message: MessageSummary;
  labels?: Label[];
  isSelected: boolean;
  onClick: () => void;
  onToggleStar?: (messageId: string, newStarred: boolean) => void;
  batchMode?: boolean;
  batchSelected?: boolean;
  onToggleBatchSelect?: (messageId: string) => void;
  spamFolderId?: string;
  folderRole?: Folder["role"];
  accountColor?: string;
  accountLabel?: string;
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

function MessageItem({ message, labels = [], isSelected, onClick, onToggleStar, batchMode, batchSelected, onToggleBatchSelect, spamFolderId, folderRole, accountColor, accountLabel }: Props) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [showActions, setShowActions] = useState(false);
  const fontWeight = message.is_read ? "normal" : "600";
  const inKanban = useKanbanStore((s) => s.cardIdSet.has(message.id));
  const archiveActionLabel = folderRole === "archive"
    ? t("messageActions.unarchive", "Unarchive")
    : t("messageActions.archive", "Archive");
  const ArchiveActionIcon = folderRole === "archive" ? RotateCcw : Archive;

  function invalidateMessageViews(includeUnreadCounts = false) {
    queryClient.invalidateQueries({ queryKey: ["messages"] });
    queryClient.invalidateQueries({ queryKey: ["threads"] });
    if (includeUnreadCounts) {
      queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
    }
  }

  return (
    <div
      onClick={onClick}
      tabIndex={0}
      role="option"
      aria-selected={isSelected}
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
        setShowActions(true);
        if (!isSelected) e.currentTarget.style.backgroundColor = "var(--color-bg-hover)";
      }}
      onMouseLeave={(e) => {
        setShowActions(false);
        if (!isSelected) e.currentTarget.style.backgroundColor = "transparent";
      }}
      onFocus={(e) => {
        setShowActions(true);
        if (!isSelected) e.currentTarget.style.backgroundColor = "var(--color-bg-hover)";
      }}
      onBlur={(e) => {
        // Only hide if focus leaves this element entirely (not moving to a child)
        if (!e.currentTarget.contains(e.relatedTarget as Node)) {
          setShowActions(false);
          if (!isSelected) e.currentTarget.style.backgroundColor = "transparent";
        }
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
    >
      {accountColor && (
        <span
          aria-label={accountLabel}
          title={accountLabel}
          style={{
            position: "absolute",
            left: 0,
            top: "10px",
            bottom: "10px",
            width: "3px",
            borderRadius: "0 3px 3px 0",
            backgroundColor: accountColor,
          }}
        />
      )}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "2px" }}>
        {batchMode && (
          <input
            type="checkbox"
            checked={batchSelected}
            aria-label={t("batch.selectMessage", "Select message")}
            className="batch-checkbox message-row-checkbox"
            onChange={(e) => {
              e.stopPropagation();
              onToggleBatchSelect?.(message.id);
            }}
            onClick={(e) => e.stopPropagation()}
          />
        )}
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
          {inKanban && (
            <LayoutGrid size={13} color="var(--color-accent)" />
          )}
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
        {message.subject || t("inbox.noSubject")}
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
        {labels.length > 0 && labels.map((l) => (
          <span
            key={l.id}
            style={{
              display: "inline-block",
              fontSize: "10px",
              padding: "0 5px",
              borderRadius: "3px",
              backgroundColor: l.color + "22",
              color: l.color,
              border: `1px solid ${l.color}44`,
              marginLeft: "6px",
              verticalAlign: "middle",
              lineHeight: "16px",
              fontWeight: 500,
            }}
          >
            {l.name}
          </span>
        ))}
      </div>
      {showActions && (
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
              const previousLists = snapshotMessagesCache(queryClient);
              patchMessagesCache(queryClient, (page) => page.filter((m) => m.id !== message.id));
              archiveMessage(message.id)
                .then((result) => {
                  if (result === "skipped") {
                    restoreMessagesCache(queryClient, previousLists);
                    return;
                  }
                  invalidateMessageViews(true);
                  const msg = result === "unarchived"
                    ? t("messageActions.unarchiveSuccess", "Message moved to inbox")
                    : t("messageActions.archiveSuccess", "Message archived");
                  useToastStore.getState().addToast({ message: msg, type: "success" });
                })
                .catch(() => {
                  restoreMessagesCache(queryClient, previousLists);
                  queryClient.invalidateQueries({ queryKey: ["messages"] });
                  const msg = folderRole === "archive"
                    ? t("messageActions.unarchiveFailed", "Failed to unarchive")
                    : t("messageActions.archiveFailed", "Failed to archive");
                  useToastStore.getState().addToast({ message: msg, type: "error" });
                });
            }}
            aria-label={archiveActionLabel}
            title={archiveActionLabel}
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
            <ArchiveActionIcon size={14} />
          </button>
          {spamFolderId && (
            <button
              onClick={(e) => {
                e.stopPropagation();
                const previousLists = snapshotMessagesCache(queryClient);
                patchMessagesCache(queryClient, (page) => page.filter((m) => m.id !== message.id));
                moveToFolder(message.id, spamFolderId)
                  .then(() => {
                    invalidateMessageViews(true);
                    useToastStore.getState().addToast({ message: t("messageActions.spamSuccess", "Marked as spam"), type: "success" });
                  })
                  .catch(() => {
                    restoreMessagesCache(queryClient, previousLists);
                    queryClient.invalidateQueries({ queryKey: ["messages"] });
                    useToastStore.getState().addToast({ message: t("messageActions.spamFailed", "Failed to mark as spam"), type: "error" });
                  });
              }}
              aria-label={t("messageActions.reportSpam", "Report spam")}
              title={t("messageActions.reportSpam", "Report spam")}
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
              <ShieldAlert size={14} />
            </button>
          )}
          <button
            onClick={(e) => {
              e.stopPropagation();
              useKanbanStore.getState().addCard(message.id, "todo")
                .then(() => {
                  useToastStore.getState().addToast({ message: t("messageActions.kanbanSuccess", "Added to kanban board"), type: "success" });
                })
                .catch(() => {
                  useToastStore.getState().addToast({ message: t("messageActions.kanbanFailed", "Failed to add to kanban"), type: "error" });
                });
            }}
            aria-label={t("messageActions.addToKanban")}
            title={t("messageActions.addToKanban")}
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
            <LayoutGrid size={14} />
          </button>
          <button
            onClick={(e) => {
              e.stopPropagation();
              updateMessageFlags(message.id, undefined, !message.is_starred)
                .then(() => {
                  invalidateMessageViews();
                  queryClient.invalidateQueries({ queryKey: ["starred-messages"] });
                  queryClient.invalidateQueries({ queryKey: ["message", message.id] });
                })
                .catch(console.error);
              if (onToggleStar) onToggleStar(message.id, !message.is_starred);
            }}
            aria-label={message.is_starred ? t("messageActions.unstar") : t("messageActions.star")}
            aria-pressed={message.is_starred}
            title={message.is_starred ? t("messageActions.unstar") : t("messageActions.star")}
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
        </div>
      )}
    </div>
  );
}

export default memo(MessageItem);
