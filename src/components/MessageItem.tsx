import { memo, useState } from "react";
import MoveToButton, { MoveToDialog, moveDisabledReason } from "./MoveToButton";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import {
  Star, StarOff, Paperclip, Archive, LayoutGrid, ShieldAlert, RotateCcw, Mail, MailOpen,
  Reply, ReplyAll, Forward, Trash2, FolderInput,
} from "lucide-react";
import type { EmailAddress, Folder, Label, MessageSummary } from "@/lib/api";
import { getMessage, restoreMessage, updateMessageFlags } from "@/lib/api";
import { useIsInKanban } from "@/hooks/queries/useKanbanCardsQuery";
import { addKanbanCard } from "@/hooks/mutations/useKanbanMutations";
import { runMailAction, trashConfirmCopy, type MailActionKind } from "@/hooks/mutations/mailActions";
import { toastError, useToastStore } from "@/stores/toast.store";
import { useMailStore } from "@/stores/mail.store";
import { useComposeStore } from "@/stores/compose.store";
import { useConfirmStore } from "@/stores/confirm.store";
import { patchMessagesCache, restoreMessagesCache, snapshotMessagesCache } from "@/hooks/queries";
import IconButton from "./ui/IconButton";
import ContextMenu, { useContextMenuState, type ContextMenuItem } from "./ContextMenu";
import { formatListDate } from "@/lib/formatDate";
import type { BadgeAccount } from "@/hooks/useAccountBadges";
import MailRowAccountBadge from "./MailRowAccountBadge";
import "./mail-list.css";

interface Props {
  message: MessageSummary;
  labels?: Label[];
  isSelected: boolean;
  /** Row click. Receives the mouse event so lists can handle ⌘/Ctrl/Shift-click. */
  onClick: (event?: React.MouseEvent) => void;
  /** Enter on the row. Defaults to onClick. */
  onOpen?: () => void;
  onToggleStar?: (messageId: string, newStarred: boolean) => void;
  batchMode?: boolean;
  batchSelected?: boolean;
  onToggleBatchSelect?: (messageId: string) => void;
  spamFolderId?: string;
  folderRole?: Folder["role"];
  /** Account to badge the row with (lists spanning several accounts only). */
  account?: BadgeAccount;
  /** Roving tabindex: 0 for the list's active row, -1 for the others. */
  tabIndex?: number;
  /** The row received focus (lists use it to move their roving tab stop). */
  onRowFocus?: (messageId: string) => void;
}

function addressLabel(address: EmailAddress): string {
  return address.name?.trim() || address.address;
}

function recipientLabel(addresses: EmailAddress[]): string {
  return addresses.map(addressLabel).filter(Boolean).join(", ");
}

const MAX_INLINE_LABELS = 2;

