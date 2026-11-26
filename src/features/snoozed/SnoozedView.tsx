import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Clock, Bell } from "lucide-react";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import { formatDateTime } from "@/lib/formatDate";
import type { SnoozedMessage, Message } from "@/lib/api";
import { listSnoozed, unsnoozeMessage, getMessagesBatch } from "@/lib/api";
import { useUIStore } from "@/stores/ui.store";
import { toastError } from "@/stores/toast.store";
import Button from "@/components/ui/Button";
import Spinner from "@/components/Spinner";
import { LOADING_INDICATOR_DELAY_MS } from "@/hooks/useDelayedVisible";
import "@/components/mail-list.css";

interface SnoozedEntry {
  snooze: SnoozedMessage;
  message: Message | null;
}

export default function SnoozedView() {
  const { t, i18n } = useTranslation();
  const [entries, setEntries] = useState<SnoozedEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const openMessageInInbox = useUIStore((s) => s.openMessageInInbox);

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
      toastError(err, "snoozed.unsnoozeFailed");
    }
  }

  function handleOpen(messageId: string) {
    openMessageInInbox(messageId);
  }

  function formatSnoozeTime(timestamp: number): string {
    return formatDateTime(timestamp, undefined, i18n?.language);
  }

  if (loading) {
    return <Spinner delayMs={LOADING_INDICATOR_DELAY_MS} label={t("common.loading", "Loading...")} />;
  }

  if (error) {
    return (
      <div className="fade-in mail-empty-state">
        <Clock size={40} strokeWidth={1.2} />
        <p role="alert" className="mail-empty-state__error">
          {t("snoozed.loadError", "Failed to load snoozed messages")}
        </p>
        <p className="mail-empty-state__detail">{error}</p>
        <Button onClick={() => void loadSnoozed()}>
          {t("common.retry", "Retry")}
        </Button>
      </div>
    );
  }

  if (entries.length === 0) {
    return (
      <div className="fade-in mail-empty-state">
        <Clock size={40} strokeWidth={1.2} />
        <p>{t("snoozed.empty", "No snoozed messages")}</p>
      </div>
    );
  }

  return (
    <div className="fade-in" style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="pane-header">
        <Clock size={16} aria-hidden="true" className="pane-header__icon" />
        {t("snoozed.title", "Snoozed Messages")}
        <span className="pane-header__count">{entries.length}</span>
      </div>

      <div className="scroll-region snoozed-list-scroll" style={{ flex: 1, overflowY: "auto" }}>
        {entries.map((entry) => (
          <div
            key={entry.snooze.message_id}
            role="button"
            tabIndex={0}
            className="list-row snoozed-row"
            onClick={() => handleOpen(entry.snooze.message_id)}
            onKeyDown={(e) => {
              if (e.target !== e.currentTarget) return;
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                handleOpen(entry.snooze.message_id);
              }
            }}
          >
            <div className="snoozed-row__text">
              <div className="snoozed-row__subject">
                {entry.message?.subject || entry.snooze.message_id}
              </div>
              <div className="snoozed-row__from">
                {entry.message?.from_address || ""}
              </div>
              <div className="snoozed-row__wake">
                <Clock size={12} aria-hidden="true" />
                {t("snoozed.wakeAt", "Wake at")} {formatSnoozeTime(entry.snooze.unsnoozed_at)}
              </div>
            </div>

            <Button
              size="sm"
              icon={<Bell size={14} />}
              onClick={(e) => {
                e.stopPropagation();
                void handleUnsnooze(entry.snooze.message_id);
              }}
              onKeyDown={(e) => e.stopPropagation()}
            >
              {t("snoozed.unsnooze", "Unsnooze")}
            </Button>
          </div>
        ))}
      </div>
    </div>
  );
}
