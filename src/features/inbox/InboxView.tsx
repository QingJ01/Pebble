import { useMailStore } from "@/stores/mail.store";
import { useAccountsQuery, useMessagesQuery, useThreadsQuery } from "@/hooks/queries";
import { useUIStore } from "@/stores/ui.store";
import MessageList from "@/components/MessageList";
import MessageDetail from "@/components/MessageDetail";
import ThreadView from "./ThreadView";
import ThreadItem from "@/components/ThreadItem";
import SearchBar from "@/components/SearchBar";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { List, MessageSquare, Mail } from "lucide-react";
import { MessageListSkeleton } from "@/components/Skeleton";

export default function InboxView() {
  const { t } = useTranslation();
  const { setActiveView } = useUIStore();
  const {
    activeFolderId,
    selectedMessageId, setSelectedMessage,
    threadView, toggleThreadView,
    selectedThreadId, selectThread,
    setMessages,
  } = useMailStore();
  const { data: accounts = [] } = useAccountsQuery();

  const { data: messages = [], isLoading: loadingMessages } = useMessagesQuery(
    threadView ? null : activeFolderId,
  );
  const { data: threads = [], isLoading: loadingThreads } = useThreadsQuery(
    threadView ? activeFolderId : null,
  );

  useEffect(() => {
    setMessages(messages);
  }, [messages, setMessages]);

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
        <button
          onClick={toggleThreadView}
          style={{
            background: "none", border: "none", cursor: "pointer", padding: "6px 10px",
            color: "var(--color-text-secondary)", display: "flex", alignItems: "center",
            gap: "4px", fontSize: "12px", marginRight: "8px",
          }}
          title={threadView ? "Message view" : "Thread view"}
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
              onSelectThread={selectThread}
              loading={loadingThreads}
            />
          ) : (
            <MessageList
              messages={messages}
              selectedMessageId={selectedMessageId}
              onSelectMessage={(id) => setSelectedMessage(id)}
              loading={loadingMessages}
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
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: threads.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76,
  });

  if (loading) {
    return <MessageListSkeleton />;
  }

  if (threads.length === 0) {
    return (
      <div className="fade-in" style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%", color: "var(--color-text-secondary)", fontSize: "14px" }}>
        No threads
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
