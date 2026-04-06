import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useQuery } from "@tanstack/react-query";
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
}

export default function MessageList({
  messages,
  selectedMessageId,
  onSelectMessage,
  loading,
  onToggleStar,
}: Props) {
  const { t } = useTranslation();
  const parentRef = useRef<HTMLDivElement>(null);
  const messageIds = messages.map((message) => message.id);
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
          alignItems: "center",
          justifyContent: "center",
          height: "100%",
          color: "var(--color-text-secondary)",
          fontSize: "14px",
        }}
      >
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
    </div>
  );
}
