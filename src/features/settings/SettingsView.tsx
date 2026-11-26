import { useTranslation } from "react-i18next";
import { useUIStore, type SettingsTab } from "@/stores/ui.store";
import AccountsTab from "./AccountsTab";
import GeneralTab from "./GeneralTab";
import ProxyTab from "./ProxyTab";
import AppearanceTab from "./AppearanceTab";
import CloudSyncTab from "./CloudSyncTab";
import RulesTab from "./RulesTab";
import PendingOpsTab from "./PendingOpsTab";
import ShortcutsTab from "./ShortcutsTab";
import TranslateTab from "./TranslateTab";
import PrivacyTab from "./PrivacyTab";
import AboutTab from "./AboutTab";
import MailArchiveView from "../mail-archive/MailArchiveView";

const TAB_IDS = ["accounts", "general", "proxy", "appearance", "privacy", "rules", "remoteWrites", "mailArchive", "translation", "shortcuts", "cloudSync", "about"] as const;

const TAB_LABEL_KEYS: Record<SettingsTab, string> = {
  accounts: "settings.accounts",
  general: "settings.general",
  proxy: "settings.proxy",
  appearance: "settings.appearance",
  privacy: "settings.privacy",
  rules: "settings.rules",
  remoteWrites: "settings.remoteWrites",
  mailArchive: "mailArchive.title",
  translation: "settings.translation",
  shortcuts: "settings.shortcuts",
  cloudSync: "settings.cloudSync",
  about: "settings.about",
};

/** Optional one-line description under each tab's page title. */
const TAB_DESCRIPTION_KEYS: Partial<Record<SettingsTab, string>> = {
  cloudSync: "cloudSync.description",
  remoteWrites: "pendingOps.description",
};

export default function SettingsView() {
  const { t } = useTranslation();
  const activeTab = useUIStore((s) => s.settingsTab);
  const setSettingsTab = useUIStore((s) => s.setSettingsTab);
  const descriptionKey = TAB_DESCRIPTION_KEYS[activeTab];

  function handleTabChange(id: SettingsTab) {
    setSettingsTab(id);
  }

  return (
    <div className="settings-view">
      <nav className="settings-nav" aria-label={t("settings.tabs", "Settings tabs")}>
        <div role="tablist" aria-orientation="vertical" aria-label={t("settings.tabs", "Settings tabs")}>
          {TAB_IDS.map((id, index) => (
            <button
              key={id}
              id={`settings-tab-${id}`}
              role="tab"
              type="button"
              className={activeTab === id ? "sidebar-btn is-active" : "sidebar-btn"}
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
            >
              <span className="sidebar-btn__label">{t(TAB_LABEL_KEYS[id])}</span>
            </button>
          ))}
        </div>
      </nav>
      <div
        id={`settings-tabpanel-${activeTab}`}
        className="scroll-region settings-panel-scroll settings-panel"
        role="tabpanel"
        aria-labelledby={`settings-tab-${activeTab}`}
        style={{
          maxWidth: activeTab === "remoteWrites" || activeTab === "mailArchive" ? "980px" : "640px",
          boxSizing: "border-box",
          overflowY: "auto",
          overflowX: "hidden",
        }}
      >
        {/* Mail archive renders its own header. */}
        {activeTab !== "mailArchive" && (
          <header className="page-header page-header--flush">
            <div className="page-header__text">
              <h2 className="page-header__title">{t(TAB_LABEL_KEYS[activeTab])}</h2>
              {descriptionKey && <p className="page-header__description">{t(descriptionKey)}</p>}
            </div>
          </header>
        )}
        {activeTab === "accounts" && <AccountsTab />}
        {activeTab === "general" && <GeneralTab />}
        {activeTab === "proxy" && <ProxyTab />}
        {activeTab === "appearance" && <AppearanceTab />}
        {activeTab === "rules" && <RulesTab />}
        {activeTab === "remoteWrites" && <PendingOpsTab />}
        {activeTab === "mailArchive" && <MailArchiveView />}
        {activeTab === "translation" && <TranslateTab />}
        {activeTab === "shortcuts" && <ShortcutsTab />}
        {activeTab === "privacy" && <PrivacyTab />}
        {activeTab === "cloudSync" && <CloudSyncTab />}
        {activeTab === "about" && <AboutTab />}
      </div>
    </div>
  );
}
