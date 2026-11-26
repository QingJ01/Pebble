import { memo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Star, StarOff, Paperclip, Mail, MailOpen, Archive, Trash2, Reply, MessageSquare, FolderInput } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Folder, ThreadSummary } from "@/lib/api";
import { batchStar, listThreadMessages, updateThreadReadFlagsInFolders } from "@/lib/api";
import { toastError, useToastStore } from "@/stores/toast.store";
import { isThreadSelected, useMailStore } from "@/stores/mail.store";
import { useComposeStore } from "@/stores/compose.store";
import { useConfirmStore } from "@/stores/confirm.store";
import {
  runMailAction,
  scopedThreadMessages,
  trashConfirmCopy,
  type MailActionKind,
} from "@/hooks/mutations/mailActions";
import ContextMenu, { useContextMenuState, type ContextMenuItem } from "./ContextMenu";
import { MoveToDialog, type MoveMessage } from "./MoveToButton";
import { formatListDate } from "@/lib/formatDate";
import type { BadgeAccount } from "@/hooks/useAccountBadges";
import MailRowAccountBadge from "./MailRowAccountBadge";
import "./mail-list.css";

interface Props {
  thread: ThreadSummary;
  isSelected: boolean;
  onClick: () => void;
  folderIds: string[];
  folderRole?: Folder["role"];
  /** Roving tabindex: 0 for the list's active row, -1 for the others. */
  tabIndex?: number;
  onRowFocus?: (thread: ThreadSummary) => void;
  /** Account to badge the row with (lists spanning several accounts only). */
  account?: BadgeAccount;
}

