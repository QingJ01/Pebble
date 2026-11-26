import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { MAIL_ROW_HEIGHT } from "@/components/mailRowMetrics";
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import { Search, SlidersHorizontal } from "lucide-react";
import type { AdvancedSearchQuery, SearchHit } from "@/lib/api";
import { advancedSearch, searchMessages } from "@/lib/api";
import { useUIStore } from "@/stores/ui.store";
import SearchFilters from "./SearchFilters";
import SearchResultItem from "./SearchResultItem";
import MessageDetail from "@/components/MessageDetail";
import Button from "@/components/ui/Button";
import IconButton from "@/components/ui/IconButton";
import "@/components/mail-list.css";
import Spinner from "@/components/Spinner";
import PaneSplitter, { usePaneWidth } from "@/components/PaneSplitter";
import { useRovingList } from "@/hooks/useRovingList";
import { useAccountBadges } from "@/hooks/useAccountBadges";
import { LOADING_INDICATOR_DELAY_MS } from "@/hooks/useDelayedVisible";
import { extractErrorMessage } from "@/lib/extractErrorMessage";

const emptyFilters: AdvancedSearchQuery = {};

function hasActiveFilters(filters: AdvancedSearchQuery): boolean {
  return !!(
    filters.from ||
    filters.to ||
    filters.subject ||
    filters.dateFrom ||
    filters.dateTo ||
    filters.hasAttachment ||
    filters.folderId
  );
}

