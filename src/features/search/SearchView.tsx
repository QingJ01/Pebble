import { useState, useCallback, useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import { Search, SlidersHorizontal, Loader } from "lucide-react";
import type { AdvancedSearchQuery, SearchHit } from "@/lib/api";
import { advancedSearch, searchMessages } from "@/lib/api";
import { useUIStore } from "@/stores/ui.store";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import SearchFilters from "./SearchFilters";
import SearchResultItem from "./SearchResultItem";
import MessageDetail from "@/components/MessageDetail";

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
  const [results, setResults] = useState<SearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hasSearched, setHasSearched] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const initialQueryRef = useRef<string | null>(null);
  const requestIdRef = useRef(0);

  const resultsParentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: results.length,
    getScrollElement: () => resultsParentRef.current,
    estimateSize: () => 76,
    measureElement: (el) => el.getBoundingClientRect().height,
    overscan: 5,
  });

  const doSearch = useCallback(async () => {
    const trimmed = query.trim();
    const filtersActive = hasActiveFilters(filters);

    if (!trimmed && !filtersActive) {
      requestIdRef.current += 1; // invalidate any in-flight response
      setResults([]);
      setHasSearched(false);
      setLoading(false);
      return;
    }

    const myId = ++requestIdRef.current;
    setLoading(true);
    setError(null);
    setHasSearched(true);
    try {
      let hits: SearchHit[];
      if (filtersActive) {
        hits = await advancedSearch({ ...filters, text: trimmed || undefined });
      } else {
        hits = await searchMessages(trimmed);
      }
      // Ignore stale responses from superseded requests
      if (myId !== requestIdRef.current) return;
      setResults(hits);
      setSelectedId(null);
    } catch (err) {
      if (myId === requestIdRef.current) {
        setError(extractErrorMessage(err));
        setResults([]);
      }
    } finally {
      if (myId === requestIdRef.current) {
        setLoading(false);
      }
    }
  }, [query, filters]);

  // Pick up query from store on every mount (works across navigations)
  useEffect(() => {
    const storeQuery = useUIStore.getState().searchQuery;
    if (storeQuery) {
      setQuery(storeQuery);
      useUIStore.getState().setSearchQuery("");
      initialQueryRef.current = storeQuery;
    }
  }, []);

  // Search after mount if we had a store query
  useEffect(() => {
    if (initialQueryRef.current) {
      initialQueryRef.current = null;
      doSearch();
    }
  }, [doSearch]);

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
      {/* Search header */}
      <form
        onSubmit={handleSubmit}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "8px",
          padding: "10px 14px",
          borderBottom: "1px solid var(--color-border)",
          backgroundColor: "var(--color-bg)",
        }}
      >
        <Search size={16} color="var(--color-text-secondary)" style={{ flexShrink: 0 }} />
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("inbox.searchPlaceholder")}
          aria-label={t("search.title", "Search")}
          autoFocus
          style={{
            flex: 1,
            border: "none",
            backgroundColor: "transparent",
            fontSize: "14px",
            color: "var(--color-text-primary)",
          }}
        />
        <button
          type="submit"
          style={{
            background: "none",
            border: "none",
            cursor: "pointer",
            padding: "4px 8px",
            color: "var(--color-accent)",
            display: "flex",
            alignItems: "center",
            fontSize: "13px",
            fontWeight: 500,
          }}
        >
          {t("search.searchButton")}
        </button>
        <button
          type="button"
          onClick={() => setShowFilters(!showFilters)}
          title={t("search.filters")}
          aria-label={t("search.filters")}
          style={{
            background: "none",
            border: "none",
            cursor: "pointer",
            padding: "4px",
            color: showFilters || hasActiveFilters(filters)
              ? "var(--color-accent)"
              : "var(--color-text-secondary)",
            display: "flex",
            alignItems: "center",
          }}
        >
          <SlidersHorizontal size={16} />
        </button>
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
          style={{
            width: selectedId ? "360px" : "100%",
            flexShrink: 0,
            overflow: "auto",
            borderRight: selectedId ? "1px solid var(--color-border)" : "none",
            transition: "width 0.15s ease",
          }}
        >
          {loading && (
            <div
              className="fade-in"
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                padding: "40px",
                color: "var(--color-text-secondary)",
                fontSize: "13px",
                gap: "10px",
              }}
            >
              <Loader size={20} className="spinner" />
              <span>{t("common.loading")}</span>
            </div>
          )}

          {!loading && error && (
            <div
              className="fade-in"
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                padding: "40px",
                color: "var(--color-text-secondary)",
                fontSize: "14px",
                gap: "8px",
              }}
            >
              <p style={{ color: "var(--color-error, #e53e3e)", margin: 0 }}>
                {t("search.error", "Search failed")}
              </p>
              <p style={{ fontSize: "13px", margin: 0 }}>{error}</p>
              <button
                onClick={doSearch}
                style={{
                  marginTop: "8px",
                  padding: "6px 16px",
                  borderRadius: "4px",
                  border: "1px solid var(--color-border)",
                  backgroundColor: "transparent",
                  color: "var(--color-accent)",
                  fontSize: "13px",
                  cursor: "pointer",
                }}
              >
                {t("common.retry", "Retry")}
              </button>
            </div>
          )}

          {!loading && !error && hasSearched && results.length === 0 && (
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                padding: "40px",
                color: "var(--color-text-secondary)",
                fontSize: "14px",
                gap: "8px",
              }}
            >
              <Search size={28} strokeWidth={1.2} />
              {t("search.noResults")}
            </div>
          )}

          {!loading && !hasSearched && (
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                padding: "60px 20px",
                color: "var(--color-text-tertiary)",
                fontSize: "14px",
                gap: "8px",
              }}
            >
              <Search size={32} />
              <span>{t("search.title")}</span>
            </div>
          )}

          {!loading && results.length > 0 && (
            <div style={{ height: `${virtualizer.getTotalSize()}px`, width: "100%", position: "relative" }}>
              {virtualizer.getVirtualItems().map((virtualItem) => {
                const hit = results[virtualItem.index];
                return (
                  <div
                    key={hit.message_id}
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
                    <SearchResultItem
                      hit={hit}
                      isSelected={hit.message_id === selectedId}
                      onClick={() => setSelectedId(hit.message_id)}
                      query={query}
                    />
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Detail panel */}
        {selectedId && (
          <div style={{ flex: 1, overflow: "hidden" }}>
            <MessageDetail
              messageId={selectedId}
              onBack={() => setSelectedId(null)}
            />
          </div>
        )}
      </div>
    </div>
  );
}
