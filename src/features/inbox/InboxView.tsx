import { useMailStore } from "@/stores/mail.store";
import { useAccountsQuery, useMessagesQuery, useThreadsQuery, useFoldersForAccountsQuery, patchMessagesCache } from "@/hooks/queries";
import { useUIStore } from "@/stores/ui.store";
import { useToastStore } from "@/stores/toast.store";
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
  const setSelectedThreadId = useMailStore((s) => s.setSelectedThreadId);
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
        <button
          onClick={() => setActiveView("settings")}
          style={{
            marginTop: "8px", padding: "8px 20px", borderRadius: "6px",
            border: "none", backgroundColor: "var(--color-accent)", color: "#fff",
            fontSize: "13px", fontWeight: 600, cursor: "pointer",
          }}
        >
          {t("settings.addAccount", "Add Account")}
        </button>
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
            <button
              onClick={() => setShowTrashConfirm(true)}
              style={{
                background: "none", border: "1px solid var(--color-border)", cursor: "pointer",
                padding: "4px 10px", borderRadius: "4px", color: "var(--color-text-secondary)",
                display: "flex", alignItems: "center", gap: "4px", fontSize: "12px", marginRight: "4px",
              }}
              title={t("messageActions.emptyTrash", "Empty Trash")}
            >
              <Trash2 size={14} />
              {t("messageActions.emptyTrash", "Empty Trash")}
            </button>
            {showTrashConfirm && (
              <ConfirmDialog
                title={t("messageActions.emptyTrash", "Empty Trash")}
                message={t("messageActions.emptyTrashConfirm", "Permanently delete all messages in Trash?")}
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
                  } catch {
                    addToast({ message: t("messageActions.emptyTrashFailed"), type: "error" });
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
              selectedThreadId={selectedThreadId}
              onSelectThread={setSelectedThreadId}
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
function ThreadList({ threads, selectedThreadId, onSelectThread, loading }: {
  threads: ThreadSummary[];
  selectedThreadId: string | null;
  onSelectThread: (id: string) => void;
  loading: boolean;
}) {
  const { t } = useTranslation();
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: threads.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76,
    measureElement: (el) => el.getBoundingClientRect().height,
    overscan: 5,
  });

  // Scroll selected thread into view on keyboard navigation
  useEffect(() => {
    if (!selectedThreadId) return;
    const idx = threads.findIndex((t) => t.thread_id === selectedThreadId);
    if (idx >= 0) {
      virtualizer.scrollToIndex(idx, { align: "auto" });
    }
  }, [selectedThreadId, threads, virtualizer]);

  if (loading) {
    return <MessageListSkeleton />;
  }

  if (threads.length === 0) {
    return (
      <div className="fade-in" style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", color: "var(--color-text-secondary)", fontSize: "14px", gap: "8px" }}>
        <Inbox size={32} strokeWidth={1.2} />
        {t("common.noThreads")}
      </div>
    );
  }

  return (
    <div ref={parentRef} className="scroll-region thread-list-scroll" style={{ height: "100%", overflow: "auto" }}>
      <div
        role="listbox"
        aria-label={t("inbox.threadList", "Threads")}
        style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}
      >
        {virtualizer.getVirtualItems().map((virtualItem) => {
          const thread = threads[virtualItem.index];
          return (
            <div
              key={virtualItem.key}
              ref={virtualizer.measureElement}
              data-index={virtualItem.index}
              style={{
                position: "absolute", top: 0, left: 0, width: "100%",
                transform: `translateY(${virtualItem.start}px)`,
              }}
            >
              <ThreadItem
                thread={thread}
                isSelected={thread.thread_id === selectedThreadId}
                onClick={() => onSelectThread(thread.thread_id)}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
