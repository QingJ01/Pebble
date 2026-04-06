import { create } from "zustand";
import type { Message } from "@/lib/api";

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

function getComposeResetState() {
  return {
    composeMode: null,
    composeReplyTo: null,
    composeDirty: false,
  };
}

export function canLeaveCompose(state: Pick<UIState, "activeView" | "composeDirty">): boolean {
  if (state.activeView !== "compose" || !state.composeDirty) {
    return true;
  }

  return globalThis.confirm("You have an unsaved draft. Discard and leave?");
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
  composeMode: "new" | "reply" | "reply-all" | "forward" | null;
  composeReplyTo: Message | null;
  composeDirty: boolean;
  setComposeDirty: (dirty: boolean) => void;
  toggleSidebar: () => void;
  setActiveView: (view: ActiveView) => void;
  setTheme: (theme: Theme) => void;
  setLanguage: (lang: Language) => void;
  setSyncStatus: (status: "idle" | "syncing" | "error") => void;
  setNetworkStatus: (status: NetworkStatus) => void;
  setLastMailError: (error: string | null) => void;
  openCompose: (mode: "new" | "reply" | "reply-all" | "forward", replyTo?: Message | null) => void;
  closeCompose: () => void;
  pollInterval: number;
  setPollInterval: (secs: number) => void;
  searchQuery: string;
  setSearchQuery: (q: string) => void;
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
  composeMode: null,
  composeReplyTo: null,
  composeDirty: false,
  setComposeDirty: (dirty) => set({ composeDirty: dirty }),
  toggleSidebar: () =>
    set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),
  setActiveView: (view) => {
    const state = useUIStore.getState();
    if (state.activeView === view) {
      return;
    }

    if (state.activeView === "compose" && view !== "compose") {
      if (!canLeaveCompose(state)) {
        return;
      }

      set({ activeView: view, ...getComposeResetState() });
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
    import("@/lib/i18n").then((mod) => mod.default.changeLanguage(lang));
    localStorage.setItem("pebble-language", lang);
    set({ language: lang });
  },
  setSyncStatus: (status) => set({ syncStatus: status }),
  setNetworkStatus: (status) => set({ networkStatus: status }),
  setLastMailError: (error) => set({ lastMailError: error }),
  openCompose: (mode, replyTo = null) =>
    set((state) => ({
      previousView: state.activeView === "compose" ? state.previousView : state.activeView,
      activeView: "compose" as ActiveView,
      composeMode: mode,
      composeReplyTo: replyTo,
      composeDirty: false,
    })),
  closeCompose: () => {
    const state = useUIStore.getState();
    if (state.activeView !== "compose") {
      return;
    }

    if (!canLeaveCompose(state)) {
      return;
    }

    set((current) => ({
      activeView: current.previousView,
      ...getComposeResetState(),
    }));
  },
  pollInterval: Number(localStorage.getItem("pebble-poll-interval")) || 15,
  setPollInterval: (secs) => {
    localStorage.setItem("pebble-poll-interval", String(secs));
    set({ pollInterval: secs });
  },
  searchQuery: "",
  setSearchQuery: (q) => set({ searchQuery: q }),
}));
