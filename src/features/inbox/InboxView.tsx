import { isThreadSelected, useMailStore } from "@/stores/mail.store";
import { useAccountsQuery, useMessagesQuery, useThreadsQuery, useFoldersForAccountsQuery, patchMessagesCache } from "@/hooks/queries";
import { useUIStore } from "@/stores/ui.store";
import { toastError, useToastStore } from "@/stores/toast.store";
import MessageList from "@/components/MessageList";
import MessageDetail from "@/components/MessageDetail";
import ThreadView from "./ThreadView";
import ThreadItem from "@/components/ThreadItem";
import SearchBar from "@/components/SearchBar";
import { useRef, useState, useCallback, useEffect, useMemo } from "react";
import ConfirmDialog from "@/components/ConfirmDialog";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { List, MessageSquare, Mail, Trash2, Inbox, CheckSquare } from "lucide-react";
import { MessageListSkeleton } from "@/components/Skeleton";
import Button from "@/components/ui/Button";
import { eventToKeyString, useShortcutStore } from "@/stores/shortcut.store";
import { emptyTrash } from "@/lib/api";
import { folderIdsForSelection, roleForSelection } from "@/lib/folderAggregation";
import type { ThreadSummary } from "@/lib/api";

const EMPTY_THREADS: ThreadSummary[] = [];

