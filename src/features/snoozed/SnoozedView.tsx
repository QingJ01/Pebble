import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Clock, Bell } from "lucide-react";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import type { SnoozedMessage, Message } from "@/lib/api";
import { listSnoozed, unsnoozeMessage, getMessagesBatch } from "@/lib/api";
import { useMailStore } from "@/stores/mail.store";
import { useUIStore } from "@/stores/ui.store";
import { useToastStore } from "@/stores/toast.store";

interface SnoozedEntry {
  snooze: SnoozedMessage;
  message: Message | null;
}

export default function SnoozedView() {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<SnoozedEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadSnoozed();
  }, []);

  async function loadSnoozed() {
    setLoading(true);
    setError(null);
    try {
      const snoozed = await listSnoozed();
      const ids = snoozed.map((s) => s.message_id);
      const messages = ids.length > 0 ? await getMessagesBatch(ids) : [];
      const messageMap = new Map(messages.map((m) => [m.id, m]));
      const withMessages = snoozed.map((s) => ({
        snooze: s,
        message: messageMap.get(s.message_id) ?? null,
      }));
      setEntries(withMessages);
    } catch (err) {
      setError(extractErrorMessage(err));
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }

  async function handleUnsnooze(messageId: string) {
    try {
      await unsnoozeMessage(messageId);
      setEntries((prev) => prev.filter((e) => e.snooze.message_id !== messageId));
    } catch (err) {
      console.error("Failed to unsnooze:", err);
      useToastStore.getState().addToast({ message: t("snoozed.unsnoozeFailed", "Failed to unsnooze message"), type: "error" });
    }
  }

  function handleOpen(messageId: string) {
    useMailStore.getState().setSelectedMessage(messageId);
    useUIStore.getState().setActiveView("inbox");
  }

  function formatSnoozeTime(timestamp: number): string {
    const date = new Date(timestamp * 1000);
    return date.toLocaleString();
  }

  if (loading) {
    return (
      <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%", color: "var(--color-text-secondary)" }}>
        <Clock size={20} className="spin" style={{ marginRight: "8px" }} />
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
        <Clock size={40} strokeWidth={1.2} />
        <p style={{ color: "var(--color-error, #e53e3e)", fontSize: "14px", margin: 0 }}>
          {t("snoozed.loadError", "Failed to load snoozed messages")}
        </p>
        <p style={{ fontSize: "13px", margin: 0 }}>{error}</p>
        <button
          onClick={loadSnoozed}
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

  if (entries.length === 0) {
    return (
      <div className="fade-in" style={{
        display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
        height: "100%", gap: "12px", color: "var(--color-text-secondary)",
      }}>
        <Clock size={40} strokeWidth={1.2} />
        <p style={{ fontSize: "14px", margin: 0 }}>{t("snoozed.empty", "No snoozed messages")}</p>
      </div>
    );
  }

  return (
    <div className="fade-in" style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{
        padding: "12px 16px", borderBottom: "1px solid var(--color-border)",
        fontSize: "14px", fontWeight: 600, color: "var(--color-text-primary)",
        display: "flex", alignItems: "center", gap: "8px",
      }}>
        <Clock size={16} />
        {t("snoozed.title", "Snoozed Messages")}
        <span style={{
          fontSize: "12px", fontWeight: 400, color: "var(--color-text-secondary)",
          backgroundColor: "var(--color-bg-secondary, rgba(0,0,0,0.06))",
          padding: "2px 8px", borderRadius: "10px",
        }}>
          {entries.length}
        </span>
      </div>

      <div style={{ flex: 1, overflowY: "auto" }}>
        {entries.map((entry) => (
          <div
            key={entry.snooze.message_id}
            style={{
              padding: "12px 16px",
              borderBottom: "1px solid var(--color-border)",
              display: "flex",
              alignItems: "center",
              gap: "12px",
              cursor: "pointer",
              transition: "background-color 0.1s ease",
            }}
            onClick={() => handleOpen(entry.snooze.message_id)}
            onMouseEnter={(e) => { e.currentTarget.style.backgroundColor = "var(--color-bg-hover, rgba(0,0,0,0.03))"; }}
            onMouseLeave={(e) => { e.currentTarget.style.backgroundColor = "transparent"; }}
          >
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{
                fontSize: "13px", fontWeight: 500, color: "var(--color-text-primary)",
                overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
              }}>
                {entry.message?.subject || entry.snooze.message_id}
              </div>
              <div style={{ fontSize: "12px", color: "var(--color-text-secondary)", marginTop: "2px" }}>
                {entry.message?.from_address || ""}
              </div>
              <div style={{ fontSize: "11px", color: "var(--color-text-secondary)", marginTop: "4px", display: "flex", alignItems: "center", gap: "4px" }}>
                <Clock size={12} />
                {t("snoozed.wakeAt", "Wake at")} {formatSnoozeTime(entry.snooze.unsnoozed_at)}
              </div>
            </div>

            <button
              onClick={(e) => {
                e.stopPropagation();
                handleUnsnooze(entry.snooze.message_id);
              }}
              title={t("snoozed.unsnooze", "Unsnooze")}
              style={{
                display: "flex", alignItems: "center", gap: "4px",
                padding: "4px 10px", borderRadius: "4px",
                border: "1px solid var(--color-border)",
                backgroundColor: "transparent",
                color: "var(--color-text-secondary)",
                fontSize: "12px", cursor: "pointer",
                transition: "background-color 0.1s ease",
              }}
              onMouseEnter={(e) => { e.currentTarget.style.backgroundColor = "var(--color-bg-hover, rgba(0,0,0,0.06))"; }}
              onMouseLeave={(e) => { e.currentTarget.style.backgroundColor = "transparent"; }}
            >
              <Bell size={13} />
              {t("snoozed.unsnooze", "Unsnooze")}
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
