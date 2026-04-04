import { create } from "zustand";

export type ActiveView = "inbox" | "kanban" | "settings";
export type Theme = "light" | "dark" | "system";

interface UIState {
  sidebarCollapsed: boolean;
  activeView: ActiveView;
  theme: Theme;
  syncStatus: "idle" | "syncing" | "error";
  toggleSidebar: () => void;
  setActiveView: (view: ActiveView) => void;
  setTheme: (theme: Theme) => void;
  setSyncStatus: (status: "idle" | "syncing" | "error") => void;
}

export const useUIStore = create<UIState>((set) => ({
  sidebarCollapsed: false,
  activeView: "inbox",
  theme: "light",
  syncStatus: "idle",
  toggleSidebar: () =>
    set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),
  setActiveView: (view) => set({ activeView: view }),
  setTheme: (theme) => set({ theme }),
  setSyncStatus: (status) => set({ syncStatus: status }),
}));
