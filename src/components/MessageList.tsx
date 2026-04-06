import { useRef, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useQuery } from "@tanstack/react-query";
import { Inbox } from "lucide-react";
import type { MessageSummary } from "@/lib/api";
import { getMessageLabelsBatch } from "@/lib/api";
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
  const parentRef = useRef<HTMLDivElement>(null);
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

  return (
    <div
      ref={parentRef}
      role="listbox"
      aria-label={t("inbox.messageList", "Messages")}
      style={{
        height: "100%",
        overflow: "auto",
      }}
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
                onClick={() => onSelectMessage(message.id)}
                onToggleStar={onToggleStar}
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
  );
}
