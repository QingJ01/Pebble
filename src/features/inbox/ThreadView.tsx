import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, MessageSquare } from "lucide-react";
import { useMailStore } from "@/stores/mail.store";
import { useThreadMessagesQuery, useThreadsQuery } from "@/hooks/queries";
import ThreadMessageBubble from "@/components/ThreadMessageBubble";
import Spinner from "@/components/Spinner";

export default function ThreadView() {
  const { t } = useTranslation();
  const selectedThreadId = useMailStore((s) => s.selectedThreadId);
  const setSelectedThreadId = useMailStore((s) => s.setSelectedThreadId);
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const { data: threadMessages = [], isLoading } = useThreadMessagesQuery(selectedThreadId);
  const { data: threads = [] } = useThreadsQuery(activeFolderId);
  const scrollRef = useRef<HTMLDivElement>(null);

  const thread = threads.find((t) => t.thread_id === selectedThreadId);

  useEffect(() => {
    if (scrollRef.current && threadMessages.length > 0) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [threadMessages]);

  if (isLoading) {
    return <Spinner />;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", backgroundColor: "var(--color-bg)" }}>
      {/* Header */}
      <div style={{ padding: "12px 16px", borderBottom: "1px solid var(--color-border)", flexShrink: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
          <button
            onClick={() => setSelectedThreadId(null)}
            aria-label={t("thread.back", "Back")}
            style={{
              background: "none", border: "none", cursor: "pointer", padding: "4px",
              borderRadius: "4px", color: "var(--color-text-secondary)", display: "flex", alignItems: "center",
            }}
          >
            <ArrowLeft size={18} />
          </button>
          <h2 style={{
            fontSize: "15px", fontWeight: "600", color: "var(--color-text-primary)",
            overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", margin: 0, flex: 1,
          }}>
            {thread?.subject || t("thread.title")}
          </h2>
          <div style={{ display: "flex", alignItems: "center", gap: "4px", color: "var(--color-text-secondary)", fontSize: "12px" }}>
            <MessageSquare size={14} />
            <span>{threadMessages.length}</span>
          </div>
        </div>
      </div>

      {/* Messages */}
      <div ref={scrollRef} style={{ flex: 1, overflow: "auto", padding: "16px" }}>
        {threadMessages.map((msg, i) => (
          <ThreadMessageBubble
            key={msg.id}
            message={msg}
            defaultExpanded={i === threadMessages.length - 1}
          />
        ))}
      </div>
    </div>
  );
}
