import { isThreadSelected, useMailStore } from "@/stores/mail.store";
import { MAIL_ROW_HEIGHT } from "@/components/mailRowMetrics";
import { useAccountsQuery, useMessagesQuery, useThreadsQuery, useFoldersForAccountsQuery, patchMessagesCache, onMessagesCachePatched } from "@/hooks/queries";
import { useUIStore } from "@/stores/ui.store";
import { toastError, useToastStore } from "@/stores/toast.store";
import MessageList from "@/components/MessageList";
import MessageDetail from "@/components/MessageDetail";
import ThreadView from "./ThreadView";
import ThreadItem from "@/components/ThreadItem";
import SearchBar from "@/components/SearchBar";
import { useRef, useState, useCallback, useEffect, useMemo } from "react";
import ConfirmDialog from "@/components/ConfirmDialog";
import PaneSplitter, { usePaneWidth } from "@/components/PaneSplitter";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { List, MessageSquare, Mail, Trash2, Inbox, CheckSquare } from "lucide-react";
import { MessageListSkeleton } from "@/components/Skeleton";
import Button from "@/components/ui/Button";
import IconButton from "@/components/ui/IconButton";
import "@/components/mail-list.css";
import { eventToKeyString, useShortcutStore } from "@/stores/shortcut.store";
import { emptyTrash } from "@/lib/api";
import { folderIdsForSelection, isAllAccountsFolderId, roleForSelection } from "@/lib/folderAggregation";
import { useAccountBadges, withResolvedColors } from "@/hooks/useAccountBadges";
import AccountFilterBar from "./AccountFilterBar";
import type { MessageSummary, ThreadSummary } from "@/lib/api";
import { useStickyRows } from "./stickyRows";
import { clearActiveMailList, setActiveMailList } from "@/lib/activeMailList";
import { revalidateMessage } from "@/hooks/useMessageLoader";

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
  const unreadOnly = useMailStore((s) => s.unreadOnly);
  const toggleUnreadOnly = useMailStore((s) => s.toggleUnreadOnly);
  const selectedThreadId = useMailStore((s) => s.selectedThreadId);
  const selectedThreadAccountId = useMailStore((s) => s.selectedThreadAccountId);
  const selectThread = useMailStore((s) => s.selectThread);
  const batchMode = useMailStore((s) => s.batchMode);
  const unifiedAccountFilter = useMailStore((s) => s.unifiedAccountFilter);
  const setUnifiedAccountFilter = useMailStore((s) => s.setUnifiedAccountFilter);
  const { data: accounts = [] } = useAccountsQuery();
  const folderAccountIds = useMemo(
    () => activeAccountId ? [activeAccountId] : accounts.map((account) => account.id),
    [accounts, activeAccountId],
  );
  const { data: folders = [] } = useFoldersForAccountsQuery(folderAccountIds);
  const queryClient = useQueryClient();
  const addToast = useToastStore((s) => s.addToast);
  const [showTrashConfirm, setShowTrashConfirm] = useState(false);
  const listPane = usePaneWidth("pebble-inbox-list-width", 320, 260, 560);

  const activeFolderRole = roleForSelection(activeFolderId, folders);
  const isTrashFolder = activeFolderRole === "trash";
  // Unified view with several accounts: the account filter bar narrows the
  // merged query to one account's folders (a stale filter is ignored).
  const showAccountFilter = !activeAccountId && isAllAccountsFolderId(activeFolderId) && accounts.length > 1;
  const accountFilter = showAccountFilter && accounts.some((account) => account.id === unifiedAccountFilter)
    ? unifiedAccountFilter
    : null;
  const spansAccounts = showAccountFilter && !accountFilter;
  const filterAccounts = useMemo(() => {
    if (!showAccountFilter) return [];
    return withResolvedColors(accounts);
  }, [accounts, showAccountFilter]);
  const selectedFolderIds = folderIdsForSelection(activeFolderId, folders, accountFilter);
  const queryFolderId = selectedFolderIds[0] ?? null;
  const queryFolderIds = selectedFolderIds.length > 1 ? selectedFolderIds : undefined;

  const {
    data: serverMessages,
    isLoading: loadingMessages,
    hasNextPage,
    isFetchingNextPage,
    fetchNextPage,
    dataUpdatedAt: messagesUpdatedAt,
  } = useMessagesQuery(
    threadView ? null : queryFolderId,
    threadView ? undefined : queryFolderIds,
    unreadOnly,
  );
  const { data: serverThreads = EMPTY_THREADS, isLoading: loadingThreads } = useThreadsQuery(
    threadView ? queryFolderId : null,
    50,
    0,
    threadView ? queryFolderIds : undefined,
    unreadOnly,
  );
  // Unread-only: the open row stays listed (now read) while it is open,
  // instead of vanishing under the reader.
  const listResetKey = `${threadView}|${activeFolderId}|${accountFilter}|${unreadOnly}`;
  const messages = useStickyRows<MessageSummary>({
    rows: serverMessages,
    enabled: unreadOnly && !threadView,
    resetKey: listResetKey,
    openedKey: selectedMessageId,
    keyOf: (m) => m.id,
    markRead: (m) => (m.is_read ? m : { ...m, is_read: true }),
    compareNewestFirst: (a, b) => b.date - a.date || (b.id < a.id ? -1 : b.id > a.id ? 1 : 0),
    subscribePatches: onMessagesCachePatched,
  });
  const threads = useStickyRows<ThreadSummary>({
    rows: serverThreads,
    enabled: unreadOnly && threadView,
    resetKey: listResetKey,
    openedKey: selectedThreadId ? `${selectedThreadAccountId ?? ""}:${selectedThreadId}` : null,
    keyOf: (thread) => `${thread.account_id}:${thread.thread_id}`,
    markRead: (thread) => (thread.unread_count === 0 ? thread : { ...thread, unread_count: 0 }),
    compareNewestFirst: (a, b) => b.last_date - a.last_date,
  });
  // Global J/K walks the list on screen, not whichever list was cached first.
  useEffect(() => {
    setActiveMailList(threadView ? { messages: null, threads } : { messages, threads: null });
  });
  useEffect(() => clearActiveMailList, []);

  // The open message left the list without the user acting on it (sync, an
  // outside delete): have the reader re-check it instead of showing it stale.
  // Keyed on the list's update time, so it runs once per list change.
  useEffect(() => {
    if (threadView || loadingMessages || !selectedMessageId || !messagesUpdatedAt) return;
    if (!messages.some((m) => m.id === selectedMessageId)) revalidateMessage(selectedMessageId);
  }, [messagesUpdatedAt, selectedMessageId, loadingMessages, threadView]); // eslint-disable-line react-hooks/exhaustive-deps

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
      <div className="fade-in mail-empty-state mail-empty-state--welcome">
        <Mail size={48} strokeWidth={1.2} />
        <p className="mail-empty-state__title">
          {t("inbox.welcome", "Welcome to Pebble")}
        </p>
        <p className="mail-empty-state__detail">
          {t("inbox.addAccountHint", "Add an email account to get started")}
        </p>
        <Button variant="primary" onClick={() => setActiveView("settings")}>
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
              variant="danger-ghost"
              icon={<Trash2 size={14} />}
              onClick={() => setShowTrashConfirm(true)}
            >
              {t("messageActions.emptyTrash", "Empty Trash")}
            </Button>
            {showTrashConfirm && (
              <ConfirmDialog
                title={t("messageActions.emptyTrash", "Empty Trash")}
                message={activeAccountId || accountFilter
                  ? t("messageActions.emptyTrashConfirm", "Permanently delete all messages in this account's Trash? This can't be undone.")
                  : t("messageActions.emptyTrashConfirmAll", "Permanently delete all messages in Trash for all accounts? This can't be undone.")}
                confirmLabel={t("messageActions.deletePermanently", "Delete permanently")}
                destructive
                onCancel={() => setShowTrashConfirm(false)}
                onConfirm={async () => {
                  setShowTrashConfirm(false);
                  try {
                    const scopedAccountId = activeAccountId ?? accountFilter;
                    const targetAccountIds = scopedAccountId ? [scopedAccountId] : accounts.map((account) => account.id);
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
        <IconButton
          size="md"
          label={unreadOnly
            ? t("inbox.showAll", "Show all mail")
            : t("inbox.showUnreadOnly", "Show unread only")}
          icon={<Mail size={16} />}
          pressed={unreadOnly}
          onClick={toggleUnreadOnly}
        />
        {!threadView && (
          <IconButton
            size="md"
            label={t("batch.toggle", "Batch select")}
            icon={<CheckSquare size={16} />}
            pressed={batchMode}
            onClick={() => useMailStore.getState().toggleBatchMode()}
          />
        )}
        <IconButton
          size="md"
          label={threadView ? t("inbox.messageView") : t("inbox.threadView")}
          icon={threadView ? <List size={16} /> : <MessageSquare size={16} />}
          onClick={toggleThreadView}
        />
      </div>
      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        {/* List panel */}
        <div
          id="inbox-list-pane"
          style={{
            width: detailOpen ? `${listPane.width}px` : "100%",
            maxWidth: detailOpen ? "55%" : undefined,
            flexShrink: 0,
            borderRight: detailOpen ? "1px solid var(--color-border)" : "none",
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
          }}
        >
          {showAccountFilter && (
            <AccountFilterBar
              accounts={filterAccounts}
              folders={folders}
              role={activeFolderRole}
              value={accountFilter}
              onChange={setUnifiedAccountFilter}
            />
          )}
          {threadView ? (
            <ThreadList
              threads={threads}
              folderIds={selectedFolderIds}
              folderRole={activeFolderRole}
              selectedThreadId={selectedThreadId}
              selectedThreadAccountId={selectedThreadAccountId}
              onSelectThread={selectThread}
              loading={loadingThreads}
              spansAccounts={spansAccounts}
              emptyLabel={unreadOnly ? t("inbox.noUnread", "No unread mail") : undefined}
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
              spansAccounts={spansAccounts}
              emptyLabel={unreadOnly ? t("inbox.noUnread", "No unread mail") : undefined}
            />
          )}
        </div>

        {detailOpen && (
          <PaneSplitter
            width={listPane.width}
            min={listPane.min}
            max={listPane.max}
            onChange={listPane.setWidth}
            onReset={listPane.reset}
            controls="inbox-list-pane"
          />
        )}

        {/* Detail panel */}
        {detailOpen && (
          <div style={{ flex: 1, minWidth: 0, overflow: "hidden" }}>
            {threadView && selectedThreadId ? (
              <ThreadView />
            ) : selectedMessageId ? (
              <MessageDetail
                messageId={selectedMessageId}
                onBack={() => setSelectedMessage(null)}
                onReopen={(id) => {
                  // Only if the user is still on the same list and hasn't
                  // opened something else meanwhile (this closure holds the
                  // folder and filter the action started in).
                  const state = useMailStore.getState();
                  if (
                    state.selectedMessageId === null
                    && state.activeFolderId === activeFolderId
                    && state.activeAccountId === activeAccountId
                    && state.unifiedAccountFilter === unifiedAccountFilter
                  ) {
                    setSelectedMessage(id);
                  }
                }}
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
function ThreadList({
  threads, folderIds, folderRole, selectedThreadId, selectedThreadAccountId, onSelectThread, loading, spansAccounts = false, emptyLabel,
}: {
  threads: ThreadSummary[];
  folderIds: string[];
  folderRole?: ReturnType<typeof roleForSelection>;
  selectedThreadId: string | null;
  selectedThreadAccountId: string | null;
  onSelectThread: (thread: ThreadSummary) => void;
  loading: boolean;
  /** The unfiltered unified view: badge rows by account even on a one-account page. */
  spansAccounts?: boolean;
  /** Text for the empty list (defaults to "No threads"). */
  emptyLabel?: string;
}) {
  const { t } = useTranslation();
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: threads.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => MAIL_ROW_HEIGHT,
    getItemKey: (index) => {
      const thread = threads[index];
      // thread_id is only unique per account.
      return thread ? `${thread.account_id}:${thread.thread_id}` : index;
    },
    measureElement: (el) => el.getBoundingClientRect().height,
    overscan: 5,
  });
  const threadAccountIds = useMemo(() => threads.map((thread) => thread.account_id), [threads]);
  const badgeAccounts = useAccountBadges(threadAccountIds, spansAccounts);
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
      <div className="fade-in mail-empty-state">
        <Inbox size={32} strokeWidth={1.2} />
        {emptyLabel ?? t("common.noThreads")}
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
                account={badgeAccounts?.get(thread.account_id)}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
