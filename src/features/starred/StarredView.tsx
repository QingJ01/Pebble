import { useCallback, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import { Star } from "lucide-react";
import type { MessageSummary } from "@/lib/api";
import { listStarredMessages } from "@/lib/api";
import { useMailStore } from "@/stores/mail.store";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import MessageItem from "@/components/MessageItem";
import MessageDetail from "@/components/MessageDetail";

export default function StarredView() {
  const { t } = useTranslation();
  const [messages, setMessages] = useState<MessageSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const activeAccountId = useMailStore((s) => s.activeAccountId);

  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76,
    measureElement: (el) => el.getBoundingClientRect().height,
    overscan: 5,
  });

  const loadStarred = useCallback(async () => {
    if (!activeAccountId) return;
    setLoading(true);
    setError(null);
    try {
      const starred = await listStarredMessages(activeAccountId, 100, 0);
      setMessages(starred);
    } catch (err) {
      setError(extractErrorMessage(err));
      setMessages([]);
    } finally {
      setLoading(false);
    }
  }, [activeAccountId]);

  useEffect(() => {
    loadStarred();
  }, [loadStarred]);

  function handleOpen(messageId: string) {
    setSelectedId(messageId);
  }

  if (loading) {
    return (
      <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%", color: "var(--color-text-secondary)" }}>
        <Star size={20} className="spinner" style={{ marginRight: "8px" }} />
        {t("common.loading", "Loading...")}
      </div>
    );
  }

  if (error) {
    return (
      <div className="fade-in" style={{
        display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
        height: "100%", gap: "12px", color: "var(--color-text-secondary)",
      }}>
        <Star size={40} strokeWidth={1.2} />
        <p style={{ color: "var(--color-error, #e53e3e)", fontSize: "14px", margin: 0 }}>
          {t("starred.loadError", "Failed to load starred messages")}
        </p>
        <p style={{ fontSize: "13px", margin: 0 }}>{error}</p>
        <button
          onClick={loadStarred}
          style={{
            marginTop: "4px",
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
    );
  }

  if (messages.length === 0) {
    return (
      <div className="fade-in" style={{
        display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
        height: "100%", gap: "12px", color: "var(--color-text-secondary)",
      }}>
        <Star size={40} strokeWidth={1.2} />
        <p style={{ fontSize: "14px", margin: 0 }}>{t("starred.empty", "No starred messages")}</p>
      </div>
    );
  }

  return (
    <div className="fade-in" style={{ display: "flex", height: "100%" }}>
      <div style={{
        width: selectedId ? "340px" : "100%",
        borderRight: selectedId ? "1px solid var(--color-border)" : "none",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
        transition: "width 0.15s ease",
      }}>
        <div style={{
          padding: "12px 16px", borderBottom: "1px solid var(--color-border)",
          fontSize: "14px", fontWeight: 600, color: "var(--color-text-primary)",
          display: "flex", alignItems: "center", gap: "8px",
        }}>
          <Star size={16} />
          {t("starred.title", "Starred Messages")}
          <span style={{
            fontSize: "12px", fontWeight: 400, color: "var(--color-text-secondary)",
            backgroundColor: "var(--color-bg-secondary, rgba(0,0,0,0.06))",
            padding: "2px 8px", borderRadius: "10px",
          }}>
            {messages.length}
          </span>
        </div>

        <div ref={parentRef} style={{ flex: 1, overflow: "auto" }}>
          <div style={{ height: `${virtualizer.getTotalSize()}px`, width: "100%", position: "relative" }}>
            {virtualizer.getVirtualItems().map((virtualItem) => {
              const msg = messages[virtualItem.index];
              return (
                <div
                  key={msg.id}
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
                    message={msg}
                    isSelected={msg.id === selectedId}
                    onClick={() => handleOpen(msg.id)}
                    onToggleStar={(id, newStarred) => {
                      if (!newStarred) {
                        setMessages((prev) => prev.filter((m) => m.id !== id));
                        if (selectedId === id) setSelectedId(null);
                      }
                    }}
                  />
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {selectedId && (
        <div style={{ flex: 1, overflow: "auto" }}>
          <MessageDetail
            messageId={selectedId}
            onBack={() => setSelectedId(null)}
          />
        </div>
      )}
    </div>
  );
}
