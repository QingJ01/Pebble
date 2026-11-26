import { useLayoutEffect, useRef, useState, type RefObject } from "react";
import {
  Reply, ReplyAll, Forward, Star, StarOff, Archive, Trash2, LayoutGrid, RotateCcw, FolderInput, MoreHorizontal,
} from "lucide-react";
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
import ContextMenu, { type ContextMenuItem, type ContextMenuPosition } from "./ContextMenu";
import { MoveToDialog, moveDisabledReason } from "./MoveToButton";
import "./mail-list.css";

interface Props {
  message: Message;
  folderRole?: string | null;
  onBack: () => void;
  /** Reopen this message after an archive/delete failed (the list was rolled back). */
  onReopen?: (messageId: string) => void;
  onMessageUpdate: (msg: Message) => void;
}

type KanbanColumn = "todo" | "waiting" | "done";

/** Toolbar button (--control-md 32) + gap (--space-1 4). */
const BUTTON_STEP = 36;
/** .toolbar-separator: 1px + --space-1 margin each side. */
const SEPARATOR_WIDTH = 9;

/**
 * True when `ref` is narrower than `minWidth` (ResizeObserver). Without
 * ResizeObserver (old engines, tests) it reports false: everything visible.
 */
export function useIsNarrower(ref: RefObject<HTMLElement | null>, minWidth: number): boolean {
  const [narrow, setNarrow] = useState(false);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const measure = (width: number) => setNarrow(width > 0 && width < minWidth);
    measure(element.getBoundingClientRect().width);
    const observer = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      measure(entry?.contentRect?.width ?? element.getBoundingClientRect().width);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref, minWidth]);
  return narrow;
}

export default function MessageActionToolbar({
  message,
  folderRole,
  onBack,
  onReopen,
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
  const toolbarRef = useRef<HTMLDivElement>(null);
  const moreButtonRef = useRef<HTMLButtonElement>(null);
  const [moreMenu, setMoreMenu] = useState<ContextMenuPosition | null>(null);
  const [moveOpen, setMoveOpen] = useState(false);

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
        onFailed: () => onReopen?.(message.id),
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
  const moveLabel = t("folderManagement.moveTo", "Move to folder");
  const moveBlocked = moveDisabledReason([message], t);

  // Reply, archive (+ restore in Trash) and delete always stay visible; the
  // rest collapses into "More" when the reading pane is too narrow for one row.
  const fullButtonCount = 8 + (isTrash ? 1 : 0);
  const narrow = useIsNarrower(toolbarRef, fullButtonCount * BUTTON_STEP + SEPARATOR_WIDTH);

  function toggleStar() {
    flagsMutation.mutate(
      { messageId: message.id, isStarred: !message.is_starred },
      { onSuccess: () => onMessageUpdate({ ...message, is_starred: !message.is_starred }) },
    );
  }

  function openMoreMenu() {
    const rect = moreButtonRef.current?.getBoundingClientRect();
    setMoreMenu({ x: rect?.left ?? 0, y: (rect?.bottom ?? 0) + 4 });
  }

  const overflowItems: ContextMenuItem[] = [
    { id: "reply-all", label: t("messageActions.replyAll"), icon: <ReplyAll size={14} />, onSelect: () => openCompose("reply-all", message) },
    { id: "forward", label: t("messageActions.forward"), icon: <Forward size={14} />, onSelect: () => openCompose("forward", message) },
    {
      id: "star",
      label: starLabel,
      icon: message.is_starred ? <StarOff size={14} /> : <Star size={14} />,
      onSelect: toggleStar,
      separatorBefore: true,
    },
    {
      id: "move",
      label: t("folderManagement.moveToMenu", "Move to…"),
      icon: <FolderInput size={14} />,
      onSelect: () => setMoveOpen(true),
      disabled: moveBlocked !== null,
    },
    {
      id: "kanban",
      label: t("messageActions.addToKanban"),
      icon: <LayoutGrid size={14} />,
      onSelect: () => void handleAddToKanban("todo"),
    },
  ];

  return (
    <div
      ref={toolbarRef}
      role="toolbar"
      aria-label={t("messageActions.toolbar", "Message actions")}
      className={narrow ? "message-toolbar message-toolbar--narrow" : "message-toolbar"}
    >
      <IconButton size="md" label={t("messageActions.reply")} icon={<Reply size={16} />} onClick={() => openCompose("reply", message)} />
      {!narrow && (
        <>
          <IconButton size="md" label={t("messageActions.replyAll")} icon={<ReplyAll size={16} />} onClick={() => openCompose("reply-all", message)} />
          <IconButton size="md" label={t("messageActions.forward")} icon={<Forward size={16} />} onClick={() => openCompose("forward", message)} />
          <IconButton
            size="md"
            label={starLabel}
            pressed={message.is_starred}
            icon={<Star size={16} fill={message.is_starred ? "currentColor" : "none"} />}
            onClick={toggleStar}
          />
        </>
      )}
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
      {!narrow && (
        <>
          <IconButton
            size="md"
            label={moveLabel}
            tooltip={moveBlocked ?? moveLabel}
            icon={<FolderInput size={16} />}
            disabled={moveBlocked !== null}
            onClick={() => setMoveOpen(true)}
          />
          <IconButton
            ref={kanbanButtonRef}
            size="md"
            label={t("messageActions.addToKanban")}
            icon={<LayoutGrid size={16} />}
            aria-haspopup="menu"
            aria-expanded={kanbanMenu !== null}
            pressed={inKanban}
            onClick={openKanbanMenu}
          />
        </>
      )}
      {narrow && (
        <IconButton
          ref={moreButtonRef}
          size="md"
          label={t("messageActions.moreActions", "More actions")}
          icon={<MoreHorizontal size={16} />}
          aria-haspopup="menu"
          aria-expanded={moreMenu !== null}
          onClick={openMoreMenu}
        />
      )}
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
      {moreMenu && (
        <ContextMenu
          label={t("messageActions.moreActions", "More actions")}
          position={moreMenu}
          onClose={() => setMoreMenu(null)}
          restoreFocusTo={moreButtonRef.current}
          items={overflowItems}
        />
      )}
      <MoveToDialog
        messages={[message]}
        open={moveOpen}
        onClose={() => setMoveOpen(false)}
        onMoved={(ids) => { if (ids.includes(message.id)) onBack(); }}
      />
    </div>
  );
}
