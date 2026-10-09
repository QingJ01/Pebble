import { create } from "zustand";
import i18n from "@/lib/i18n";
import { getInitialLanguage, LANGUAGE_STORAGE_KEY, type Language } from "@/lib/language";
import { profileLocalStorage, profileSessionStorage } from "@/lib/profileStorage";
import { readStartHiddenToTrayPreference, START_HIDDEN_TO_TRAY_KEY } from "@/lib/startupVisibility";
import { useComposeStore } from "./compose.store";
import { useMailStore } from "./mail.store";

export type ActiveView = "inbox" | "kanban" | "contacts" | "settings" | "search" | "snoozed" | "starred" | "compose";
export type SettingsTab = "accounts" | "general" | "proxy" | "appearance" | "privacy" | "rules" | "remoteWrites" | "translation" | "shortcuts" | "cloudSync" | "about";
export type Theme = "light" | "dark" | "system";
export type { Language } from "@/lib/language";
export type NetworkStatus = "online" | "offline";
export type RealtimeMode = "realtime" | "polling" | "manual" | "backoff" | "offline" | "auth_required" | "error";
export type RealtimePreference = "realtime" | "balanced" | "battery" | "manual";
export type BackgroundImageFit = "cover" | "contain" | "repeat";
export type SidebarStyle = "grouped" | "classic";

export interface BackgroundImageSettings {
  path: string;
  filename: string;
  fit: BackgroundImageFit;
  opacity: number;
  updatedAt: number;
}

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
const KEEP_RUNNING_BACKGROUND_KEY = "pebble-keep-running-background";
export const BACKGROUND_IMAGE_STORAGE_KEY = "pebble-background-image-settings";
const BACKGROUND_IMAGE_FITS = new Set<BackgroundImageFit>(["cover", "contain", "repeat"]);
const DEFAULT_BACKGROUND_IMAGE_FIT: BackgroundImageFit = "cover";
const DEFAULT_BACKGROUND_IMAGE_OPACITY = 0.35;
const MIN_BACKGROUND_IMAGE_OPACITY = 0.05;
const MAX_BACKGROUND_IMAGE_OPACITY = 1;
const SIDEBAR_STYLE_KEY = "pebble-sidebar-style";

function readSidebarStyle(): SidebarStyle {
  return profileLocalStorage.getItem(SIDEBAR_STYLE_KEY) === "classic" ? "classic" : "grouped";
}

function readRealtimePreference(): RealtimePreference {
  const stored = profileLocalStorage.getItem(REALTIME_PREFERENCE_KEY);
  return REALTIME_PREFERENCES.has(stored as RealtimePreference)
    ? (stored as RealtimePreference)
    : "realtime";
}

function clampBackgroundImageOpacity(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_BACKGROUND_IMAGE_OPACITY;
  return Math.min(MAX_BACKGROUND_IMAGE_OPACITY, Math.max(MIN_BACKGROUND_IMAGE_OPACITY, value));
}

function readBackgroundImageSettings(): BackgroundImageSettings | null {
  const stored = profileLocalStorage.getItem(BACKGROUND_IMAGE_STORAGE_KEY);
  if (!stored) return null;
  try {
    const parsed = JSON.parse(stored) as Partial<BackgroundImageSettings>;
    if (!parsed || typeof parsed.path !== "string" || typeof parsed.filename !== "string") {
      return null;
    }
    const fit = BACKGROUND_IMAGE_FITS.has(parsed.fit as BackgroundImageFit)
      ? parsed.fit as BackgroundImageFit
      : DEFAULT_BACKGROUND_IMAGE_FIT;
    return {
      path: parsed.path,
      filename: parsed.filename,
      fit,
      opacity: clampBackgroundImageOpacity(Number(parsed.opacity ?? DEFAULT_BACKGROUND_IMAGE_OPACITY)),
      updatedAt: Number.isFinite(Number(parsed.updatedAt)) ? Number(parsed.updatedAt) : Date.now(),
    };
  } catch {
    return null;
  }
}

function persistBackgroundImageSettings(settings: BackgroundImageSettings | null) {
  if (!settings) {
    profileLocalStorage.removeItem(BACKGROUND_IMAGE_STORAGE_KEY);
    return;
  }
  profileLocalStorage.setItem(BACKGROUND_IMAGE_STORAGE_KEY, JSON.stringify(settings));
}

