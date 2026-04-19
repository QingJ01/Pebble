import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useUIStore, type RealtimePreference } from "@/stores/ui.store";

const REALTIME_OPTIONS: Array<{
  mode: RealtimePreference;
  labelKey: string;
  fallback: string;
  descriptionKey: string;
  descriptionFallback: string;
}> = [
  {
    mode: "realtime",
    labelKey: "settings.realtimeModeRealtime",
    fallback: "Realtime (recommended)",
    descriptionKey: "settings.realtimeModeRealtimeDesc",
    descriptionFallback: "IMAP uses IDLE push when supported. Other providers check about every 3 seconds while you are active.",
  },
  {
    mode: "balanced",
    labelKey: "settings.realtimeModeBalanced",
    fallback: "Balanced",
    descriptionKey: "settings.realtimeModeBalancedDesc",
    descriptionFallback: "Checks about every 15 seconds while you are active.",
  },
  {
    mode: "battery",
    labelKey: "settings.realtimeModeBattery",
    fallback: "Battery saver",
    descriptionKey: "settings.realtimeModeBatteryDesc",
    descriptionFallback: "Checks about every 60 seconds while you are active and slows down in the background.",
  },
  {
    mode: "manual",
    labelKey: "settings.realtimeModeManual",
    fallback: "Manual only",
    descriptionKey: "settings.realtimeModeManualDesc",
    descriptionFallback: "Stops background checks. Use Sync now to run a single pass.",
  },
];

export default function GeneralTab() {
  const { t } = useTranslation();
  const realtimeMode = useUIStore((s) => s.realtimeMode);
  const setRealtimeMode = useUIStore((s) => s.setRealtimeMode);
  const notificationsEnabled = useUIStore((s) => s.notificationsEnabled);
  const setNotificationsEnabled = useUIStore((s) => s.setNotificationsEnabled);

  const toggleNotifications = useCallback(() => {
    setNotificationsEnabled(!notificationsEnabled);
  }, [notificationsEnabled, setNotificationsEnabled]);

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
          const label = t(option.labelKey, option.fallback);
          return (
            <button
              key={option.mode}
              type="button"
              aria-label={label}
              aria-pressed={selected}
              onClick={() => setRealtimeMode(option.mode)}
              style={{
                flex: "1 1 180px",
                minWidth: 0,
                padding: "8px 10px",
                borderRadius: "6px",
                border: selected ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
                backgroundColor: selected ? "var(--color-bg-hover)" : "transparent",
                cursor: "pointer",
                textAlign: "left",
                color: "var(--color-text-primary)",
              }}
            >
              <span style={{ display: "block", fontSize: "13px", fontWeight: selected ? 600 : 500, lineHeight: 1.3 }}>
                {label}
              </span>
              <span style={{ display: "block", marginTop: "4px", fontSize: "12px", lineHeight: 1.35, color: "var(--color-text-secondary)" }}>
                {t(option.descriptionKey, option.descriptionFallback)}
              </span>
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
