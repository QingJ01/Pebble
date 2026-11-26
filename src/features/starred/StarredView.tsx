import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MAIL_ROW_HEIGHT } from "@/components/mailRowMetrics";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import { Star } from "lucide-react";
import { useStarredMessagesQuery } from "@/hooks/queries";
import { useMailStore } from "@/stores/mail.store";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import MessageItem from "@/components/MessageItem";
import MessageDetail from "@/components/MessageDetail";
import Button from "@/components/ui/Button";
import { MessageListSkeleton } from "@/components/Skeleton";
import PaneSplitter, { usePaneWidth } from "@/components/PaneSplitter";
import { LOADING_INDICATOR_DELAY_MS } from "@/hooks/useDelayedVisible";
import { useRovingList } from "@/hooks/useRovingList";
import { useAccountBadges } from "@/hooks/useAccountBadges";
import "@/components/mail-list.css";

const EMPTY_REMOVED_IDS = new Set<string>();

export default function StarredView() {
  const { t } = useTranslation();
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const [stateAccountId, setStateAccountId] = useState(activeAccountId);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [removedIds, setRemovedIds] = useState<Set<string>>(() => new Set());
  const accountStateMatches = stateAccountId === activeAccountId;
  const scopedRemovedIds = accountStateMatches ? removedIds : EMPTY_REMOVED_IDS;
  const scopedSelectedId = accountStateMatches ? selectedId : null;
  const { data: messages, loading, error, hasNextPage, isFetchingNextPage, fetchNextPage, refetch } =
    useStarredMessagesQuery(activeAccountId, scopedRemovedIds.size);

  useEffect(() => {
    if (stateAccountId === activeAccountId) return;
    setStateAccountId(activeAccountId);
    setSelectedId(null);
    setRemovedIds(new Set());
  }, [activeAccountId, stateAccountId]);

  const visibleMessages = useMemo(() => {
    const seen = new Set<string>();
    return messages.filter((message) => {
      if (scopedRemovedIds.has(message.id) || seen.has(message.id)) return false;
      seen.add(message.id);
      return true;
    });
  }, [messages, scopedRemovedIds]);

  const parentRef = useRef<HTMLDivElement>(null);
  const listPane = usePaneWidth("pebble-starred-list-width", 340, 260, 560);
  const virtualizer = useVirtualizer({
    count: visibleMessages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => MAIL_ROW_HEIGHT,
    measureElement: (el) => el.getBoundingClientRect().height,
    overscan: 5,
  });

  function handleOpen(messageId: string) {
    setSelectedId(messageId);
  }

  const visibleIds = useMemo(() => visibleMessages.map((m) => m.id), [visibleMessages]);
  const visibleAccountIds = useMemo(() => visibleMessages.map((m) => m.account_id), [visibleMessages]);
  const badgeAccounts = useAccountBadges(visibleAccountIds);
  const virtualItems = virtualizer.getVirtualItems();
  const roving = useRovingList({
    ids: visibleIds,
    selectedId: scopedSelectedId,
    onSelect: setSelectedId,
    containerRef: parentRef,
    scrollToIndex: (index) => virtualizer.scrollToIndex(index, { align: "auto" }),
    renderedIds: virtualItems.map((item) => visibleIds[item.index]).filter(Boolean),
  });

  const handleLoadMore = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const handleToggleStar = useCallback((messageId: string, newStarred: boolean) => {
    setRemovedIds((prev) => {
      const next = new Set(prev);
      if (newStarred) {
        next.delete(messageId);
      } else {
        next.add(messageId);
      }
      return next;
    });
    if (!newStarred && scopedSelectedId === messageId) {
      setSelectedId(null);
    }
  }, [scopedSelectedId]);

  const totalCount = visibleMessages.length;

  if (error && messages.length === 0) {
    return (
      <div className="fade-in mail-empty-state">
        <Star size={40} strokeWidth={1.2} />
        <p role="alert" className="mail-empty-state__error">
          {t("starred.loadError", "Failed to load starred messages")}
        </p>
        <p className="mail-empty-state__detail">{extractErrorMessage(error)}</p>
        <Button onClick={() => refetch()}>
          {t("common.retry", "Retry")}
        </Button>
      </div>
    );
  }

  if (loading) {
    return <MessageListSkeleton delayMs={LOADING_INDICATOR_DELAY_MS} />;
  }

  if (visibleMessages.length === 0 && !hasNextPage) {
    return (
      <div className="fade-in mail-empty-state">
        <Star size={40} strokeWidth={1.2} />
        <p>{t("starred.empty", "No starred messages")}</p>
      </div>
    );
  }

  return (
    <div className="fade-in" style={{ display: "flex", height: "100%" }}>
      <div id="starred-list-pane" style={{
        width: scopedSelectedId ? `${listPane.width}px` : "100%",
        maxWidth: "55%",
        flexShrink: 0,
        borderRight: scopedSelectedId ? "1px solid var(--color-border)" : "none",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
      }}>
        <div className="pane-header">
          <Star size={16} aria-hidden="true" className="pane-header__icon" />
          {t("starred.title", "Starred Messages")}
          <span className="pane-header__count">{totalCount}</span>
        </div>

        <div
          ref={parentRef}
          className="scroll-region starred-list-scroll"
          role="listbox"
          aria-label={t("starred.title", "Starred Messages")}
          {...roving.containerProps}
          style={{ flex: 1, overflow: "auto", outline: "none" }}
        >
          <div role="presentation" style={{ height: `${virtualizer.getTotalSize()}px`, width: "100%", position: "relative" }}>
            {virtualItems.map((virtualItem) => {
              const msg = visibleMessages[virtualItem.index];
              return (
                <div
                  key={msg.id}
                  ref={virtualizer.measureElement}
                  data-index={virtualItem.index}
                  role="presentation"
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${virtualItem.start}px)`,
                  }}
                >
                  <MessageItem
                    message={msg}
                    isSelected={msg.id === scopedSelectedId}
                    onClick={() => {
                      roving.setActiveId(msg.id);
                      handleOpen(msg.id);
                    }}
                    onOpen={() => handleOpen(msg.id)}
                    onToggleStar={handleToggleStar}
                    tabIndex={roving.tabIndexFor(msg.id)}
                    onRowFocus={roving.setActiveId}
                    account={badgeAccounts?.get(msg.account_id)}
                  />
                </div>
              );
            })}
          </div>
        </div>

        {hasNextPage && (
          <div className="mail-list-load-more mail-list-load-more--bar">
            <Button size="sm" onClick={handleLoadMore} loading={isFetchingNextPage}>
              {t("common.loadMore", "Load More")}
            </Button>
          </div>
        )}
      </div>

      {scopedSelectedId && (
        <PaneSplitter
          width={listPane.width}
          min={listPane.min}
          max={listPane.max}
          onChange={listPane.setWidth}
          onReset={listPane.reset}
          controls="starred-list-pane"
        />
      )}
      {scopedSelectedId && (
        <div className="scroll-region starred-detail-scroll" style={{ flex: 1, minWidth: 0, overflow: "auto" }}>
          <MessageDetail
            messageId={scopedSelectedId}
            onBack={() => setSelectedId(null)}
            onReopen={(id) => setSelectedId((current) => current ?? id)}
          />
        </div>
      )}
    </div>
  );
}
