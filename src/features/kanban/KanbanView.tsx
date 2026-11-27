import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  closestCorners,
  useSensor,
  useSensors,
  type Announcements,
  type DragEndEvent,
  type DragOverEvent,
  type DragStartEvent,
  type UniqueIdentifier,
} from "@dnd-kit/core";
import { arrayMove, sortableKeyboardCoordinates } from "@dnd-kit/sortable";
import type { KanbanCard, KanbanColumnType, Message } from "@/lib/api";
import { getMessagesBatch } from "@/lib/api";
import { useKanbanContextNotesQuery, type KanbanContextNotes } from "@/hooks/queries/useKanbanContextNotesQuery";
import { useKanbanCardsQuery } from "@/hooks/queries/useKanbanCardsQuery";
import {
  addKanbanCard,
  useRemoveKanbanCardMutation,
  useReorderKanbanColumnMutation,
} from "@/hooks/mutations/useKanbanMutations";
import { toastError, useToastStore } from "@/stores/toast.store";
import { useUIStore } from "@/stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import KanbanColumn from "./KanbanColumn";
import { KanbanCardPreview } from "./KanbanCard";
import { KanbanSkeleton } from "@/components/Skeleton";
import { useAccountBadges } from "@/hooks/useAccountBadges";
import "./kanban.css";

const COLUMN_IDS: { id: KanbanColumnType; titleKey: string; hintKey: string; hint: string }[] = [
  { id: "todo", titleKey: "kanban.todo", hintKey: "kanban.todoHint", hint: "Mail you still need to act on" },
  { id: "waiting", titleKey: "kanban.waiting", hintKey: "kanban.waitingHint", hint: "Waiting on someone else" },
  { id: "done", titleKey: "kanban.done", hintKey: "kanban.doneHint", hint: "Finished items land here" },
];

type ColumnItems = Record<KanbanColumnType, string[]>;

const NO_CARDS: KanbanCard[] = [];
const NO_NOTES: KanbanContextNotes = {};
/** Ignore the click that a pointer drag can leave behind on the card. */
const CLICK_AFTER_DRAG_MS = 250;

export function columnsFromCards(cards: KanbanCard[]): ColumnItems {
  const items: ColumnItems = { todo: [], waiting: [], done: [] };
  for (const card of [...cards].sort((a, b) => a.position - b.position)) {
    items[card.column]?.push(card.message_id);
  }
  return items;
}

function isColumnId(id: UniqueIdentifier): id is KanbanColumnType {
  return COLUMN_IDS.some((column) => column.id === id);
}

function findColumn(items: ColumnItems, id: UniqueIdentifier): KanbanColumnType | null {
  if (isColumnId(id)) return id;
  for (const column of COLUMN_IDS) {
    if (items[column.id].includes(String(id))) return column.id;
  }
  return null;
}

/** Move `activeId` into the column of `overId`, before `overId` (or at the end of an empty column). */
export function moveAcrossColumns(items: ColumnItems, activeId: string, overId: UniqueIdentifier): ColumnItems {
  const from = findColumn(items, activeId);
  const to = findColumn(items, overId);
  if (!from || !to || from === to) return items;
  const target = items[to].filter((id) => id !== activeId);
  const overIndex = isColumnId(overId) ? target.length : target.indexOf(String(overId));
  const insertAt = overIndex < 0 ? target.length : overIndex;
  target.splice(insertAt, 0, activeId);
  return {
    ...items,
    [from]: items[from].filter((id) => id !== activeId),
    [to]: target,
  };
}

