import { create } from "zustand";

export interface ShortcutBinding {
  actionId: string;
  keys: string;
}

interface ShortcutState {
  bindings: Record<string, string>;
  recording: string | null;
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
  "compose-new": "C",
  "reply": "R",
  "toggle-view-inbox": "Ctrl+Shift+I",
  "toggle-view-kanban": "Ctrl+Shift+K",
  "open-search": "Ctrl+Shift+F",
  "backup-to-cloud": "Ctrl+Shift+B",
  "toggle-notifications": "Ctrl+Shift+N",
};

const STORAGE_KEY = "pebble-shortcuts";

function loadBindings(): Record<string, string> {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return { ...DEFAULT_BINDINGS, ...JSON.parse(saved) };
  } catch {
    // ignore parse errors
  }
  return { ...DEFAULT_BINDINGS };
}

export const useShortcutStore = create<ShortcutState>((set, get) => ({
  bindings: loadBindings(),
  recording: null,
  updateShortcut: (actionId, keys) => {
    const newBindings = { ...get().bindings, [actionId]: keys };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(newBindings));
    set({ bindings: newBindings, recording: null });
  },
  resetToDefaults: () => {
    localStorage.removeItem(STORAGE_KEY);
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
