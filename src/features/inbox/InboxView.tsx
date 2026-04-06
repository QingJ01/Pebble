import { useMailStore } from "@/stores/mail.store";
import { useAccountsQuery, useMessagesQuery, useThreadsQuery, useFoldersQuery } from "@/hooks/queries";
import { useUIStore } from "@/stores/ui.store";
import { useToastStore } from "@/stores/toast.store";
import MessageList from "@/components/MessageList";
import MessageDetail from "@/components/MessageDetail";
import ThreadView from "./ThreadView";
import ThreadItem from "@/components/ThreadItem";
import SearchBar from "@/components/SearchBar";
import { useRef, useState, useCallback } from "react";
import ConfirmDialog from "@/components/ConfirmDialog";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { List, MessageSquare, Mail, Trash2, Inbox, CheckSquare } from "lucide-react";
import { MessageListSkeleton } from "@/components/Skeleton";
import { emptyTrash } from "@/lib/api";
import type { MessageSummary, ThreadSummary } from "@/lib/api";

const EMPTY_MESSAGES: MessageSummary[] = [];
const EMPTY_THREADS: ThreadSummary[] = [];

export default function InboxView() {
  const { t } = useTranslation();
  const { setActiveView } = useUIStore();
  const {
    activeFolderId,
    activeAccountId,
    selectedMessageId, setSelectedMessage,
    threadView, toggleThreadView,
    selectedThreadId, setSelectedThreadId,
  } = useMailStore();
  const { data: accounts = [] } = useAccountsQuery();
  const { data: folders = [] } = useFoldersQuery(activeAccountId);
  const queryClient = useQueryClient();
  const addToast = useToastStore((s) => s.addToast);
  const [showTrashConfirm, setShowTrashConfirm] = useState(false);

  const activeFolder = folders.find((f) => f.id === activeFolderId);
  const isTrashFolder = activeFolder?.role === "trash";

  // Collect all folder IDs with the same role for merged queries (e.g. spam + virus + ad)
  const siblingFolderIds = (() => {
    if (!activeFolder?.role) return undefined;
    const ids = folders.filter((f) => f.role === activeFolder.role).map((f) => f.id);
    return ids.length > 1 ? ids : undefined;
  })();

  const [messageLimit, setMessageLimit] = useState(50);
  const { data: messages = EMPTY_MESSAGES, isLoading: loadingMessages } = useMessagesQuery(
    threadView ? null : activeFolderId,
    messageLimit, 0,
    threadView ? undefined : siblingFolderIds,
  );
  const { data: threads = EMPTY_THREADS, isLoading: loadingThreads } = useThreadsQuery(
    threadView ? activeFolderId : null,
  );
  const handleLoadMore = useCallback(() => setMessageLimit((prev) => prev + 50), []);

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
      <div style={{ display: "flex", alignItems: "center" }}>
        <div style={{ flex: 1 }}>
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
                  if (!activeAccountId) return;
                  try {
                    const count = await emptyTrash(activeAccountId);
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
            onClick={() => useMailStore.getState().toggleBatchMode()}
            style={{
              background: "none", border: "none", cursor: "pointer", padding: "6px 10px",
              color: "var(--color-text-secondary)", display: "flex", alignItems: "center",
              fontSize: "12px",
            }}
            title={t("batch.toggle", "Batch select")}
          >
            <CheckSquare size={16} />
          </button>
        )}
        <button
          onClick={toggleThreadView}
          style={{
            background: "none", border: "none", cursor: "pointer", padding: "6px 10px",
            color: "var(--color-text-secondary)", display: "flex", alignItems: "center",
            gap: "4px", fontSize: "12px", marginRight: "8px",
          }}
          title={threadView ? t("inbox.messageView") : t("inbox.threadView")}
        >
          {threadView ? <List size={16} /> : <MessageSquare size={16} />}
        </button>
      </div>
      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        {/* List panel */}
        <div
          style={{
            width: detailOpen ? "360px" : "100%",
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
              onSelectMessage={(id) => setSelectedMessage(id)}
              loading={loadingMessages}
              onLoadMore={handleLoadMore}
              onToggleStar={(messageId, newStarred) => {
                queryClient.setQueriesData<MessageSummary[]>(
                  { queryKey: ["messages"] },
                  (old) => old?.map((m) =>
                    m.id === messageId ? { ...m, is_starred: newStarred } : m,
                  ),
                );
              }}
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
                folderRole={activeFolder?.role}
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
  threads: { thread_id: string; subject: string; snippet: string; last_date: number; message_count: number; unread_count: number; is_starred: boolean; participants: string[]; has_attachments: boolean }[];
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
    <div ref={parentRef} style={{ height: "100%", overflow: "auto" }}>
      <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
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
