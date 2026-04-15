import { useState } from "react";
import { useTranslation } from "react-i18next";
import AccountsTab from "./AccountsTab";
import GeneralTab from "./GeneralTab";
import AppearanceTab from "./AppearanceTab";
import CloudSyncTab from "./CloudSyncTab";
import RulesTab from "./RulesTab";
import ShortcutsTab from "./ShortcutsTab";
import TranslateTab from "./TranslateTab";
import PrivacyTab from "./PrivacyTab";
import AboutTab from "./AboutTab";

const TAB_IDS = ["accounts", "general", "appearance", "privacy", "rules", "translation", "shortcuts", "cloudSync", "about"] as const;

const TAB_LABEL_KEYS: Record<string, string> = {
  accounts: "settings.accounts",
  general: "settings.general",
  appearance: "settings.appearance",
  privacy: "settings.privacy",
  rules: "settings.rules",
  translation: "settings.translation",
  shortcuts: "settings.shortcuts",
  cloudSync: "settings.cloudSync",
  about: "settings.about",
};

export default function SettingsView() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState(
    () => (sessionStorage.getItem("pebble-settings-tab") as typeof TAB_IDS[number]) || "accounts",
  );

  function handleTabChange(id: typeof TAB_IDS[number]) {
    setActiveTab(id);
    sessionStorage.setItem("pebble-settings-tab", id);
  }

  return (
    <div style={{ display: "flex", height: "100%" }}>
      {/* Tab sidebar */}
      <div
        role="tablist"
        aria-orientation="vertical"
        aria-label={t("settings.tabs", "Settings tabs")}
        style={{
          width: "180px",
          borderRight: "1px solid var(--color-border)",
          padding: "16px 0",
          flexShrink: 0,
        }}
      >
        {TAB_IDS.map((id, index) => (
          <button
            key={id}
            id={`settings-tab-${id}`}
            role="tab"
            aria-selected={activeTab === id}
            aria-controls={`settings-tabpanel-${id}`}
            tabIndex={activeTab === id ? 0 : -1}
            onClick={() => handleTabChange(id)}
            onKeyDown={(e) => {
              let nextIndex = index;
              if (e.key === "ArrowDown") { nextIndex = (index + 1) % TAB_IDS.length; }
              else if (e.key === "ArrowUp") { nextIndex = (index - 1 + TAB_IDS.length) % TAB_IDS.length; }
              else if (e.key === "Home") { nextIndex = 0; }
              else if (e.key === "End") { nextIndex = TAB_IDS.length - 1; }
              else { return; }
              e.preventDefault();
              handleTabChange(TAB_IDS[nextIndex]);
              document.getElementById(`settings-tab-${TAB_IDS[nextIndex]}`)?.focus();
            }}
            style={{
              display: "block",
              width: "100%",
              textAlign: "left",
              padding: "8px 20px",
              border: "none",
              background: activeTab === id ? "var(--color-bg-hover)" : "none",
              color: activeTab === id ? "var(--color-text-primary)" : "var(--color-text-secondary)",
              fontWeight: activeTab === id ? 600 : 400,
              fontSize: "13px",
              cursor: "pointer",
              borderRight: activeTab === id ? "2px solid var(--color-accent)" : "2px solid transparent",
              transition: "all 0.15s ease",
            }}
          >
            {t(TAB_LABEL_KEYS[id])}
          </button>
        ))}
      </div>
      {/* Tab content */}
      <div
        id={`settings-tabpanel-${activeTab}`}
        role="tabpanel"
        aria-labelledby={`settings-tab-${activeTab}`}
        style={{ flex: 1, padding: "32px", maxWidth: "640px", overflow: "auto" }}
      >
        {activeTab === "accounts" && <AccountsTab />}
        {activeTab === "general" && <GeneralTab />}
        {activeTab === "appearance" && <AppearanceTab />}
        {activeTab === "rules" && <RulesTab />}
        {activeTab === "translation" && <TranslateTab />}
        {activeTab === "shortcuts" && <ShortcutsTab />}
        {activeTab === "privacy" && <PrivacyTab />}
        {activeTab === "cloudSync" && <CloudSyncTab />}
        {activeTab === "about" && <AboutTab />}
      </div>
    </div>
  );
}
