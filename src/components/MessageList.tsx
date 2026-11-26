import { useRef, useMemo, useState, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import { Inbox, Archive, Trash2, MailOpen, Mail, Star, StarOff, X } from "lucide-react";
import type { MessageSummary } from "@/lib/api";
import type { Label } from "@/lib/ipc-types";
import { getMessageLabelsBatch, batchMarkRead, batchStar } from "@/lib/api";
import { useAccountsQuery, useFoldersForAccountsQuery } from "@/hooks/queries";
import { invalidateKanbanQueries } from "@/hooks/queries/useKanbanCardsQuery";
import { runMailAction, trashConfirmCopy } from "@/hooks/mutations/mailActions";
import { useMailStore } from "@/stores/mail.store";
import { eventToKeyString, useShortcutStore } from "@/stores/shortcut.store";
import { toastError, useToastStore } from "@/stores/toast.store";
import { useConfirmStore } from "@/stores/confirm.store";
import { roleForSelection } from "@/lib/folderAggregation";
import { assignAccountColors, getAccountColor, getAccountLabel } from "@/lib/accountColors";
import MessageItem from "./MessageItem";
import MoveToButton from "./MoveToButton";
import IconButton from "./ui/IconButton";
import Button from "./ui/Button";
import { MessageListSkeleton } from "./Skeleton";
import { prefetchMessage } from "@/hooks/useMessageLoader";
import { defaultPrivacyMode } from "@/lib/privacyMode";
import "./mail-list.css";

const LABEL_CHUNK_SIZE = 50;
const PAGE_STEP = 10;
/** Wait for the selection to settle (J/K held down) before warming neighbours. */
const PREFETCH_DELAY_MS = 300;

// Module-level so React Query only re-runs it when a chunk result changes.
function mergeLabelChunks(results: { data?: Record<string, Label[]> }[]) {
  const merged: Record<string, Label[]> = {};
  for (const result of results) {
    if (result.data) Object.assign(merged, result.data);
  }
  return merged;
}

interface Props {
  messages: MessageSummary[];
  selectedMessageId: string | null;
  onSelectMessage: (id: string) => void;
  loading: boolean;
  onToggleStar?: (messageId: string, newStarred: boolean) => void;
  onLoadMore?: () => void;
  hasNextPage?: boolean;
  isFetchingNextPage?: boolean;
}

type FlagAction = "markRead" | "markUnread" | "star" | "unstar";

function matchesBinding(event: React.KeyboardEvent, binding: string | undefined) {
  if (!binding) return false;
  return eventToKeyString(event.nativeEvent).toLowerCase() === binding.toLowerCase();
}

export default function MessageList({
  messages,
  selectedMessageId,
  onSelectMessage,
  loading,
  onToggleStar,
  onLoadMore,
  hasNextPage = false,
  isFetchingNextPage = false,
}: Props) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useToastStore((s) => s.addToast);
  const parentRef = useRef<HTMLDivElement>(null);
  const batchMode = useMailStore((s) => s.batchMode);
  const selectedMessageIds = useMailStore((s) => s.selectedMessageIds);
  const toggleBatchMode = useMailStore((s) => s.toggleBatchMode);
  const toggleMessageSelection = useMailStore((s) => s.toggleMessageSelection);
  const selectAllMessages = useMailStore((s) => s.selectAllMessages);
  const clearSelection = useMailStore((s) => s.clearSelection);
  const [batchLoading, setBatchLoading] = useState(false);
  const confirm = useConfirmStore((s) => s.confirm);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const { data: accounts = [] } = useAccountsQuery();
  const accountsById = useMemo(
    () => new Map(accounts.map((account) => [account.id, account])),
    [accounts],
  );
  const accountColorsById = useMemo(() => assignAccountColors(accounts), [accounts]);
  const showAccountColorMarkers = !activeAccountId && accounts.length > 1;
  const folderAccountIds = useMemo(
    () => activeAccountId ? [activeAccountId] : accounts.map((account) => account.id),
    [accounts, activeAccountId],
  );
  const { data: folders = [] } = useFoldersForAccountsQuery(folderAccountIds);
  // Offer spam action only when NOT already viewing the spam folder
  const activeFolderRole = roleForSelection(activeFolderId, folders);
  const spamFolder = activeAccountId ? folders.find((f) => f.account_id === activeAccountId && f.role === "spam") : undefined;
  const spamFolderId = activeFolderRole !== "spam" ? spamFolder?.id : undefined;
  const messageIds = useMemo(() => messages.map((m) => m.id), [messages]);
  const indexById = useMemo(() => new Map(messageIds.map((id, index) => [id, index])), [messageIds]);
  // Fetch labels per page-sized chunk so loading another page only fetches the
  // new chunk; earlier chunks keep their cache entries.
  const labelChunks = useMemo(() => {
    const chunks: string[][] = [];
    for (let start = 0; start < messageIds.length; start += LABEL_CHUNK_SIZE) {
      chunks.push(messageIds.slice(start, start + LABEL_CHUNK_SIZE));
    }
    return chunks;
  }, [messageIds]);
  const labelsByMessage = useQueries({
    queries: labelChunks.map((ids) => ({
      queryKey: ["message-labels", ids.join(",")],
      queryFn: () => getMessageLabelsBatch(ids),
      staleTime: 60_000,
    })),
    combine: mergeLabelChunks,
  });

  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76,
    getItemKey: (index) => messages[index]?.id ?? index,
    measureElement: (el) => el.getBoundingClientRect().height,
    overscan: 5,
  });

  // ── Roving tab stop ────────────────────────────────────────────────────
  // One row is tabbable (the "active" row); ↑/↓/J/K/Home/End move it.
  const [activeId, setActiveId] = useState<string | null>(null);
  const anchorIndexRef = useRef<number | null>(null);
  const effectiveActiveId =
    (activeId && indexById.has(activeId) ? activeId : null)
    ?? (selectedMessageId && indexById.has(selectedMessageId) ? selectedMessageId : null)
    ?? messageIds[0]
    ?? null;

  const focusRow = useCallback((id: string) => {
    const index = indexById.get(id);
    if (index === undefined) return;
    virtualizer.scrollToIndex(index, { align: "auto" });
    const tryFocus = (attempt: number) => {
      const row = Array.from(parentRef.current?.querySelectorAll<HTMLElement>("[data-row-id]") ?? [])
        .find((el) => el.dataset.rowId === id);
      if (row) row.focus({ preventScroll: true });
      else if (attempt < 3) requestAnimationFrame(() => tryFocus(attempt + 1));
    };
    tryFocus(0);
  }, [indexById, virtualizer]);

  // Scroll to the selected message only when the selection itself changes —
  // not on every background refresh of the list (that would yank the view
  // back while the user scrolls).
  const scrolledForRef = useRef<string | null>(null);
  useEffect(() => {
    if (!selectedMessageId) {
      scrolledForRef.current = null;
      return;
    }
    if (scrolledForRef.current === selectedMessageId) return;
    const idx = indexById.get(selectedMessageId);
    if (idx === undefined) return; // not loaded yet; retry when it is
    scrolledForRef.current = selectedMessageId;
    setActiveId(selectedMessageId);
    virtualizer.scrollToIndex(idx, { align: "auto" });
    // Keyboard selection (J/K) moves focus along when the list has it.
    if (parentRef.current?.contains(document.activeElement) && document.activeElement !== parentRef.current) {
      focusRow(selectedMessageId);
    }
  }, [selectedMessageId, indexById, virtualizer, focusRow]);

  // Warm the cache for the messages around the open one, so J/K and ↑/↓ open
  // them instantly. Debounced and cancelled when the selection moves on;
  // prefetchMessage dedupes in-flight loads and swallows errors.
  const prevNeighbourId = selectedMessageId ? messageIds[(indexById.get(selectedMessageId) ?? -1) - 1] : undefined;
  const nextNeighbourId = selectedMessageId && indexById.has(selectedMessageId)
    ? messageIds[indexById.get(selectedMessageId)! + 1]
    : undefined;
  useEffect(() => {
    if (!selectedMessageId || batchMode) return;
    const neighbours = [nextNeighbourId, prevNeighbourId].filter((id): id is string => Boolean(id));
    if (neighbours.length === 0) return;
    const timer = setTimeout(() => {
      const mode = defaultPrivacyMode();
      for (const id of neighbours) void prefetchMessage(id, mode);
    }, PREFETCH_DELAY_MS);
    return () => clearTimeout(timer);
  }, [selectedMessageId, nextNeighbourId, prevNeighbourId, batchMode]);

  const moveActive = useCallback((index: number) => {
    if (messages.length === 0) return;
    const clamped = Math.max(0, Math.min(messages.length - 1, index));
    const id = messages[clamped].id;
    setActiveId(id);
    anchorIndexRef.current = clamped;
    if (!useMailStore.getState().batchMode) {
      scrolledForRef.current = id;
      onSelectMessage(id);
    }
    focusRow(id);
    if (index >= messages.length && hasNextPage && !isFetchingNextPage) onLoadMore?.();
  }, [messages, onSelectMessage, focusRow, hasNextPage, isFetchingNextPage, onLoadMore]);

  function handleListKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.defaultPrevented) return;
    const target = event.target as HTMLElement;
    const onRow = target === event.currentTarget || target.getAttribute("role") === "option";
    if (!onRow || event.altKey) return;
    const current = effectiveActiveId ? indexById.get(effectiveActiveId) ?? 0 : 0;
    const bindings = useShortcutStore.getState().bindings;
    let next: number | null = null;
    if (event.key === "ArrowDown" || matchesBinding(event, bindings["next-message"])) next = current + 1;
    else if (event.key === "ArrowUp" || matchesBinding(event, bindings["prev-message"])) next = current - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = messages.length - 1;
    else if (event.key === "PageDown") next = current + PAGE_STEP;
    else if (event.key === "PageUp") next = current - PAGE_STEP;
    if (next === null) return;
    event.preventDefault();
    moveActive(next);
  }

  function handleRowClick(index: number, event?: React.MouseEvent) {
    const id = messages[index]?.id;
    if (!id) return;
    const state = useMailStore.getState();
    if (event && (event.metaKey || event.ctrlKey)) {
      // ⌘/Ctrl-click toggles one message into a multi-selection.
      event.preventDefault();
      const ids = new Set(state.batchMode
        ? state.selectedMessageIds
        : selectedMessageId && indexById.has(selectedMessageId) ? [selectedMessageId] : []);
      if (ids.has(id)) ids.delete(id);
      else ids.add(id);
      useMailStore.setState({ batchMode: true, selectedMessageIds: ids });
      anchorIndexRef.current = index;
      setActiveId(id);
      return;
    }
    if (event?.shiftKey) {
      // Shift-click selects the range from the last clicked row.
      event.preventDefault();
      const anchor = anchorIndexRef.current
        ?? (selectedMessageId ? indexById.get(selectedMessageId) : undefined)
        ?? index;
      const [from, to] = anchor <= index ? [anchor, index] : [index, anchor];
      const ids = new Set(state.batchMode ? state.selectedMessageIds : []);
      for (const message of messages.slice(from, to + 1)) ids.add(message.id);
      useMailStore.setState({ batchMode: true, selectedMessageIds: ids });
      setActiveId(id);
      return;
    }
    anchorIndexRef.current = index;
    setActiveId(id);
    if (state.batchMode) toggleMessageSelection(id);
    else {
      scrolledForRef.current = id;
      onSelectMessage(id);
    }
  }

  if (loading) {
    return <MessageListSkeleton />;
  }

  if (messages.length === 0) {
    return (
      <div
        className="fade-in"
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          height: "100%",
          color: "var(--color-text-secondary)",
          fontSize: "var(--font-md)",
          gap: "8px",
        }}
      >
        <Inbox size={32} strokeWidth={1.2} />
        {t("common.noMessages")}
      </div>
    );
  }

  function invalidateMessageViews() {
    queryClient.invalidateQueries({ queryKey: ["messages"] });
    queryClient.invalidateQueries({ queryKey: ["threads"] });
    queryClient.invalidateQueries({ queryKey: ["starred-messages"] });
    void invalidateKanbanQueries(queryClient);
  }

  function selectedMessages() {
    return messages.filter((m) => selectedMessageIds.has(m.id));
  }

  async function handleMoveAction(kind: "archive" | "trash") {
    const targets = selectedMessages();
    if (targets.length === 0) return;
    setBatchLoading(true);
    try {
      const result = await runMailAction(queryClient, kind, targets, {
        confirm: (plan) => confirm({ ...trashConfirmCopy(plan), destructive: true }),
        onRemoved: (ids) => {
          const current = useMailStore.getState().selectedMessageId;
          if (current && ids.includes(current)) useMailStore.getState().setSelectedMessage(null);
        },
      });
      if (result.ok) clearSelection();
    } finally {
      setBatchLoading(false);
    }
  }

  async function handleFlagAction(action: FlagAction) {
    const ids = selectedMessages().map((m) => m.id);
    if (ids.length === 0) return;
    setBatchLoading(true);
    try {
      let count = 0;
      if (action === "markRead") count = await batchMarkRead(ids, true);
      else if (action === "markUnread") count = await batchMarkRead(ids, false);
      else if (action === "star") count = await batchStar(ids, true);
      else count = await batchStar(ids, false);
      invalidateMessageViews();
      if (action === "markRead" || action === "markUnread") {
        queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
      }
      addToast({ message: t("batch.success", { count }), type: "success" });
      clearSelection();
    } catch (error) {
      toastError(error, "batch.failed");
    } finally {
      setBatchLoading(false);
    }
  }

  const allSelected = messages.length > 0 && selectedMessageIds.size === messages.length;
  const virtualItems = virtualizer.getVirtualItems();
  const activeRowRendered = virtualItems.some((item) => messages[item.index]?.id === effectiveActiveId);
  const selectionMessages = selectedMessageIds.size > 0 ? selectedMessages() : [];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {/* Batch toolbar */}
      {batchMode && (
        <div className="batch-toolbar" role="toolbar" aria-label={t("batch.toolbar", "Selection actions")}>
          <label className="batch-select-control">
            <input
              type="checkbox"
              checked={allSelected}
              onChange={() => allSelected ? clearSelection() : selectAllMessages(messageIds)}
              aria-label={t("batch.selectAll", "Select all")}
              className="batch-checkbox batch-select-all-checkbox"
            />
            <span>
              {selectedMessageIds.size > 0 ? t("batch.selected", { count: selectedMessageIds.size }) : t("batch.selectAll")}
            </span>
          </label>
          {selectedMessageIds.size > 0 && (
            <>
              <span className="toolbar-separator" aria-hidden="true" />
              <MoveToButton messages={selectionMessages} disabled={batchLoading || [...selectedMessageIds].some((id) => !indexById.has(id))}/>
              <IconButton icon={<Archive size={14} />} label={t("messageActions.archive")} onClick={() => void handleMoveAction("archive")} disabled={batchLoading} />
              <IconButton icon={<MailOpen size={14} />} label={t("batch.markRead")} onClick={() => void handleFlagAction("markRead")} disabled={batchLoading} />
              <IconButton icon={<Mail size={14} />} label={t("batch.markUnread")} onClick={() => void handleFlagAction("markUnread")} disabled={batchLoading} />
              <IconButton icon={<Star size={14} />} label={t("batch.star", "Star")} onClick={() => void handleFlagAction("star")} disabled={batchLoading} />
              <IconButton icon={<StarOff size={14} />} label={t("batch.unstar", "Unstar")} onClick={() => void handleFlagAction("unstar")} disabled={batchLoading} />
              <span className="toolbar-separator" aria-hidden="true" />
              <IconButton icon={<Trash2 size={14} />} variant="danger" label={t("common.delete")} onClick={() => void handleMoveAction("trash")} disabled={batchLoading} />
            </>
          )}
          <span style={{ flex: 1 }} />
          <IconButton icon={<X size={14} />} label={t("common.close")} onClick={toggleBatchMode} />
        </div>
      )}
      <div
        ref={parentRef}
        className="scroll-region message-list-scroll"
        role="listbox"
        aria-label={t("inbox.messageList", "Messages")}
        aria-multiselectable={batchMode || undefined}
        data-shortcut-scope="list"
        tabIndex={activeRowRendered ? -1 : 0}
        onKeyDown={handleListKeyDown}
        onFocus={(event) => {
          if (event.target === event.currentTarget && effectiveActiveId) focusRow(effectiveActiveId);
        }}
        style={{ flex: 1, overflow: "auto", outline: "none" }}
      >
        <div
          style={{
            height: `${virtualizer.getTotalSize()}px`,
            position: "relative",
          }}
        >
          {virtualItems.map((virtualItem) => {
            const message = messages[virtualItem.index];
            const account = accountsById.get(message.account_id);
            return (
              <div
                key={virtualItem.key}
                ref={virtualizer.measureElement}
                data-index={virtualItem.index}
                role="presentation"
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${virtualItem.start}px)`,
                }}
              >
                <MessageItem
                  message={message}
                  labels={labelsByMessage[message.id] ?? []}
                  isSelected={batchMode ? selectedMessageIds.has(message.id) : message.id === selectedMessageId}
                  onClick={(event) => handleRowClick(virtualItem.index, event)}
                  onOpen={() => {
                    scrolledForRef.current = message.id;
                    setActiveId(message.id);
                    onSelectMessage(message.id);
                  }}
                  onToggleStar={onToggleStar}
                  batchMode={batchMode}
                  batchSelected={selectedMessageIds.has(message.id)}
                  onToggleBatchSelect={toggleMessageSelection}
                  spamFolderId={spamFolderId}
                  folderRole={activeFolderRole}
                  tabIndex={message.id === effectiveActiveId ? 0 : -1}
                  onRowFocus={setActiveId}
                  accountColor={
                    showAccountColorMarkers
                      ? accountColorsById.get(message.account_id) ?? getAccountColor(account, message.account_id)
                      : undefined
                  }
                  accountLabel={showAccountColorMarkers ? getAccountLabel(account, message.account_id) : undefined}
                />
              </div>
            );
          })}
        </div>
        {onLoadMore && hasNextPage && (
          <div className="mail-list-load-more">
            <Button
              variant="secondary"
              size="sm"
              loading={isFetchingNextPage}
              onClick={() => {
                if (!isFetchingNextPage) onLoadMore();
              }}
            >
              {isFetchingNextPage ? t("common.loading", "Loading...") : t("common.loadMore", "Load more")}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