export default function KanbanView() {
  const { t } = useTranslation();
  // Always refetch when the board opens: cards can go stale while messages
  // are deleted or moved elsewhere in the app.
  const { data: cards = NO_CARDS, isPending } = useKanbanCardsQuery({ refetchOnMount: "always" });
  // Notes are optional: a failed load leaves the board usable without them.
  const { data: contextNotes = NO_NOTES } = useKanbanContextNotesQuery({ refetchOnMount: "always" });
  const queryClient = useQueryClient();
  const reorderColumn = useReorderKanbanColumnMutation();
  const removeCard = useRemoveKanbanCardMutation();
  const [messages, setMessages] = useState<Map<string, Message>>(new Map());
  const [dragItems, setDragItems] = useState<ColumnItems | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  const lastDragEndRef = useRef(0);

  const baseItems = useMemo(() => columnsFromCards(cards), [cards]);
  const items = dragItems ?? baseItems;
  // Account badges only when the board holds mail from several accounts.
  const cardAccountIds = useMemo(() => [...messages.values()].map((m) => m.account_id), [messages]);
  const badgeAccounts = useAccountBadges(cardAccountIds);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
      // Enter is reserved for opening the card; Space picks up and drops.
      keyboardCodes: { start: ["Space"], cancel: ["Escape"], end: ["Space", "Enter"] },
    }),
  );

  // Load message details for all cards (batch)
  useEffect(() => {
    const toLoad = cards.filter((c) => !messages.has(c.message_id));
    if (toLoad.length === 0) return;
    let cancelled = false;
    getMessagesBatch(toLoad.map((c) => c.message_id)).then((msgs) => {
      if (cancelled) return;
      setMessages((current) => {
        const next = new Map(current);
        for (const msg of msgs) {
          next.set(msg.id, msg);
        }
        return next;
      });
    }).catch((error) => {
      if (!cancelled) toastError(error, "kanban.loadFailed");
    });
    return () => { cancelled = true; };
  }, [cards]); // eslint-disable-line react-hooks/exhaustive-deps

  function handleOpenMessage(messageId: string) {
    if (Date.now() - lastDragEndRef.current < CLICK_AFTER_DRAG_MS) return;
    useMailStore.getState().setSelectedMessage(messageId);
    useUIStore.getState().setActiveView("inbox");
  }

  function subjectOf(id: UniqueIdentifier) {
    const message = messages.get(String(id));
    return message?.subject || t("common.noSubject");
  }

  function columnTitle(id: UniqueIdentifier | undefined | null) {
    if (id === undefined || id === null) return "";
    const column = findColumn(items, id);
    const def = COLUMN_IDS.find((c) => c.id === column);
    return def ? t(def.titleKey) : "";
  }

  const announcements: Announcements = {
    onDragStart: ({ active }) => t("kanban.dragStart", "Picked up {{subject}}.", { subject: subjectOf(active.id) }),
    onDragOver: ({ active, over }) => over
      ? t("kanban.dragOver", "{{subject}} is over {{column}}.", { subject: subjectOf(active.id), column: columnTitle(over.id) })
      : undefined,
    onDragEnd: ({ active, over }) => over
      ? t("kanban.dragEnd", "Dropped {{subject}} in {{column}}.", { subject: subjectOf(active.id), column: columnTitle(over.id) })
      : t("kanban.dragCancel", "Moving {{subject}} was cancelled.", { subject: subjectOf(active.id) }),
    onDragCancel: ({ active }) => t("kanban.dragCancel", "Moving {{subject}} was cancelled.", { subject: subjectOf(active.id) }),
  };

  function resetDrag() {
    setDragItems(null);
    setActiveId(null);
    lastDragEndRef.current = Date.now();
  }

  function handleDragStart(event: DragStartEvent) {
    setActiveId(String(event.active.id));
    setDragItems(baseItems);
  }

  // Cross-column moves happen while dragging so the card shows where it lands.
  function handleDragOver(event: DragOverEvent) {
    const { active, over } = event;
    if (!over) return;
    setDragItems((current) => moveAcrossColumns(current ?? baseItems, String(active.id), over.id));
  }

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    const current = dragItems ?? baseItems;
    const activeKey = String(active.id);
    const originalColumn = cards.find((c) => c.message_id === activeKey)?.column;
    resetDrag();
    if (!over || !originalColumn) return;

    const column = findColumn(current, activeKey);
    if (!column) return;
    let ordered = current[column];
    if (!isColumnId(over.id)) {
      const oldIndex = ordered.indexOf(activeKey);
      const newIndex = ordered.indexOf(String(over.id));
      if (oldIndex !== -1 && newIndex !== -1 && oldIndex !== newIndex) {
        ordered = arrayMove(ordered, oldIndex, newIndex);
      }
    }

    const unchanged = column === originalColumn
      && ordered.length === baseItems[column].length
      && ordered.every((id, index) => baseItems[column][index] === id);
    if (unchanged) return;
    // Persist the whole target column order; this also moves the dragged card
    // into the column at its drop position.
    reorderColumn.mutate({ column, orderedIds: ordered });
  }

  /** Mark done (to the top of Done), or reopen a done card into To do. */
  function handleToggleDone(messageId: string) {
    const from = findColumn(baseItems, messageId);
    if (!from) return;
    const to: KanbanColumnType = from === "done" ? "todo" : "done";
    reorderColumn.mutate({ column: to, orderedIds: [messageId, ...baseItems[to].filter((id) => id !== messageId)] });
  }

  async function handleRemove(messageId: string) {
    const card = cards.find((c) => c.message_id === messageId);
    const oldColumn = card?.column ?? "todo";
    try {
      await removeCard.mutateAsync({ messageId });
    } catch {
      return; // rolled back + error toast; the card is still on the board
    }
    useToastStore.getState().addToast({
      message: t("kanban.removedFromBoard", "Card removed from board"),
      type: "info",
      action: {
        label: t("kanban.undoRemove", "Undo"),
        onClick: () => {
          addKanbanCard(messageId, oldColumn, queryClient).catch((error) => toastError(error, "kanban.restoreFailed"));
        },
      },
    });
  }

  if (isPending) {
    return <KanbanSkeleton />;
  }

  const openCount = items.todo.length + items.waiting.length;
  return (
    <div className="kanban-view">
      <header className="kanban-view__header">
        <h1 className="kanban-view__title">{t("kanban.title", "Kanban board")}</h1>
        <p className="kanban-view__summary">
          {cards.length === 0
            ? t("kanban.emptySummary", "Add mail from the reader's toolbar to track it here.")
            : t("kanban.summary", "{{open}} open · {{done}} done", { open: openCount, done: items.done.length })}
        </p>
      </header>
    <div className="kanban-board">
      <DndContext
        sensors={sensors}
        collisionDetection={closestCorners}
        onDragStart={handleDragStart}
        onDragOver={handleDragOver}
        onDragEnd={handleDragEnd}
        onDragCancel={resetDrag}
        accessibility={{
          announcements,
          screenReaderInstructions: {
            draggable: t(
              "kanban.dragInstructions",
              "Press Space to pick up a card. Use the arrow keys to move it, Space or Enter to drop it, Escape to cancel.",
            ),
          },
        }}
      >
        {COLUMN_IDS.map((col) => (
          <KanbanColumn
            key={col.id}
            id={col.id}
            title={t(col.titleKey)}
            hint={t(col.hintKey, col.hint)}
            cardIds={items[col.id]}
            contextNotes={contextNotes}
            messages={messages}
            badgeAccounts={badgeAccounts}
            onRemove={handleRemove}
            onToggleDone={handleToggleDone}
            onOpen={handleOpenMessage}
          />
        ))}
        {typeof document !== "undefined" && createPortal(
          <DragOverlay zIndex={1000 /* --z-menu */}>
            {activeId ? (
              <KanbanCardPreview
                id={activeId}
                column={findColumn(items, activeId) ?? "todo"}
                note={contextNotes[activeId]}
                message={messages.get(activeId) ?? null}
                account={(() => {
                  const message = messages.get(activeId);
                  return message && badgeAccounts ? badgeAccounts.get(message.account_id) ?? null : null;
                })()}
              />
            ) : null}
          </DragOverlay>,
          document.body,
        )}
      </DndContext>
    </div>
    </div>
  );
}