function ThreadItem({ thread, isSelected, onClick, folderIds, folderRole, tabIndex = 0, onRowFocus, account }: Props) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const menu = useContextMenuState();
  const [moveTargets, setMoveTargets] = useState<MoveMessage[]>([]);
  const [moveOpen, setMoveOpen] = useState(false);
  const hasUnread = thread.unread_count > 0;

  function invalidateThreadViews() {
    queryClient.invalidateQueries({ queryKey: ["messages"] });
    queryClient.invalidateQueries({ queryKey: ["threads"] });
    queryClient.invalidateQueries({ queryKey: ["threadMessages", thread.thread_id] });
    queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
  }

  async function handleToggleRead() {
    const nextReadState = hasUnread;
    try {
      const result = await updateThreadReadFlagsInFolders(thread.thread_id, folderIds, nextReadState);
      if (result.failed > 0) {
        useToastStore.getState().addToast({
          message: t("messageActions.threadReadFailed", { count: result.failed }),
          type: "error",
        });
      } else if (result.updated === 0 && result.queued === 0) {
        useToastStore.getState().addToast({
          message: t("messageActions.threadReadNoMessages"),
          type: "error",
        });
      } else if (result.queued > 0) {
        useToastStore.getState().addToast({
          message: result.updated > 0
            ? t("messageActions.threadReadPartiallyUpdated", { updated: result.updated, queued: result.queued })
            : t("messageActions.threadReadQueued", { count: result.queued }),
          type: "info",
        });
      }
      invalidateThreadViews();
    } catch (error) {
      toastError(error, "messageActions.updateReadFailed");
    }
  }

  async function scopedMessages() {
    return scopedThreadMessages(thread.account_id, thread.thread_id, folderIds);
  }

  async function handleToggleStar() {
    try {
      const messages = await scopedMessages();
      if (messages.length === 0) return;
      await batchStar(messages.map((m) => m.id), !thread.is_starred);
      invalidateThreadViews();
      queryClient.invalidateQueries({ queryKey: ["starred-messages"] });
    } catch (error) {
      toastError(error, thread.is_starred ? "messageActions.unstarFailed" : "messageActions.starFailed");
    }
  }

  async function runAction(kind: MailActionKind) {
    let messages;
    try {
      messages = await scopedMessages();
    } catch (error) {
      toastError(error, kind === "archive" ? "messageActions.archiveFailed" : "messageActions.deleteFailed");
      return;
    }
    if (messages.length === 0) return;
    await runMailAction(queryClient, kind, messages, {
      onRemoved: () => {
        const state = useMailStore.getState();
        if (isThreadSelected(state, thread)) state.selectThread(null);
      },
      confirm: (plan) => useConfirmStore.getState().confirm({ ...trashConfirmCopy(plan), destructive: true }),
    });
  }

  async function handleMove() {
    let messages;
    try {
      messages = await scopedMessages();
    } catch (error) {
      toastError(error, "folderManagement.loadFailed");
      return;
    }
    // Drafts cannot be moved; move the rest of the conversation.
    const movable = messages.filter((m) => !m.is_draft);
    if (movable.length === 0) {
      if (messages.length > 0) {
        useToastStore.getState().addToast({
          type: "info",
          message: t("folderManagement.draftsProtected", "Drafts cannot be moved; finish editing first"),
        });
      }
      return;
    }
    setMoveTargets(movable.map((m) => ({ id: m.id, account_id: m.account_id, is_draft: false, thread_id: thread.thread_id })));
    setMoveOpen(true);
  }

  async function handleReply() {
    try {
      const messages = await listThreadMessages(thread.account_id, thread.thread_id);
      const latest = [...messages].sort((a, b) => b.date - a.date)[0];
      if (latest) useComposeStore.getState().openCompose("reply", latest);
    } catch (error) {
      toastError(error, "common.messageLoadFailed");
    }
  }

  const readToggleLabel = hasUnread
    ? t("messageActions.markThreadRead", "Mark thread as read")
    : t("messageActions.markThreadUnread", "Mark thread as unread");
  const participantText = thread.participants.slice(0, 3).join(", ") +
    (thread.participants.length > 3 ? ` +${thread.participants.length - 3}` : "");
  const subject = thread.subject || t("inbox.noSubject");

  const menuItems: ContextMenuItem[] = [
    { id: "open", label: t("messageActions.openThread", "Open thread"), icon: <MessageSquare size={14} />, onSelect: onClick },
    { id: "reply", label: t("messageActions.reply", "Reply"), icon: <Reply size={14} />, onSelect: () => void handleReply() },
    {
      id: "read",
      label: readToggleLabel,
      icon: hasUnread ? <MailOpen size={14} /> : <Mail size={14} />,
      onSelect: () => void handleToggleRead(),
      separatorBefore: true,
    },
    {
      id: "star",
      label: thread.is_starred ? t("messageActions.unstar", "Unstar") : t("messageActions.star", "Star"),
      icon: thread.is_starred ? <StarOff size={14} /> : <Star size={14} />,
      onSelect: () => void handleToggleStar(),
    },
    {
      id: "move",
      label: t("folderManagement.moveToMenu", "Move to…"),
      icon: <FolderInput size={14} />,
      onSelect: () => void handleMove(),
    },
    ...(folderRole !== "archive" && folderRole !== "trash"
      ? [{ id: "archive", label: t("messageActions.archive", "Archive"), icon: <Archive size={14} />, onSelect: () => void runAction("archive"), separatorBefore: true }]
      : []),
    {
      id: "delete",
      label: folderRole === "trash" ? t("messageActions.deletePermanently", "Delete permanently") : t("messageActions.delete", "Delete"),
      icon: <Trash2 size={14} />,
      onSelect: () => void runAction("trash"),
      danger: true,
      separatorBefore: folderRole === "archive" || folderRole === "trash",
    },
  ];

  return (
    <div
      className={`thread-list-row${hasUnread ? " thread-list-row--unread" : ""}`}
      role="option"
      aria-selected={isSelected}
      tabIndex={tabIndex}
      data-row-id={`${thread.account_id}:${thread.thread_id}`}
      onClick={onClick}
      onContextMenu={menu.openFromMouse}
      onFocus={(event) => {
        if (event.target === event.currentTarget) onRowFocus?.(thread);
      }}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (menu.handleTriggerKeyDown(e)) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
    >
      <div className="mail-row__line">
        <span className="mail-row__primary">
          <span className="mail-row__ellipsis" title={participantText}>
            {participantText}
            {thread.message_count > 1 && (
              <span className="mail-row__count">({thread.message_count})</span>
            )}
          </span>
          {hasUnread && (
            <span
              className="mail-row__unread-dot"
              role="img"
              aria-label={t("messageActions.unreadIndicator", "Unread")}
            />
          )}
        </span>
        <div className="mail-row__meta">
          {thread.is_starred && <Star size={13} fill="var(--color-warning)" color="var(--color-warning)" aria-hidden="true" />}
          {thread.has_attachments && <Paperclip size={13} aria-hidden="true" />}
          {account && <MailRowAccountBadge account={account} />}
          <span className="mail-row__date">
            {formatListDate(thread.last_date, i18n?.language)}
          </span>
        </div>
      </div>
      <div className="mail-row__subject mail-row__ellipsis" title={subject}>
        {subject}
      </div>
      <div className="mail-row__snippet mail-row__ellipsis">
        {thread.snippet}
      </div>
      {menu.position && (
        <ContextMenu
          label={t("messageActions.threadContextMenu", "Thread actions")}
          position={menu.position}
          items={menuItems}
          onClose={menu.close}
          restoreFocusTo={menu.trigger}
        />
      )}
      <MoveToDialog
        messages={moveTargets}
        open={moveOpen}
        onClose={() => setMoveOpen(false)}
        onMoved={(ids) => {
          const state = useMailStore.getState();
          if (moveTargets.every((m) => ids.includes(m.id)) && isThreadSelected(state, thread)) state.selectThread(null);
        }}
      />
    </div>
  );
}

export default memo(ThreadItem);
