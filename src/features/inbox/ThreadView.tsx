import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, MessageSquare } from "lucide-react";
import { isThreadSelected, useMailStore } from "@/stores/mail.store";
import { useAccountsQuery, useFoldersForAccountsQuery, useThreadMessagesQuery, useThreadsQuery } from "@/hooks/queries";
import { folderIdsForSelection } from "@/lib/folderAggregation";
import ThreadMessageBubble from "@/components/ThreadMessageBubble";
import { MessageDetailSkeleton } from "@/components/Skeleton";
import IconButton from "@/components/ui/IconButton";
import { useDelayedVisible } from "@/hooks/useDelayedVisible";
import "@/components/message-reader.css";

export default function ThreadView() {
  const { t } = useTranslation();
  const selectedThreadId = useMailStore((s) => s.selectedThreadId);
  const selectedThreadAccountId = useMailStore((s) => s.selectedThreadAccountId);
  const setSelectedThreadId = useMailStore((s) => s.setSelectedThreadId);
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const { data: accounts = [] } = useAccountsQuery();
  const folderAccountIds = useMemo(
    () => activeAccountId ? [activeAccountId] : accounts.map((account) => account.id),
    [accounts, activeAccountId],
  );
  const { data: folders = [] } = useFoldersForAccountsQuery(folderAccountIds);
  const selectedFolderIds = folderIdsForSelection(activeFolderId, folders);
  const queryFolderId = selectedFolderIds[0] ?? null;
  const queryFolderIds = selectedFolderIds.length > 1 ? selectedFolderIds : undefined;
  const { data: threads = [] } = useThreadsQuery(queryFolderId, 50, 0, queryFolderIds);
  const scrollRef = useRef<HTMLDivElement>(null);
  const showLoadingSkeleton = useDelayedVisible();

  // Selection is keyed by (account_id, thread_id): the selected row's account
  // scopes the fetch so another account's thread with the same id is never
  // mixed in.
  const thread = threads.find((t) => isThreadSelected({ selectedThreadId, selectedThreadAccountId }, t));
  const threadAccountId = selectedThreadAccountId ?? thread?.account_id ?? activeAccountId ?? null;
  const { data: threadMessages = [], isLoading } = useThreadMessagesQuery(selectedThreadId, threadAccountId);

  useEffect(() => {
    if (scrollRef.current && threadMessages.length > 0) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [threadMessages]);

  if (isLoading) {
    return showLoadingSkeleton ? <MessageDetailSkeleton /> : <div role="status" aria-busy="true" className="thread-reader" />;
  }

  return (
    <div className="thread-reader">
      {/* Header: same 52px toolbar row as the message reader */}
      <div className="reader-toolbar">
        <IconButton
          size="md"
          onClick={() => setSelectedThreadId(null)}
          label={t("thread.back", "Back")}
          icon={<ArrowLeft size={16} />}
        />
        <h2 className="thread-reader__title" title={thread?.subject || undefined}>
          {thread?.subject || t("thread.title")}
        </h2>
        <div className="reader-toolbar__count">
          <MessageSquare size={14} aria-hidden="true" />
          <span>{threadMessages.length}</span>
        </div>
      </div>

      {/* Messages */}
      <div ref={scrollRef} className="scroll-region thread-message-scroll thread-reader__messages">
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
