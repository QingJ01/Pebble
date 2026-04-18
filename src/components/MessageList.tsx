import { useRef, useMemo, useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Inbox, Archive, Trash2, MailOpen, MailCheck, Star, X } from "lucide-react";
import type { MessageSummary } from "@/lib/api";
import { getMessageLabelsBatch, batchArchive, batchDelete, batchMarkRead, batchStar } from "@/lib/api";
import { useFoldersQuery } from "@/hooks/queries";
import { useMailStore } from "@/stores/mail.store";
import { useToastStore } from "@/stores/toast.store";
import { useConfirmStore } from "@/stores/confirm.store";
import MessageItem from "./MessageItem";
import { MessageListSkeleton } from "./Skeleton";

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
  const { data: folders = [] } = useFoldersQuery(activeAccountId);
  // Offer spam action only when NOT already viewing the spam folder
  const activeFolder = folders.find((f) => f.id === activeFolderId);
  const spamFolder = folders.find((f) => f.role === "spam");
  const spamFolderId = activeFolder?.role !== "spam" ? spamFolder?.id : undefined;
  const messageIds = useMemo(() => messages.map((m) => m.id), [messages]);
  const messageIdsKey = useMemo(() => messageIds.join(","), [messageIds]);
  const { data: labelsByMessage = {} } = useQuery({
    queryKey: ["message-labels", messageIdsKey],
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

  // Scroll selected message into view on keyboard navigation
  useEffect(() => {
    if (!selectedMessageId) return;
    const idx = messages.findIndex((m) => m.id === selectedMessageId);
    if (idx >= 0) {
      virtualizer.scrollToIndex(idx, { align: "auto" });
    }
  }, [selectedMessageId, messages, virtualizer]);

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

  function invalidateMessageViews() {
    queryClient.invalidateQueries({ queryKey: ["messages"] });
    queryClient.invalidateQueries({ queryKey: ["threads"] });
    queryClient.invalidateQueries({ queryKey: ["starred-messages"] });
  }

  async function handleBatchAction(action: "archive" | "delete" | "markRead" | "markUnread" | "star" | "unstar") {
    const ids = [...selectedMessageIds];
    if (ids.length === 0) return;
    if (action === "delete") {
      const count = ids.length;
      const ok = await confirm({
        title: t("batch.deleteTitle", { count, defaultValue: `Delete ${count} messages?` }),
        message: t("batch.deleteConfirm", {
          count,
          defaultValue: `This will move ${count} message(s) to Trash. This action can be undone by restoring from Trash.`,
        }),
        confirmLabel: t("batch.deleteButton", { defaultValue: "Delete" }),
        destructive: true,
      });
      if (!ok) return;
    }
    setBatchLoading(true);
    try {
      let count = 0;
      if (action === "archive") count = await batchArchive(ids);
      else if (action === "delete") count = await batchDelete(ids);
      else if (action === "markRead") count = await batchMarkRead(ids, true);
      else if (action === "markUnread") count = await batchMarkRead(ids, false);
      else if (action === "star") count = await batchStar(ids, true);
      else count = await batchStar(ids, false);
      invalidateMessageViews();
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
            aria-label={t("batch.selectAll", "Select all")}
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
              <BatchBtn icon={Star} label={t("batch.star", "Star")} onClick={() => handleBatchAction("star")} disabled={batchLoading} />
              <BatchBtn icon={Star} label={t("batch.unstar", "Unstar")} onClick={() => handleBatchAction("unstar")} disabled={batchLoading} />
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
                  spamFolderId={spamFolderId}
                  folderRole={activeFolder?.role}
                />
              </div>
            );
          })}
        </div>
        {onLoadMore && hasNextPage && (
          <div style={{ padding: "12px", textAlign: "center" }}>
            <button
              onClick={() => {
                if (!isFetchingNextPage) onLoadMore();
              }}
              disabled={isFetchingNextPage}
              style={{
                padding: "6px 20px",
                fontSize: "13px",
                border: "1px solid var(--color-border)",
                borderRadius: "6px",
                background: "transparent",
                color: "var(--color-text-secondary)",
                cursor: isFetchingNextPage ? "default" : "pointer",
                opacity: isFetchingNextPage ? 0.7 : 1,
              }}
            >
              {isFetchingNextPage ? t("common.loading", "Loading...") : t("common.loadMore", "Load more")}
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
      aria-label={label}
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
