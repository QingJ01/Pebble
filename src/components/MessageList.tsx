import { useRef, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Inbox, Archive, Trash2, MailOpen, MailCheck, X } from "lucide-react";
import type { MessageSummary } from "@/lib/api";
import { getMessageLabelsBatch, batchArchive, batchDelete, batchMarkRead } from "@/lib/api";
import { useMailStore } from "@/stores/mail.store";
import { useToastStore } from "@/stores/toast.store";
import MessageItem from "./MessageItem";
import { MessageListSkeleton } from "./Skeleton";

interface Props {
  messages: MessageSummary[];
  selectedMessageId: string | null;
  onSelectMessage: (id: string) => void;
  loading: boolean;
  onToggleStar?: (messageId: string, newStarred: boolean) => void;
  onLoadMore?: () => void;
}

export default function MessageList({
  messages,
  selectedMessageId,
  onSelectMessage,
  loading,
  onToggleStar,
  onLoadMore,
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
  const messageIds = useMemo(() => messages.map((m) => m.id), [messages]);
  const { data: labelsByMessage = {} } = useQuery({
    queryKey: ["message-labels", messageIds],
    queryFn: () => getMessageLabelsBatch(messageIds),
    staleTime: 60_000,
    enabled: messageIds.length > 0,
  });

  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76,
    measureElement: (el) => el.getBoundingClientRect().height,
    overscan: 5,
  });

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
          fontSize: "14px",
          gap: "8px",
        }}
      >
        <Inbox size={32} strokeWidth={1.2} />
        {t("common.noMessages")}
      </div>
    );
  }

  async function handleBatchAction(action: "archive" | "delete" | "markRead" | "markUnread") {
    const ids = [...selectedMessageIds];
    if (ids.length === 0) return;
    setBatchLoading(true);
    try {
      let count = 0;
      if (action === "archive") count = await batchArchive(ids);
      else if (action === "delete") count = await batchDelete(ids);
      else if (action === "markRead") count = await batchMarkRead(ids, true);
      else count = await batchMarkRead(ids, false);
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      addToast({ message: t("batch.success", { count }), type: "success" });
      clearSelection();
    } catch {
      addToast({ message: t("batch.failed"), type: "error" });
    } finally {
      setBatchLoading(false);
    }
  }

  const allSelected = messages.length > 0 && selectedMessageIds.size === messages.length;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {/* Batch toolbar */}
      {batchMode && (
        <div style={{
          display: "flex", alignItems: "center", gap: "6px",
          padding: "6px 10px", borderBottom: "1px solid var(--color-border)",
          backgroundColor: "var(--color-bg)", flexShrink: 0,
        }}>
          <input
            type="checkbox"
            checked={allSelected}
            onChange={() => allSelected ? clearSelection() : selectAllMessages(messageIds)}
            style={{ cursor: "pointer", accentColor: "var(--color-accent)" }}
          />
          <span style={{ fontSize: "12px", color: "var(--color-text-secondary)", marginRight: "auto" }}>
            {selectedMessageIds.size > 0 ? t("batch.selected", { count: selectedMessageIds.size }) : t("batch.selectAll")}
          </span>
          {selectedMessageIds.size > 0 && (
            <>
              <BatchBtn icon={Archive} label={t("messageActions.archive")} onClick={() => handleBatchAction("archive")} disabled={batchLoading} />
              <BatchBtn icon={Trash2} label={t("common.delete")} onClick={() => handleBatchAction("delete")} disabled={batchLoading} />
              <BatchBtn icon={MailOpen} label={t("batch.markRead")} onClick={() => handleBatchAction("markRead")} disabled={batchLoading} />
              <BatchBtn icon={MailCheck} label={t("batch.markUnread")} onClick={() => handleBatchAction("markUnread")} disabled={batchLoading} />
            </>
          )}
          <BatchBtn icon={X} label={t("common.close")} onClick={toggleBatchMode} disabled={false} />
        </div>
      )}
      <div
        ref={parentRef}
        role="listbox"
        aria-label={t("inbox.messageList", "Messages")}
        style={{ flex: 1, overflow: "auto" }}
      >
        <div
          style={{
            height: `${virtualizer.getTotalSize()}px`,
            position: "relative",
          }}
        >
          {virtualizer.getVirtualItems().map((virtualItem) => {
            const message = messages[virtualItem.index];
            return (
              <div
                key={virtualItem.key}
                ref={virtualizer.measureElement}
                data-index={virtualItem.index}
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
                  isSelected={message.id === selectedMessageId}
                  onClick={() => batchMode ? toggleMessageSelection(message.id) : onSelectMessage(message.id)}
                  onToggleStar={onToggleStar}
                  batchMode={batchMode}
                  batchSelected={selectedMessageIds.has(message.id)}
                  onToggleBatchSelect={toggleMessageSelection}
                />
              </div>
            );
          })}
        </div>
        {onLoadMore && messages.length > 0 && messages.length % 50 === 0 && (
          <div style={{ padding: "12px", textAlign: "center" }}>
            <button
              onClick={onLoadMore}
              style={{
                padding: "6px 20px",
                fontSize: "13px",
                border: "1px solid var(--color-border)",
                borderRadius: "6px",
                background: "transparent",
                color: "var(--color-text-secondary)",
                cursor: "pointer",
              }}
            >
              {t("common.loadMore", "Load more")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function BatchBtn({ icon: Icon, label, onClick, disabled }: {
  icon: React.ElementType; label: string; onClick: () => void; disabled: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={label}
      style={{
        display: "flex", alignItems: "center", padding: "4px",
        border: "none", background: "transparent", borderRadius: "4px",
        cursor: disabled ? "default" : "pointer",
        color: "var(--color-text-secondary)", opacity: disabled ? 0.5 : 1,
      }}
    >
      <Icon size={14} />
    </button>
  );
}
