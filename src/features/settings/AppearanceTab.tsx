import { useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "@/stores/ui.store";
import type { Theme, Language } from "@/stores/ui.store";

const NOTIFICATIONS_KEY = "pebble-notifications-enabled";

const THEMES: { id: Theme; labelKey: string; descKey: string }[] = [
  { id: "light", labelKey: "settings.themeLight", descKey: "settings.themeLightDesc" },
  { id: "dark", labelKey: "settings.themeDark", descKey: "settings.themeDarkDesc" },
  { id: "system", labelKey: "settings.themeSystem", descKey: "settings.themeSystemDesc" },
];

const LANGUAGES: { id: Language; label: string }[] = [
  { id: "en", label: "English" },
  { id: "zh", label: "中文" },
];

export default function AppearanceTab() {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);
  const language = useUIStore((s) => s.language);
  const setLanguage = useUIStore((s) => s.setLanguage);

  const [notificationsEnabled, setNotificationsEnabled] = useState(() => {
    return localStorage.getItem(NOTIFICATIONS_KEY) !== "false";
  });

  const toggleNotifications = useCallback(() => {
    setNotificationsEnabled((prev) => {
      const next = !prev;
      localStorage.setItem(NOTIFICATIONS_KEY, String(next));
      return next;
    });
  }, []);

  return (
    <div>
      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "16px" }}>{t("settings.theme")}</h3>
      <div style={{ display: "flex", gap: "12px" }}>
        {THEMES.map((th) => (
          <button
            key={th.id}
            onClick={() => setTheme(th.id)}
            style={{
              flex: 1,
              padding: "16px",
              borderRadius: "8px",
              border: theme === th.id ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
              backgroundColor: theme === th.id ? "var(--color-bg-hover)" : "transparent",
              cursor: "pointer",
              textAlign: "left",
              color: "var(--color-text-primary)",
            }}
          >
            <div style={{ fontWeight: 600, fontSize: "13px", marginBottom: "4px" }}>{t(th.labelKey)}</div>
            <div style={{ fontSize: "12px", color: "var(--color-text-secondary)" }}>{t(th.descKey)}</div>
          </button>
        ))}
      </div>

      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "16px", marginTop: "32px" }}>
        {t("settings.language")}
      </h3>
      <div style={{ display: "flex", gap: "12px" }}>
        {LANGUAGES.map((l) => (
          <button
            key={l.id}
            onClick={() => setLanguage(l.id)}
            style={{
              flex: 1,
              padding: "16px",
              borderRadius: "8px",
              border: language === l.id ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
              backgroundColor: language === l.id ? "var(--color-bg-hover)" : "transparent",
              cursor: "pointer",
              textAlign: "left",
              color: "var(--color-text-primary)",
            }}
          >
            <div style={{ fontWeight: 600, fontSize: "13px" }}>{l.label}</div>
          </button>
        ))}
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
    </div>
  );
}
