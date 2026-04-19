import { create } from "zustand";
import i18n from "@/lib/i18n";
import { useComposeStore } from "./compose.store";
import { useMailStore } from "./mail.store";

export type ActiveView = "inbox" | "kanban" | "settings" | "search" | "snoozed" | "starred" | "compose";
export type SettingsTab = "accounts" | "general" | "appearance" | "privacy" | "rules" | "remoteWrites" | "translation" | "shortcuts" | "cloudSync" | "about";
export type Theme = "light" | "dark" | "system";
export type Language = "en" | "zh";
export type NetworkStatus = "online" | "offline";
export type RealtimeMode = "realtime" | "polling" | "backoff" | "offline" | "auth_required" | "error";
export type RealtimePreference = "realtime" | "balanced" | "battery" | "manual";

export interface RealtimeStatus {
  account_id: string;
  mode: RealtimeMode;
  provider: string;
  last_success_at?: number | null;
  next_retry_at?: number | null;
  message?: string | null;
}

const REALTIME_PREFERENCE_KEY = "pebble-realtime-mode";
const REALTIME_PREFERENCES = new Set<RealtimePreference>(["realtime", "balanced", "battery", "manual"]);
const NOTIFICATIONS_KEY = "pebble-notifications-enabled";

function readRealtimePreference(): RealtimePreference {
  const stored = localStorage.getItem(REALTIME_PREFERENCE_KEY);
  return REALTIME_PREFERENCES.has(stored as RealtimePreference)
    ? (stored as RealtimePreference)
    : "realtime";
}

export function readNotificationsEnabledPreference(): boolean {
  const stored = localStorage.getItem(NOTIFICATIONS_KEY);
  return stored === null ? true : stored === "true";
}

export function realtimePreferenceToPollInterval(mode: RealtimePreference): number {
  switch (mode) {
    case "realtime":
      return 3;
    case "balanced":
      return 15;
    case "battery":
      return 60;
    case "manual":
      return 0;
  }
}

const initialRealtimeMode = readRealtimePreference();
const initialNotificationsEnabled = readNotificationsEnabledPreference();

/** Resolve "system" theme to an actual "dark" | "light" value. */
function resolveTheme(theme: Theme): "dark" | "light" {
  if (theme === "system") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  return theme;
}

/** Apply the resolved theme to the DOM immediately (no React effect needed). */
export function applyThemeToDom(theme: Theme) {
  document.documentElement.setAttribute("data-theme", resolveTheme(theme));
}

interface UIState {
  sidebarCollapsed: boolean;
  activeView: ActiveView;
  theme: Theme;
  language: Language;
  syncStatus: "idle" | "syncing" | "error";
  networkStatus: NetworkStatus;
  lastMailError: string | null;
  realtimeStatusByAccount: Record<string, RealtimeStatus>;
  realtimeMode: RealtimePreference;
  notificationsEnabled: boolean;
  setNotificationsEnabled: (enabled: boolean) => void;
  previousView: ActiveView;
  toggleSidebar: () => void;
  setActiveView: (view: ActiveView) => void;
  openMessageInInbox: (messageId: string) => void;
  setTheme: (theme: Theme) => void;
  setLanguage: (lang: Language) => void;
  setSyncStatus: (status: "idle" | "syncing" | "error") => void;
  setNetworkStatus: (status: NetworkStatus) => void;
  setLastMailError: (error: string | null) => void;
  setRealtimeStatus: (accountId: string, status: RealtimeStatus) => void;
  setRealtimeMode: (mode: RealtimePreference) => void;
  pollInterval: number;
  setPollInterval: (secs: number) => void;
  searchQuery: string;
  setSearchQuery: (q: string) => void;
  settingsTab: SettingsTab;
  setSettingsTab: (tab: SettingsTab) => void;
  pendingRuleDraftText: string | null;
  setPendingRuleDraftText: (text: string | null) => void;
  showFolderUnreadCount: boolean;
  setShowFolderUnreadCount: (show: boolean) => void;
}

export const useUIStore = create<UIState>((set) => ({
  sidebarCollapsed: false,
  activeView: "inbox",
  theme: (localStorage.getItem("pebble-theme") as Theme) || "light",
  language: (localStorage.getItem("pebble-language") as Language) || "en",
  syncStatus: "idle",
  networkStatus: "online",
  lastMailError: null,
  realtimeStatusByAccount: {},
  realtimeMode: initialRealtimeMode,
  notificationsEnabled: initialNotificationsEnabled,
  setNotificationsEnabled: (enabled) => {
    localStorage.setItem(NOTIFICATIONS_KEY, String(enabled));
    set({ notificationsEnabled: enabled });
  },
  previousView: "inbox",
  toggleSidebar: () =>
    set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),
  setActiveView: (view) => {
    const state = useUIStore.getState();
    if (state.activeView === view) {
      return;
    }

    // Delegate dirty-compose guard to the compose store
    if (state.activeView === "compose" && view !== "compose") {
      const composeState = useComposeStore.getState();
      if (composeState.composeDirty) {
        useComposeStore.setState({ showComposeLeaveConfirm: true, pendingView: view });
        return;
      }
      useComposeStore.setState({
        composeMode: null,
        composeReplyTo: null,
        composeDirty: false,
      });
      set({ activeView: view });
      return;
    }

    set({ activeView: view });
  },
  openMessageInInbox: (messageId) => {
    useMailStore.setState({
      selectedMessageId: messageId,
      selectedThreadId: null,
      threadView: false,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
    set({ activeView: "inbox" });
  },
  setTheme: (theme) => {
    localStorage.setItem("pebble-theme", theme);
    applyThemeToDom(theme);
    set({ theme });
  },
  setLanguage: (lang) => {
    i18n.changeLanguage(lang);
    localStorage.setItem("pebble-language", lang);
    set({ language: lang });
  },
  setSyncStatus: (status) => set({ syncStatus: status }),
  setNetworkStatus: (status) => set({ networkStatus: status }),
  setLastMailError: (error) => set({ lastMailError: error }),
  setRealtimeStatus: (accountId, status) =>
    set((state) => ({
      realtimeStatusByAccount: {
        ...state.realtimeStatusByAccount,
        [accountId]: status,
      },
    })),
  setRealtimeMode: (mode) => {
    const pollInterval = realtimePreferenceToPollInterval(mode);
    localStorage.setItem(REALTIME_PREFERENCE_KEY, mode);
    localStorage.setItem("pebble-poll-interval", String(pollInterval));
    set({
      realtimeMode: mode,
      pollInterval,
    });
  },
  pollInterval: realtimePreferenceToPollInterval(initialRealtimeMode),
  setPollInterval: (secs) => {
    localStorage.setItem("pebble-poll-interval", String(secs));
    set({ pollInterval: secs });
  },
  searchQuery: "",
  setSearchQuery: (q) => set({ searchQuery: q }),
  settingsTab: (sessionStorage.getItem("pebble-settings-tab") as SettingsTab) || "accounts",
  setSettingsTab: (tab) => {
    sessionStorage.setItem("pebble-settings-tab", tab);
    set({ settingsTab: tab });
  },
  pendingRuleDraftText: null,
  setPendingRuleDraftText: (text) => set({ pendingRuleDraftText: text }),
  showFolderUnreadCount: localStorage.getItem("pebble-show-unread-count") === "true",
  setShowFolderUnreadCount: (show) => {
    localStorage.setItem("pebble-show-unread-count", String(show));
    set({ showFolderUnreadCount: show });
  },
}));