export default function InboxView() {
  const { t } = useTranslation();
  const setActiveView = useUIStore((s) => s.setActiveView);
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const selectedMessageId = useMailStore((s) => s.selectedMessageId);
  const setSelectedMessage = useMailStore((s) => s.setSelectedMessage);
  const threadView = useMailStore((s) => s.threadView);
  const toggleThreadView = useMailStore((s) => s.toggleThreadView);
  const selectedThreadId = useMailStore((s) => s.selectedThreadId);
  const selectedThreadAccountId = useMailStore((s) => s.selectedThreadAccountId);
  const selectThread = useMailStore((s) => s.selectThread);
  const { data: accounts = [] } = useAccountsQuery();
  const folderAccountIds = useMemo(
    () => activeAccountId ? [activeAccountId] : accounts.map((account) => account.id),
    [accounts, activeAccountId],
  );
  const { data: folders = [] } = useFoldersForAccountsQuery(folderAccountIds);
  const queryClient = useQueryClient();
  const addToast = useToastStore((s) => s.addToast);
  const [showTrashConfirm, setShowTrashConfirm] = useState(false);

  const activeFolderRole = roleForSelection(activeFolderId, folders);
  const isTrashFolder = activeFolderRole === "trash";
  const selectedFolderIds = folderIdsForSelection(activeFolderId, folders);
  const queryFolderId = selectedFolderIds[0] ?? null;
  const queryFolderIds = selectedFolderIds.length > 1 ? selectedFolderIds : undefined;

  const {
    data: messages,
    isLoading: loadingMessages,
    hasNextPage,
    isFetchingNextPage,
    fetchNextPage,
  } = useMessagesQuery(
    threadView ? null : queryFolderId,
    threadView ? undefined : queryFolderIds,
  );
  const { data: threads = EMPTY_THREADS, isLoading: loadingThreads } = useThreadsQuery(
    threadView ? queryFolderId : null,
    50,
    0,
    threadView ? queryFolderIds : undefined,
  );
  const handleLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);
  const handleToggleStar = useCallback((messageId: string, newStarred: boolean) => {
    patchMessagesCache(queryClient, (page) =>
      page.map((m) => (m.id === messageId ? { ...m, is_starred: newStarred } : m)),
    );
  }, [queryClient]);

  const detailOpen = threadView ? selectedThreadId !== null : selectedMessageId !== null;

  // No accounts or no folder selected — show welcome / setup prompt
  if (accounts.length === 0 || !activeFolderId) {
    return (
      <div className="fade-in" style={{
        display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
        height: "100%", gap: "16px", color: "var(--color-text-secondary)",
      }}>
        <Mail size={48} strokeWidth={1.2} />
        <p style={{ fontSize: "16px", fontWeight: 500, color: "var(--color-text-primary)", margin: 0 }}>
          {t("inbox.welcome", "Welcome to Pebble")}
        </p>
        <p style={{ fontSize: "13px", margin: 0 }}>
          {t("inbox.addAccountHint", "Add an email account to get started")}
        </p>
        <Button variant="primary" onClick={() => setActiveView("settings")} style={{ marginTop: "8px" }}>
          {t("settings.addAccount", "Add Account")}
        </Button>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden" }}>
      <div className="inbox-toolbar-row">
        <div className="inbox-toolbar-search">
          <SearchBar onSearch={() => {}} onClear={() => {}} />
        </div>
        {isTrashFolder && messages.length > 0 && (
          <>
            <Button
              variant="secondary"
              size="sm"
              icon={<Trash2 size={14} />}
              onClick={() => setShowTrashConfirm(true)}
              style={{ marginRight: "4px" }}
            >
              {t("messageActions.emptyTrash", "Empty Trash")}
            </Button>
            {showTrashConfirm && (
              <ConfirmDialog
                title={t("messageActions.emptyTrash", "Empty Trash")}
                message={t("messageActions.emptyTrashConfirm", "Permanently delete all messages in Trash?")}
                confirmLabel={t("messageActions.deletePermanently", "Delete permanently")}
                destructive
                onCancel={() => setShowTrashConfirm(false)}
                onConfirm={async () => {
                  setShowTrashConfirm(false);
                  try {
                    const targetAccountIds = activeAccountId ? [activeAccountId] : accounts.map((account) => account.id);
                    const counts = await Promise.all(targetAccountIds.map((accountId) => emptyTrash(accountId)));
                    const count = counts.reduce((sum, current) => sum + current, 0);
                    queryClient.invalidateQueries({ queryKey: ["messages"] });
                    addToast({ message: t("messageActions.emptyTrashSuccess", { count }), type: "success" });
                  } catch (error) {
                    toastError(error, "messageActions.emptyTrashFailed");
                  }
                }}
              />
            )}
          </>
        )}
        {!threadView && (
          <button
            type="button"
            onClick={() => useMailStore.getState().toggleBatchMode()}
            aria-label={t("batch.toggle", "Batch select")}
            className="inbox-toolbar-button"
            title={t("batch.toggle", "Batch select")}
          >
            <CheckSquare size={16} />
          </button>
        )}
        <button
          type="button"
          onClick={toggleThreadView}
          aria-label={threadView ? t("inbox.messageView") : t("inbox.threadView")}
          className="inbox-toolbar-button inbox-toolbar-button--end"
          title={threadView ? t("inbox.messageView") : t("inbox.threadView")}
        >
          {threadView ? <List size={16} /> : <MessageSquare size={16} />}
        </button>
      </div>
      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        {/* List panel */}
        <div
          style={{
            width: detailOpen ? "clamp(260px, 32%, 360px)" : "100%",
            flexShrink: 0,
            borderRight: detailOpen ? "1px solid var(--color-border)" : "none",
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
          }}
        >
          {threadView ? (
            <ThreadList
              threads={threads}
              folderIds={selectedFolderIds}
              folderRole={activeFolderRole}
              selectedThreadId={selectedThreadId}
              selectedThreadAccountId={selectedThreadAccountId}
              onSelectThread={selectThread}
              loading={loadingThreads}
            />
          ) : (
            <MessageList
              messages={messages}
              selectedMessageId={selectedMessageId}
              onSelectMessage={setSelectedMessage}
              loading={loadingMessages}
              onLoadMore={handleLoadMore}
              hasNextPage={hasNextPage}
              isFetchingNextPage={isFetchingNextPage}
              onToggleStar={handleToggleStar}
            />
          )}
        </div>

        {/* Detail panel */}
        {detailOpen && (
          <div style={{ flex: 1, overflow: "hidden" }}>
            {threadView && selectedThreadId ? (
              <ThreadView />
            ) : selectedMessageId ? (
              <MessageDetail
                messageId={selectedMessageId}
                onBack={() => setSelectedMessage(null)}
                folderRole={activeFolderRole}
              />
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}

// Inline ThreadList component using virtualizer
function ThreadList({ threads, folderIds, folderRole, selectedThreadId, selectedThreadAccountId, onSelectThread, loading }: {
  threads: ThreadSummary[];
  folderIds: string[];
  folderRole?: ReturnType<typeof roleForSelection>;
  selectedThreadId: string | null;
  selectedThreadAccountId: string | null;
  onSelectThread: (thread: ThreadSummary) => void;
  loading: boolean;
}) {
  const { t } = useTranslation();
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: threads.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76,
    getItemKey: (index) => threads[index]?.thread_id ?? index,
    measureElement: (el) => el.getBoundingClientRect().height,
    overscan: 5,
  });
  const rowKey = (thread: ThreadSummary) => `${thread.account_id}:${thread.thread_id}`;
  const selectedIndex = threads.findIndex((thread) => isThreadSelected({ selectedThreadId, selectedThreadAccountId }, thread));
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const activeIndex = (() => {
    const byActive = activeKey ? threads.findIndex((thread) => rowKey(thread) === activeKey) : -1;
    if (byActive >= 0) return byActive;
    return selectedIndex >= 0 ? selectedIndex : 0;
  })();

  const focusRow = useCallback((index: number) => {
    const thread = threads[index];
    if (!thread) return;
    virtualizer.scrollToIndex(index, { align: "auto" });
    const key = rowKey(thread);
    const tryFocus = (attempt: number) => {
      const row = Array.from(parentRef.current?.querySelectorAll<HTMLElement>("[data-row-id]") ?? [])
        .find((el) => el.dataset.rowId === key);
      if (row) row.focus({ preventScroll: true });
      else if (attempt < 3) requestAnimationFrame(() => tryFocus(attempt + 1));
    };
    tryFocus(0);
  }, [threads, virtualizer]);

  // Scroll to the selected thread only when the selection changes, not on
  // every refetch of the thread list.
  const selectedKey = selectedThreadId ? `${selectedThreadAccountId ?? ""}:${selectedThreadId}` : null;
  const scrolledForRef = useRef<string | null>(null);
  useEffect(() => {
    if (!selectedKey) {
      scrolledForRef.current = null;
      return;
    }
    if (scrolledForRef.current === selectedKey || selectedIndex < 0) return;
    scrolledForRef.current = selectedKey;
    setActiveKey(rowKey(threads[selectedIndex]));
    virtualizer.scrollToIndex(selectedIndex, { align: "auto" });
    if (parentRef.current?.contains(document.activeElement) && document.activeElement !== parentRef.current) {
      focusRow(selectedIndex);
    }
  }, [selectedKey, selectedIndex, threads, virtualizer, focusRow]);

  if (loading) {
    return <MessageListSkeleton />;
  }

  if (threads.length === 0) {
    return (
      <div className="fade-in" style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", color: "var(--color-text-secondary)", fontSize: "var(--font-md)", gap: "8px" }}>
        <Inbox size={32} strokeWidth={1.2} />
        {t("common.noThreads")}
      </div>
    );
  }

  function moveTo(index: number) {
    const clamped = Math.max(0, Math.min(threads.length - 1, index));
    const thread = threads[clamped];
    setActiveKey(rowKey(thread));
    scrolledForRef.current = `${thread.account_id ?? ""}:${thread.thread_id}`;
    onSelectThread(thread);
    focusRow(clamped);
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.defaultPrevented || event.altKey) return;
    const target = event.target as HTMLElement;
    if (target !== event.currentTarget && target.getAttribute("role") !== "option") return;
    const bindings = useShortcutStore.getState().bindings;
    const keyString = eventToKeyString(event.nativeEvent).toLowerCase();
    let next: number | null = null;
    if (event.key === "ArrowDown" || keyString === bindings["next-message"]?.toLowerCase()) next = activeIndex + 1;
    else if (event.key === "ArrowUp" || keyString === bindings["prev-message"]?.toLowerCase()) next = activeIndex - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = threads.length - 1;
    if (next === null) return;
    event.preventDefault();
    moveTo(next);
  }

  const virtualItems = virtualizer.getVirtualItems();
  const activeRowRendered = virtualItems.some((item) => item.index === activeIndex);

  return (
    <div
      ref={parentRef}
      className="scroll-region thread-list-scroll"
      style={{ height: "100%", overflow: "auto", outline: "none" }}
      role="listbox"
      aria-label={t("inbox.threadList", "Threads")}
      data-shortcut-scope="list"
      tabIndex={activeRowRendered ? -1 : 0}
      onKeyDown={handleKeyDown}
      onFocus={(event) => {
        if (event.target === event.currentTarget) focusRow(activeIndex);
      }}
    >
      <div
        role="presentation"
        style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}
      >
        {virtualItems.map((virtualItem) => {
          const thread = threads[virtualItem.index];
          return (
            <div
              key={virtualItem.key}
              ref={virtualizer.measureElement}
              data-index={virtualItem.index}
              role="presentation"
              style={{
                position: "absolute", top: 0, left: 0, width: "100%",
                transform: `translateY(${virtualItem.start}px)`,
              }}
            >
              <ThreadItem
                thread={thread}
                isSelected={isThreadSelected({ selectedThreadId, selectedThreadAccountId }, thread)}
                onClick={() => {
                  setActiveKey(rowKey(thread));
                  scrolledForRef.current = `${thread.account_id ?? ""}:${thread.thread_id}`;
                  onSelectThread(thread);
                }}
                folderIds={folderIds}
                folderRole={folderRole}
                tabIndex={virtualItem.index === activeIndex ? 0 : -1}
                onRowFocus={(focused) => setActiveKey(rowKey(focused))}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