function MessageItem({
  message, labels = [], isSelected, onClick, onOpen, onToggleStar, batchMode, batchSelected,
  onToggleBatchSelect, spamFolderId, folderRole, account, tabIndex = 0, onRowFocus,
}: Props) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const menu = useContextMenuState();
  const [moveOpen, setMoveOpen] = useState(false);
  const inKanban = useIsInKanban(message.id);
  const isArchiveFolder = folderRole === "archive";
  const archiveActionLabel = isArchiveFolder
    ? t("messageActions.unarchive", "Unarchive")
    : t("messageActions.archive", "Archive");
  const ArchiveActionIcon = isArchiveFolder ? RotateCcw : Archive;
  const primaryContact = folderRole === "sent" && message.to_list.length > 0
    ? recipientLabel(message.to_list)
    : message.from_name || message.from_address;
  const subject = message.subject || t("inbox.noSubject");
  const starLabel = message.is_starred ? t("messageActions.unstar") : t("messageActions.star");

  function invalidateMessageViews(includeUnreadCounts = false) {
    queryClient.invalidateQueries({ queryKey: ["messages"] });
    queryClient.invalidateQueries({ queryKey: ["threads"] });
    queryClient.invalidateQueries({ queryKey: ["starred-messages"] });
    if (includeUnreadCounts) {
      queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
    }
  }

  function closeIfSelected() {
    if (useMailStore.getState().selectedMessageId === message.id) {
      useMailStore.getState().setSelectedMessage(null);
    }
  }

  function runAction(kind: MailActionKind) {
    void runMailAction(queryClient, kind, [message], {
      spamFolderId,
      onRemoved: closeIfSelected,
      confirm: (plan) => useConfirmStore.getState().confirm({ ...trashConfirmCopy(plan), destructive: true }),
    });
  }

  async function handleToggleRead() {
    const nextReadState = !message.is_read;
    try {
      await updateMessageFlags(message.id, nextReadState);
      invalidateMessageViews(true);
    } catch (error) {
      toastError(error, "messageActions.updateReadFailed");
    }
  }

  function handleToggleStar() {
    const nextStarred = !message.is_starred;
    const previousLists = snapshotMessagesCache(queryClient);
    onToggleStar?.(message.id, nextStarred);
    updateMessageFlags(message.id, undefined, nextStarred)
      .then(() => {
        invalidateMessageViews();
        queryClient.invalidateQueries({ queryKey: ["message", message.id] });
      })
      .catch((error) => {
        restoreMessagesCache(queryClient, previousLists);
        onToggleStar?.(message.id, message.is_starred);
        invalidateMessageViews();
        queryClient.invalidateQueries({ queryKey: ["message", message.id] });
        toastError(error, message.is_starred ? "messageActions.unstarFailed" : "messageActions.starFailed");
      });
  }

  function handleAddToKanban() {
    addKanbanCard(message.id, "todo", queryClient)
      .then(() => {
        useToastStore.getState().addToast({ message: t("messageActions.kanbanSuccess", "Added to kanban board"), type: "success" });
      })
      .catch((error) => toastError(error, "messageActions.kanbanFailed"));
  }

  async function handleRestore() {
    const previousLists = snapshotMessagesCache(queryClient);
    patchMessagesCache(queryClient, (page) => page.filter((m) => m.id !== message.id));
    try {
      await restoreMessage(message.id);
      closeIfSelected();
      invalidateMessageViews(true);
      useToastStore.getState().addToast({ message: t("messageActions.restoreSuccess", "Message restored to inbox"), type: "success" });
    } catch (error) {
      restoreMessagesCache(queryClient, previousLists);
      toastError(error, "messageActions.restoreFailed");
    }
  }

  function openCompose(mode: "reply" | "reply-all" | "forward") {
    getMessage(message.id)
      .then((full) => {
        if (full) useComposeStore.getState().openCompose(mode, full);
      })
      .catch((error) => toastError(error, "common.messageLoadFailed"));
  }

  const menuItems: ContextMenuItem[] = [
    { id: "reply", label: t("messageActions.reply", "Reply"), icon: <Reply size={14} />, onSelect: () => openCompose("reply") },
    { id: "reply-all", label: t("messageActions.replyAll", "Reply All"), icon: <ReplyAll size={14} />, onSelect: () => openCompose("reply-all") },
    { id: "forward", label: t("messageActions.forward", "Forward"), icon: <Forward size={14} />, onSelect: () => openCompose("forward") },
    {
      id: "read",
      label: message.is_read ? t("messageActions.markUnread", "Mark as unread") : t("messageActions.markRead", "Mark as read"),
      icon: message.is_read ? <Mail size={14} /> : <MailOpen size={14} />,
      onSelect: () => void handleToggleRead(),
      separatorBefore: true,
    },
    {
      id: "star",
      label: starLabel,
      icon: message.is_starred ? <StarOff size={14} /> : <Star size={14} />,
      onSelect: handleToggleStar,
    },
    { id: "kanban", label: t("messageActions.addToKanban", "Add to Kanban"), icon: <LayoutGrid size={14} />, onSelect: handleAddToKanban },
    {
      id: "move",
      label: t("folderManagement.moveToMenu", "Move to…"),
      icon: <FolderInput size={14} />,
      onSelect: () => setMoveOpen(true),
      disabled: moveDisabledReason([message], t) !== null,
    },
    ...(folderRole === "trash"
      ? [{ id: "restore", label: t("messageActions.restore", "Restore"), icon: <RotateCcw size={14} />, onSelect: () => void handleRestore(), separatorBefore: true }]
      : [{ id: "archive", label: archiveActionLabel, icon: <ArchiveActionIcon size={14} />, onSelect: () => runAction("archive"), separatorBefore: true }]),
    ...(spamFolderId
      ? [{ id: "spam", label: t("messageActions.reportSpam", "Report spam"), icon: <ShieldAlert size={14} />, onSelect: () => runAction("spam") }]
      : []),
    {
      id: "delete",
      label: folderRole === "trash" ? t("messageActions.deletePermanently", "Delete permanently") : t("messageActions.delete", "Delete"),
      icon: <Trash2 size={14} />,
      onSelect: () => runAction("trash"),
      danger: true,
    },
  ];

  const inlineLabels = labels.slice(0, MAX_INLINE_LABELS);
  const hiddenLabelCount = labels.length - inlineLabels.length;

  return (
    <div
      className={`message-list-row${message.is_read ? "" : " message-list-row--unread"}`}
      onClick={(event) => onClick(event)}
      onContextMenu={menu.openFromMouse}
      tabIndex={tabIndex}
      role="option"
      aria-selected={isSelected}
      data-row-id={message.id}
      onFocus={(event) => {
        if (event.target === event.currentTarget) onRowFocus?.(message.id);
      }}
      onKeyDown={(e) => {
        // Keys pressed on the row's own buttons/checkbox belong to them.
        if (e.target !== e.currentTarget) return;
        if (menu.handleTriggerKeyDown(e)) return;
        if (e.key === "Enter") {
          e.preventDefault();
          (onOpen ?? onClick)();
        } else if (e.key === " ") {
          e.preventDefault();
          if (batchMode && onToggleBatchSelect) onToggleBatchSelect(message.id);
          else onClick();
        }
      }}
    >
      <div className="mail-row__line">
        {batchMode && (
          <input
            type="checkbox"
            checked={batchSelected}
            tabIndex={-1}
            aria-label={t("batch.selectMessage", "Select message")}
            className="batch-checkbox message-row-checkbox"
            onChange={(e) => {
              e.stopPropagation();
              onToggleBatchSelect?.(message.id);
            }}
            onClick={(e) => e.stopPropagation()}
          />
        )}
        <span className="mail-row__primary">
          <span className="mail-row__ellipsis" title={primaryContact}>
            {primaryContact}
          </span>
          {!message.is_read && (
            <span
              className="mail-row__unread-dot"
              role="img"
              aria-label={t("messageActions.unreadIndicator", "Unread")}
            />
          )}
        </span>
        <div className="mail-row__meta">
          {inlineLabels.map((l) => (
            <span key={l.id} className="chip mail-row__label" title={l.name}>
              <span className="mail-row__label-dot" style={{ backgroundColor: l.color }} aria-hidden="true" />
              <span className="mail-row__label-text">{l.name}</span>
            </span>
          ))}
          {hiddenLabelCount > 0 && (
            <span className="mail-row__date" title={labels.slice(MAX_INLINE_LABELS).map((l) => l.name).join(", ")}>
              +{hiddenLabelCount}
            </span>
          )}
          {inKanban && (
            <LayoutGrid size={13} aria-hidden="true" />
          )}
          {message.is_starred && (
            <Star size={13} fill="var(--color-warning)" color="var(--color-warning)" aria-hidden="true" />
          )}
          {message.has_attachments && (
            <Paperclip size={13} aria-hidden="true" />
          )}
          {account && <MailRowAccountBadge account={account} />}
          <span className="mail-row__date">
            {formatListDate(message.date, i18n?.language)}
          </span>
        </div>
      </div>
      <div className="mail-row__subject mail-row__ellipsis" title={subject}>
        {subject}
      </div>
      <div className="mail-row__snippet mail-row__ellipsis">
        {message.snippet}
      </div>
      <div className="message-row-actions" onClick={(e) => e.stopPropagation()}>
        <MoveToButton messages={[message]} />
        <IconButton
          label={archiveActionLabel}
          icon={<ArchiveActionIcon size={14} />}
          onClick={(e) => {
            e.stopPropagation();
            runAction("archive");
          }}
        />
        {spamFolderId && (
          <IconButton
            label={t("messageActions.reportSpam", "Report spam")}
            icon={<ShieldAlert size={14} />}
            onClick={(e) => {
              e.stopPropagation();
              runAction("spam");
            }}
          />
        )}
        <IconButton
          label={t("messageActions.addToKanban")}
          icon={<LayoutGrid size={14} />}
          onClick={(e) => {
            e.stopPropagation();
            handleAddToKanban();
          }}
        />
        <IconButton
          label={starLabel}
          className="message-row-actions__star"
          pressed={message.is_starred}
          icon={<Star size={14} fill={message.is_starred ? "currentColor" : "none"} />}
          onClick={(e) => {
            e.stopPropagation();
            handleToggleStar();
          }}
        />
      </div>
      {menu.position && (
        <ContextMenu
          label={t("messageActions.contextMenu", "Message actions")}
          position={menu.position}
          items={menuItems}
          onClose={menu.close}
          restoreFocusTo={menu.trigger}
        />
      )}
      <MoveToDialog messages={[message]} open={moveOpen} onClose={() => setMoveOpen(false)} />
    </div>
  );
}

export default memo(MessageItem);