export default function SearchView() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [filters, setFilters] = useState<AdvancedSearchQuery>(emptyFilters);
  const [showFilters, setShowFilters] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hasSearched, setHasSearched] = useState(false);
  const storeSearchQuery = useUIStore((s) => s.searchQuery);

  const trimmed = query.trim();
  const filtersActive = hasActiveFilters(filters);
  const searchEnabled = hasSearched && (trimmed.length > 0 || filtersActive);

  const { data: results = [], isLoading: loading, error: queryError, refetch } = useQuery({
    queryKey: ["search", trimmed, filters],
    queryFn: () => {
      if (filtersActive) {
        return advancedSearch({ ...filters, text: trimmed || undefined });
      }
      return searchMessages(trimmed);
    },
    enabled: searchEnabled,
    staleTime: 60_000,
    placeholderData: (prev: SearchHit[] | undefined) => prev,
  });

  const resultsParentRef = useRef<HTMLDivElement>(null);
  const listPane = usePaneWidth("pebble-search-list-width", 340, 260, 560);
  const virtualizer = useVirtualizer({
    count: results.length,
    getScrollElement: () => resultsParentRef.current,
    estimateSize: () => MAIL_ROW_HEIGHT,
    measureElement: (el) => el.getBoundingClientRect().height,
    overscan: 5,
  });

  const resultIds = useMemo(() => results.map((hit) => hit.message_id), [results]);
  const resultAccountIds = useMemo(() => results.map((hit) => hit.account_id), [results]);
  const badgeAccounts = useAccountBadges(resultAccountIds);
  const virtualItems = virtualizer.getVirtualItems();
  const roving = useRovingList({
    ids: resultIds,
    selectedId,
    onSelect: setSelectedId,
    containerRef: resultsParentRef,
    scrollToIndex: (index) => virtualizer.scrollToIndex(index, { align: "auto" }),
    renderedIds: virtualItems.map((item) => resultIds[item.index]).filter(Boolean),
  });

  const doSearch = useCallback(() => {
    const t = query.trim();
    if (!t && !hasActiveFilters(filters)) {
      setHasSearched(false);
      setSelectedId(null);
      return;
    }
    setHasSearched(true);
    setSelectedId(null);
  }, [query, filters]);

  // Pick up context queries from other views and from this view while mounted.
  useEffect(() => {
    if (!storeSearchQuery) return;
    setQuery(storeSearchQuery);
    setSelectedId(null);
    setHasSearched(true);
    useUIStore.getState().setSearchQuery("");
  }, [storeSearchQuery]);

  // Auto-search when filters change
  useEffect(() => {
    if (hasActiveFilters(filters) || query.trim()) {
      doSearch();
    }
  }, [filters]); // eslint-disable-line react-hooks/exhaustive-deps

  // Debounced type-ahead search (300ms after last keystroke)
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    const trimmed = query.trim();
    if (!trimmed && !hasActiveFilters(filters)) return;
    debounceRef.current = setTimeout(() => {
      doSearch();
    }, 300);
    return () => { if (debounceRef.current) clearTimeout(debounceRef.current); };
  }, [query]); // eslint-disable-line react-hooks/exhaustive-deps

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (debounceRef.current) clearTimeout(debounceRef.current);
    doSearch();
  }

  function handleClearFilters() {
    setFilters(emptyFilters);
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden" }}>
      <form onSubmit={handleSubmit} className="search-toolbar">
        <div className="search-input-shell">
          <Search size={16} aria-hidden="true" />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("inbox.searchPlaceholder")}
            aria-label={t("search.title", "Search")}
            autoFocus
            className="search-input"
          />
        </div>
        <Button type="submit" icon={<Search size={14} />}>
          {t("search.searchButton")}
        </Button>
        <IconButton
          size="md"
          label={t("search.filters")}
          icon={<SlidersHorizontal size={16} />}
          aria-expanded={showFilters}
          className={showFilters || filtersActive ? "is-active" : undefined}
          onClick={() => setShowFilters(!showFilters)}
        />
      </form>

      {/* Filters panel */}
      {showFilters && (
        <SearchFilters
          filters={filters}
          onChange={setFilters}
          onClear={handleClearFilters}
        />
      )}

      {/* Results + Detail split layout */}
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        {/* Results list */}
        <div
          ref={resultsParentRef}
          id="search-results-pane"
          className="scroll-region search-results-scroll"
          style={{
            width: selectedId ? `${listPane.width}px` : "100%",
            maxWidth: "55%",
            flexShrink: 0,
            overflow: "auto",
            borderRight: selectedId ? "1px solid var(--color-border)" : "none",
          }}
        >
          {loading && (
            <Spinner delayMs={LOADING_INDICATOR_DELAY_MS} label={t("common.loading")} />
          )}

          {!loading && queryError && (
            <div className="fade-in mail-empty-state">
              <p role="alert" className="mail-empty-state__error">
                {t("search.error", "Search failed")}
              </p>
              <p className="mail-empty-state__detail">{extractErrorMessage(queryError)}</p>
              <Button onClick={() => refetch()}>
                {t("common.retry", "Retry")}
              </Button>
            </div>
          )}

          {!loading && !queryError && hasSearched && results.length === 0 && (
            <div className="mail-empty-state">
              <Search size={28} strokeWidth={1.2} />
              {t("search.noResults")}
            </div>
          )}

          {!loading && !hasSearched && (
            <div className="mail-empty-state mail-empty-state--muted">
              <Search size={32} strokeWidth={1.2} />
              <span>{t("search.title")}</span>
            </div>
          )}

          {!loading && results.length > 0 && (
            <div
              role="listbox"
              aria-label={t("search.results", "Search results")}
              {...roving.containerProps}
              style={{ height: `${virtualizer.getTotalSize()}px`, width: "100%", position: "relative", outline: "none" }}
            >
              {virtualItems.map((virtualItem) => {
                const hit = results[virtualItem.index];
                return (
                  <div
                    key={hit.message_id}
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
                    <SearchResultItem
                      hit={hit}
                      isSelected={hit.message_id === selectedId}
                      onClick={() => {
                        roving.setActiveId(hit.message_id);
                        setSelectedId(hit.message_id);
                      }}
                      query={query}
                      tabIndex={roving.tabIndexFor(hit.message_id)}
                      onRowFocus={roving.setActiveId}
                      account={hit.account_id ? badgeAccounts?.get(hit.account_id) : undefined}
                    />
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Detail panel */}
        {selectedId && (
          <PaneSplitter
            width={listPane.width}
            min={listPane.min}
            max={listPane.max}
            onChange={listPane.setWidth}
            onReset={listPane.reset}
            controls="search-results-pane"
          />
        )}
        {selectedId && (
          <div style={{ flex: 1, minWidth: 0, overflow: "hidden" }}>
            <MessageDetail
              messageId={selectedId}
              onBack={() => setSelectedId(null)}
              onReopen={(id) => setSelectedId((current) => current ?? id)}
            />
          </div>
        )}
      </div>
    </div>
  );
}
