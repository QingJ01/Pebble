import { useRef, useState } from "react";
import { Reply, ReplyAll, Forward, Star, Archive, Trash2, LayoutGrid, RotateCcw } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { invalidateKanbanQueries, useIsInKanban } from "@/hooks/queries/useKanbanCardsQuery";
import { addKanbanCard } from "@/hooks/mutations/useKanbanMutations";
import { runMailAction, trashConfirmCopy } from "@/hooks/mutations/mailActions";
import { toastError, useToastStore } from "@/stores/toast.store";
import { useConfirmStore } from "@/stores/confirm.store";
import { useUpdateFlagsMutation } from "@/hooks/mutations/useUpdateFlagsMutation";
import { useComposeStore } from "@/stores/compose.store";
import { restoreMessage } from "@/lib/api";
import type { Message } from "@/lib/api";
import {
  patchMessagesCache,
  snapshotMessagesCache,
  restoreMessagesCache,
} from "@/hooks/queries";
import IconButton from "./ui/IconButton";
import ContextMenu, { type ContextMenuPosition } from "./ContextMenu";
import "./mail-list.css";

interface Props {
  message: Message;
  folderRole?: string | null;
  onBack: () => void;
  onMessageUpdate: (msg: Message) => void;
}

type KanbanColumn = "todo" | "waiting" | "done";

export default function MessageActionToolbar({
  message,
  folderRole,
  onBack,
  onMessageUpdate,
}: Props) {
  const { t } = useTranslation();
  const openCompose = useComposeStore((s) => s.openCompose);
  const queryClient = useQueryClient();
  const flagsMutation = useUpdateFlagsMutation();
  const inKanban = useIsInKanban(message.id);
  const kanbanButtonRef = useRef<HTMLButtonElement>(null);
  const [kanbanMenu, setKanbanMenu] = useState<ContextMenuPosition | null>(null);
  const [busy, setBusy] = useState(false);

  function invalidateMessageViews(includeUnreadCounts = false) {
    queryClient.invalidateQueries({ queryKey: ["messages"] });
    queryClient.invalidateQueries({ queryKey: ["threads"] });
    // Deleted/moved messages drop off the board.
    void invalidateKanbanQueries(queryClient);
    if (includeUnreadCounts) {
      queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
    }
  }

  async function handleAddToKanban(column: KanbanColumn) {
    try {
      await addKanbanCard(message.id, column, queryClient);
      useToastStore.getState().addToast({
        message: t("messageActions.kanbanSuccess", "Added to kanban board"),
        type: "success",
      });
    } catch (error) {
      toastError(error, "messageActions.kanbanFailed");
    }
  }

  async function handleMoveAction(kind: "archive" | "trash") {
    if (busy) return;
    setBusy(true);
    try {
      await runMailAction(queryClient, kind, [message], {
        confirm: (plan) => useConfirmStore.getState().confirm({ ...trashConfirmCopy(plan), destructive: true }),
        onRemoved: () => onBack(),
      });
    } finally {
      setBusy(false);
    }
  }

  async function handleRestore() {
    const previousLists = snapshotMessagesCache(queryClient);
    patchMessagesCache(queryClient, (page) => page.filter((m) => m.id !== message.id));
    try {
      await restoreMessage(message.id);
      invalidateMessageViews(true);
      useToastStore.getState().addToast({
        message: t("messageActions.restoreSuccess", "Message restored to inbox"),
        type: "success",
      });
      onBack();
    } catch (error) {
      restoreMessagesCache(queryClient, previousLists);
      toastError(error, "messageActions.restoreFailed");
    }
  }

  function openKanbanMenu() {
    const rect = kanbanButtonRef.current?.getBoundingClientRect();
    setKanbanMenu({ x: rect?.left ?? 0, y: (rect?.bottom ?? 0) + 4 });
  }

  const starLabel = message.is_starred ? t("messageActions.unstar") : t("messageActions.star");
  const isArchiveFolder = folderRole === "archive";
  const archiveLabel = isArchiveFolder ? t("messageActions.unarchive", "Unarchive") : t("messageActions.archive");
  const isTrash = folderRole === "trash";

  return (
    <div
      role="toolbar"
      aria-label={t("messageActions.toolbar", "Message actions")}
      style={{
        display: "flex",
        alignItems: "center",
        gap: "2px",
        padding: "4px 16px 4px 48px",
        position: "relative",
        userSelect: "none",
        WebkitUserSelect: "none",
      }}
    >
      <IconButton size="md" label={t("messageActions.reply")} icon={<Reply size={16} />} onClick={() => openCompose("reply", message)} />
      <IconButton size="md" label={t("messageActions.replyAll")} icon={<ReplyAll size={16} />} onClick={() => openCompose("reply-all", message)} />
      <IconButton size="md" label={t("messageActions.forward")} icon={<Forward size={16} />} onClick={() => openCompose("forward", message)} />
      <IconButton
        size="md"
        label={starLabel}
        pressed={message.is_starred}
        icon={<Star size={16} fill={message.is_starred ? "currentColor" : "none"} />}
        onClick={() => {
          flagsMutation.mutate(
            { messageId: message.id, isStarred: !message.is_starred },
            { onSuccess: () => onMessageUpdate({ ...message, is_starred: !message.is_starred }) },
          );
        }}
      />
      <IconButton
        size="md"
        label={archiveLabel}
        icon={isArchiveFolder ? <RotateCcw size={16} /> : <Archive size={16} />}
        onClick={() => void handleMoveAction("archive")}
      />
      {isTrash && (
        <IconButton
          size="md"
          label={t("messageActions.restore", "Restore")}
          icon={<RotateCcw size={16} />}
          onClick={() => void handleRestore()}
        />
      )}
      <IconButton
        ref={kanbanButtonRef}
        size="md"
        label={t("messageActions.addToKanban")}
        icon={<LayoutGrid size={16} />}
        aria-haspopup="menu"
        aria-expanded={kanbanMenu !== null}
        style={inKanban ? { color: "var(--color-accent-text)" } : undefined}
        onClick={openKanbanMenu}
      />
      {/* Destructive action kept apart from the safe ones. */}
      <span className="toolbar-separator" aria-hidden="true" />
      <IconButton
        size="md"
        variant="danger"
        label={isTrash ? t("messageActions.deletePermanently", "Delete permanently") : t("messageActions.delete")}
        icon={<Trash2 size={16} />}
        onClick={() => void handleMoveAction("trash")}
      />
      {kanbanMenu && (
        <ContextMenu
          label={t("messageActions.addToKanban")}
          position={kanbanMenu}
          onClose={() => setKanbanMenu(null)}
          restoreFocusTo={kanbanButtonRef.current}
          items={([
            { col: "todo" as const, label: t("kanban.todo", "To Do") },
            { col: "waiting" as const, label: t("kanban.waiting", "Waiting") },
            { col: "done" as const, label: t("kanban.done", "Done") },
          ]).map(({ col, label }) => ({ id: col, label, onSelect: () => void handleAddToKanban(col) }))}
        />
      )}
    </div>
  );
}
