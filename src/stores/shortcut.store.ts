import { create } from "zustand";
import { profileLocalStorage } from "@/lib/profileStorage";

export interface ShortcutBinding {
  actionId: string;
  keys: string;
}

interface ShortcutState {
  bindings: Record<string, string>;
  recording: string | null;
  /** Vim-style keys (dd, gg, G, gi, h, l…) layered on top of the bindings */
  vimMode: boolean;
  setVimMode: (enabled: boolean) => void;
  updateShortcut: (actionId: string, keys: string) => void;
  resetToDefaults: () => void;
  startRecording: (actionId: string) => void;
  stopRecording: () => void;
  detectConflict: (keys: string, excludeAction?: string) => string | null;
}

const DEFAULT_BINDINGS: Record<string, string> = {
  "command-palette": "Ctrl+K",
  "close-modal": "Escape",
  "next-message": "J",
  "prev-message": "K",
  "open-message": "Enter",
  "toggle-star": "S",
  "archive-message": "E",
  "delete-message": "Delete",
  "compose-new": "C",
  "reply": "R",
  "reply-all": "A",
  "forward": "F",
  "focus-search": "/",
  "toggle-view-inbox": "Ctrl+Shift+I",
  "toggle-view-kanban": "Ctrl+Shift+K",
  "open-search": "Ctrl+Shift+F",
  "open-cloud-settings": "Ctrl+Shift+B",
  "toggle-notifications": "Ctrl+Shift+N",
  "translate-selection": "T",
  "toggle-bilingual": "Ctrl+Shift+T",
};

const STORAGE_KEY = "pebble-shortcuts";
const VIM_MODE_STORAGE_KEY = "pebble-vim-mode";

function loadVimMode(): boolean {
  try {
    return profileLocalStorage.getItem(VIM_MODE_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

function loadBindings(): Record<string, string> {
  try {
    const saved = profileLocalStorage.getItem(STORAGE_KEY);
    if (saved) return { ...DEFAULT_BINDINGS, ...JSON.parse(saved) };
  } catch {
    // ignore parse errors
  }
  return { ...DEFAULT_BINDINGS };
}

export const useShortcutStore = create<ShortcutState>((set, get) => ({
  bindings: loadBindings(),
  recording: null,
  vimMode: loadVimMode(),
  setVimMode: (enabled) => {
    try {
      profileLocalStorage.setItem(VIM_MODE_STORAGE_KEY, String(enabled));
    } catch {
      // storage unavailable: keep the in-memory setting
    }
    set({ vimMode: enabled });
  },
  updateShortcut: (actionId, keys) => {
    const newBindings = { ...get().bindings, [actionId]: keys };
    profileLocalStorage.setItem(STORAGE_KEY, JSON.stringify(newBindings));
    set({ bindings: newBindings, recording: null });
  },
  resetToDefaults: () => {
    profileLocalStorage.removeItem(STORAGE_KEY);
    set({ bindings: { ...DEFAULT_BINDINGS } });
  },
  startRecording: (actionId) => set({ recording: actionId }),
  stopRecording: () => set({ recording: null }),
  detectConflict: (keys, excludeAction) => {
    const { bindings } = get();
    for (const [action, bound] of Object.entries(bindings)) {
      if (action !== excludeAction && bound.toLowerCase() === keys.toLowerCase()) {
        return action;
      }
    }
    return null;
  },
}));

export { DEFAULT_BINDINGS };

/** Keyboard event → stored binding format ("Ctrl" = Ctrl or ⌘). */
export function eventToKeyString(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (e.shiftKey) parts.push("Shift");
  if (e.altKey) parts.push("Alt");
  const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  if (!["Control", "Meta", "Shift", "Alt"].includes(e.key)) {
    parts.push(key);
  }
  return parts.join("+");
}

/**
 * Bindings are stored platform-neutrally: "Ctrl" means the primary modifier
 * (⌘ on macOS, Ctrl elsewhere). Use formatShortcut() for anything shown to users.
 */
export function isMacPlatform(): boolean {
  if (typeof navigator === "undefined") return false;
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  const platform = nav.userAgentData?.platform || nav.platform || nav.userAgent || "";
  return /mac|iphone|ipad|ipod/i.test(platform);
}

const MAC_SYMBOLS: Record<string, string> = {
  Ctrl: "⌘",
  Shift: "⇧",
  Alt: "⌥",
  Enter: "↩",
  Escape: "Esc",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Backspace: "⌫",
  Delete: "⌦",
};

const OTHER_NAMES: Record<string, string> = {
  Escape: "Esc",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
};

/** "Ctrl+Shift+K" → "⌘⇧K" on macOS, "Ctrl+Shift+K" elsewhere. */
export function formatShortcut(keys: string | undefined | null, mac: boolean = isMacPlatform()): string {
  if (!keys) return "";
  // Split on "+" but keep a literal "+" key ("Ctrl++").
  const parts = keys.split(/\+(?!$)/).filter(Boolean);
  if (mac) return parts.map((part) => MAC_SYMBOLS[part] ?? part).join("");
  return parts.map((part) => OTHER_NAMES[part] ?? part).join("+");
}