export function readNotificationsEnabledPreference(): boolean {
  const stored = profileLocalStorage.getItem(NOTIFICATIONS_KEY);
  return stored === null ? true : stored === "true";
}

export function readKeepRunningInBackgroundPreference(): boolean {
  const stored = profileLocalStorage.getItem(KEEP_RUNNING_BACKGROUND_KEY);
  return stored === null ? true : stored === "true";
}

// The status bar once described this toggle by the action a click would
// perform ("Exit on close") while styling the current state, so users who
// meant to keep the app in the background often stored "false" by mistake.
// Correct it once for existing profiles.
const KEEP_RUNNING_BACKGROUND_RESET_MARKER = "pebble-keep-running-background-reset-v1";

function correctMisleadingKeepRunningPreferenceOnce() {
  if (profileLocalStorage.getItem(KEEP_RUNNING_BACKGROUND_RESET_MARKER) === "done") return;
  if (profileLocalStorage.getItem(KEEP_RUNNING_BACKGROUND_KEY) === "false") {
    profileLocalStorage.setItem(KEEP_RUNNING_BACKGROUND_KEY, "true");
  }
  profileLocalStorage.setItem(KEEP_RUNNING_BACKGROUND_RESET_MARKER, "done");
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
correctMisleadingKeepRunningPreferenceOnce();
const initialKeepRunningInBackground = readKeepRunningInBackgroundPreference();
const initialStartHiddenToTray = readStartHiddenToTrayPreference();
const initialLanguage = getInitialLanguage();
const initialBackgroundImage = readBackgroundImageSettings();
const initialSidebarStyle = readSidebarStyle();

/** Resolve "system" theme to an actual "dark" | "light" value. */
export function resolveTheme(theme: Theme): "dark" | "light" {
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
  backgroundImage: BackgroundImageSettings | null;
  language: Language;
  syncStatus: "idle" | "syncing" | "error";
  networkStatus: NetworkStatus;
  lastMailError: string | null;
  realtimeStatusByAccount: Record<string, RealtimeStatus>;
  realtimeMode: RealtimePreference;
  notificationsEnabled: boolean;
  setNotificationsEnabled: (enabled: boolean) => void;
  keepRunningInBackground: boolean;
  setKeepRunningInBackground: (enabled: boolean) => void;
  startHiddenToTray: boolean;
  setStartHiddenToTray: (enabled: boolean) => void;
  previousView: ActiveView;
  pendingContactId: string | null;
  toggleSidebar: () => void;
  setActiveView: (view: ActiveView) => void;
  openContactInContacts: (contactId: string) => void;
  clearPendingContact: () => void;
  openMessageInInbox: (messageId: string) => void;
  setTheme: (theme: Theme) => void;
  setBackgroundImage: (image: { path: string; filename: string }) => void;
  setBackgroundImageFit: (fit: BackgroundImageFit) => void;
  setBackgroundImageOpacity: (opacity: number) => void;
  clearBackgroundImage: () => void;
  sidebarStyle: SidebarStyle;
  setSidebarStyle: (style: SidebarStyle) => void;
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
  theme: (profileLocalStorage.getItem("pebble-theme") as Theme) || "light",
  backgroundImage: initialBackgroundImage,
  language: initialLanguage,
  syncStatus: "idle",
  networkStatus: "online",
  lastMailError: null,
  realtimeStatusByAccount: {},
  realtimeMode: initialRealtimeMode,
  notificationsEnabled: initialNotificationsEnabled,
  setNotificationsEnabled: (enabled) => {
    profileLocalStorage.setItem(NOTIFICATIONS_KEY, String(enabled));
    set({ notificationsEnabled: enabled });
  },
  keepRunningInBackground: initialKeepRunningInBackground,
  setKeepRunningInBackground: (enabled) => {
    profileLocalStorage.setItem(KEEP_RUNNING_BACKGROUND_KEY, String(enabled));
    set({ keepRunningInBackground: enabled });
  },
  startHiddenToTray: initialStartHiddenToTray,
  setStartHiddenToTray: (enabled) => {
    profileLocalStorage.setItem(START_HIDDEN_TO_TRAY_KEY, String(enabled));
    set({ startHiddenToTray: enabled });
  },
  previousView: "inbox",
  pendingContactId: null,
  toggleSidebar: () =>
    set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),
  setActiveView: (view) => {
    const state = useUIStore.getState();
    if (state.activeView === view) {
      return;
    }

    if (state.activeView === "compose" && view !== "compose") {
      const composeState = useComposeStore.getState();
      if (composeState.composeDirty) {
        useComposeStore.setState({ showComposeLeaveConfirm: true, pendingView: view });
        return;
      }

      useComposeStore.setState({
        composeMode: null,
        composeReplyTo: null,
        composePrefill: null,
        composeDirty: false,
        showComposeLeaveConfirm: false,
        pendingView: null,
      });
    }

    set({ activeView: view });
  },
  openContactInContacts: (contactId) => set({
    activeView: "contacts",
    pendingContactId: contactId,
  }),
  clearPendingContact: () => set({ pendingContactId: null }),
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
    profileLocalStorage.setItem("pebble-theme", theme);
    applyThemeToDom(theme);
    set({ theme });
  },
  setBackgroundImage: (image) => {
    const current = useUIStore.getState().backgroundImage;
    const next: BackgroundImageSettings = {
      path: image.path,
      filename: image.filename,
      fit: current?.fit ?? DEFAULT_BACKGROUND_IMAGE_FIT,
      opacity: current?.opacity ?? DEFAULT_BACKGROUND_IMAGE_OPACITY,
      updatedAt: Date.now(),
    };
    persistBackgroundImageSettings(next);
    set({ backgroundImage: next });
  },
  setBackgroundImageFit: (fit) => {
    if (!BACKGROUND_IMAGE_FITS.has(fit)) return;
    const current = useUIStore.getState().backgroundImage;
    if (!current) return;
    const next = { ...current, fit, updatedAt: Date.now() };
    persistBackgroundImageSettings(next);
    set({ backgroundImage: next });
  },
  setBackgroundImageOpacity: (opacity) => {
    const current = useUIStore.getState().backgroundImage;
    if (!current) return;
    const next = {
      ...current,
      opacity: clampBackgroundImageOpacity(opacity),
      updatedAt: Date.now(),
    };
    persistBackgroundImageSettings(next);
    set({ backgroundImage: next });
  },
  clearBackgroundImage: () => {
    persistBackgroundImageSettings(null);
    set({ backgroundImage: null });
  },
  sidebarStyle: initialSidebarStyle,
  setSidebarStyle: (style) => {
    profileLocalStorage.setItem(SIDEBAR_STYLE_KEY, style);
    set({ sidebarStyle: style });
  },
  setLanguage: (lang) => {
    i18n.changeLanguage(lang);
    profileLocalStorage.setItem(LANGUAGE_STORAGE_KEY, lang);
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
    profileLocalStorage.setItem(REALTIME_PREFERENCE_KEY, mode);
    profileLocalStorage.setItem("pebble-poll-interval", String(pollInterval));
    set({
      realtimeMode: mode,
      pollInterval,
    });
  },
  pollInterval: realtimePreferenceToPollInterval(initialRealtimeMode),
  setPollInterval: (secs) => {
    profileLocalStorage.setItem("pebble-poll-interval", String(secs));
    set({ pollInterval: secs });
  },
  searchQuery: "",
  setSearchQuery: (q) => set({ searchQuery: q }),
  settingsTab: (profileSessionStorage.getItem("pebble-settings-tab") as SettingsTab) || "accounts",
  setSettingsTab: (tab) => {
    profileSessionStorage.setItem("pebble-settings-tab", tab);
    set({ settingsTab: tab });
  },
  pendingRuleDraftText: null,
  setPendingRuleDraftText: (text) => set({ pendingRuleDraftText: text }),
  showFolderUnreadCount: profileLocalStorage.getItem("pebble-show-unread-count") === "true",
  setShowFolderUnreadCount: (show) => {
    profileLocalStorage.setItem("pebble-show-unread-count", String(show));
    set({ showFolderUnreadCount: show });
  },
}));
