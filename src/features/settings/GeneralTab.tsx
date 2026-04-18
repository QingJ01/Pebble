import { useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { useUIStore, type RealtimePreference } from "@/stores/ui.store";

const NOTIFICATIONS_KEY = "pebble-notifications-enabled";
const REALTIME_OPTIONS: Array<{
  mode: RealtimePreference;
  labelKey: string;
  fallback: string;
}> = [
  { mode: "realtime", labelKey: "settings.realtimeModeRealtime", fallback: "Realtime (recommended)" },
  { mode: "balanced", labelKey: "settings.realtimeModeBalanced", fallback: "Balanced" },
  { mode: "battery", labelKey: "settings.realtimeModeBattery", fallback: "Battery saver" },
  { mode: "manual", labelKey: "settings.realtimeModeManual", fallback: "Manual only" },
];

export default function GeneralTab() {
  const { t } = useTranslation();
  const realtimeMode = useUIStore((s) => s.realtimeMode);
  const setRealtimeMode = useUIStore((s) => s.setRealtimeMode);

  const [notificationsEnabled, setNotificationsEnabled] = useState(() => {
    return localStorage.getItem(NOTIFICATIONS_KEY) === "true";
  });

  const toggleNotifications = useCallback(() => {
    setNotificationsEnabled((prev) => {
      const next = !prev;
      localStorage.setItem(NOTIFICATIONS_KEY, String(next));
      invoke("set_notifications_enabled", { enabled: next }).catch((err) => console.warn("Failed to update notifications setting in backend", err));
      return next;
    });
  }, []);

  const showUnreadCount = useUIStore((s) => s.showFolderUnreadCount);
  const setShowUnreadCount = useUIStore((s) => s.setShowFolderUnreadCount);

  const toggleUnreadCount = useCallback(() => {
    setShowUnreadCount(!showUnreadCount);
  }, [showUnreadCount, setShowUnreadCount]);

  return (
    <div>
      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "8px" }}>
        {t("settings.realtimeMode", "Realtime Mode")}
      </h3>
      <p style={{ fontSize: "12px", color: "var(--color-text-secondary)", marginBottom: "12px", marginTop: 0 }}>
        {t("settings.realtimeModeDesc", "Choose how aggressively Pebble checks for new mail.")}
      </p>
      <div
        role="group"
        aria-label={t("settings.realtimeMode", "Realtime Mode")}
        style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}
      >
        {REALTIME_OPTIONS.map((option) => {
          const selected = realtimeMode === option.mode;
          return (
            <button
              key={option.mode}
              type="button"
              aria-pressed={selected}
              onClick={() => setRealtimeMode(option.mode)}
              style={{
                padding: "8px 16px",
                borderRadius: "6px",
                border: selected ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
                backgroundColor: selected ? "var(--color-bg-hover)" : "transparent",
                cursor: "pointer",
                fontSize: "13px",
                fontWeight: selected ? 600 : 400,
                color: "var(--color-text-primary)",
              }}
            >
              {t(option.labelKey, option.fallback)}
            </button>
          );
        })}
      </div>

      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "16px", marginTop: "32px" }}>
        {t("settings.notifications")}
      </h3>
      <label
        style={{
          display: "flex",
          alignItems: "center",
          gap: "8px",
          cursor: "pointer",
          fontSize: "13px",
          color: "var(--color-text-primary)",
        }}
      >
        <input type="checkbox" checked={notificationsEnabled} onChange={toggleNotifications} />
        <span>{t("settings.enableNotifications")}</span>
      </label>

      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "16px", marginTop: "32px" }}>
        {t("settings.folderCounts", "Folder Counts")}
      </h3>
      <label
        style={{
          display: "flex",
          alignItems: "center",
          gap: "8px",
          cursor: "pointer",
          fontSize: "13px",
          color: "var(--color-text-primary)",
        }}
      >
        <input
          type="checkbox"
          checked={showUnreadCount}
          onChange={toggleUnreadCount}
        />
        <span>{t("settings.showUnreadCount", "Show unread count badges in sidebar")}</span>
      </label>
    </div>
  );
}
