import { create } from "zustand";
import i18n from "@/lib/i18n";
import { useComposeStore } from "./compose.store";

export type ActiveView = "inbox" | "kanban" | "settings" | "search" | "snoozed" | "starred" | "compose";
export type Theme = "light" | "dark" | "system";
export type Language = "en" | "zh";
export type NetworkStatus = "online" | "offline";

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
  previousView: ActiveView;
  toggleSidebar: () => void;
  setActiveView: (view: ActiveView) => void;
  setTheme: (theme: Theme) => void;
  setLanguage: (lang: Language) => void;
  setSyncStatus: (status: "idle" | "syncing" | "error") => void;
  setNetworkStatus: (status: NetworkStatus) => void;
  setLastMailError: (error: string | null) => void;
  pollInterval: number;
  setPollInterval: (secs: number) => void;
  searchQuery: string;
  setSearchQuery: (q: string) => void;
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
  pollInterval: Number(localStorage.getItem("pebble-poll-interval")) || 15,
  setPollInterval: (secs) => {
    localStorage.setItem("pebble-poll-interval", String(secs));
    set({ pollInterval: secs });
  },
  searchQuery: "",
  setSearchQuery: (q) => set({ searchQuery: q }),
  showFolderUnreadCount: localStorage.getItem("pebble-show-unread-count") === "true",
  setShowFolderUnreadCount: (show) => {
    localStorage.setItem("pebble-show-unread-count", String(show));
    set({ showFolderUnreadCount: show });
  },
}));
