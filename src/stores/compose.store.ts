import { create } from "zustand";
import type { Message } from "@/lib/api";
import { useUIStore, type ActiveView } from "./ui.store";

export type ComposeMode = "new" | "reply" | "reply-all" | "forward";

interface ComposeState {
  composeMode: ComposeMode | null;
  composeReplyTo: Message | null;
  composeDirty: boolean;
  showComposeLeaveConfirm: boolean;
  pendingView: ActiveView | null;
  setComposeDirty: (dirty: boolean) => void;
  openCompose: (mode: ComposeMode, replyTo?: Message | null) => void;
  closeCompose: () => void;
  confirmCloseCompose: () => void;
  cancelCloseCompose: () => void;
}

function getComposeResetState() {
  return {
    composeMode: null as ComposeMode | null,
    composeReplyTo: null as Message | null,
    composeDirty: false,
  };
}

/** Check if compose view can be left without losing data. */
export function isComposeDirty(): boolean {
  const ui = useUIStore.getState();
  const compose = useComposeStore.getState();
  return ui.activeView === "compose" && compose.composeDirty;
}

export const useComposeStore = create<ComposeState>((set) => ({
  composeMode: null,
  composeReplyTo: null,
  composeDirty: false,
  showComposeLeaveConfirm: false,
  pendingView: null,
  setComposeDirty: (dirty) => set({ composeDirty: dirty }),
  openCompose: (mode, replyTo = null) => {
    const uiState = useUIStore.getState();
    useUIStore.setState({
      previousView: uiState.activeView === "compose" ? uiState.previousView : uiState.activeView,
      activeView: "compose" as ActiveView,
    });
    set({
      composeMode: mode,
      composeReplyTo: replyTo,
      composeDirty: false,
    });
  },
  closeCompose: () => {
    const state = useComposeStore.getState();
    const uiState = useUIStore.getState();
    if (uiState.activeView !== "compose") return;

    if (state.composeDirty) {
      set({ showComposeLeaveConfirm: true, pendingView: null });
      return;
    }

    useUIStore.setState({ activeView: uiState.previousView });
    set(getComposeResetState());
  },
  confirmCloseCompose: () => {
    const state = useComposeStore.getState();
    const uiState = useUIStore.getState();
    const targetView = state.pendingView ?? uiState.previousView;
    useUIStore.setState({ activeView: targetView });
    set({
      showComposeLeaveConfirm: false,
      pendingView: null,
      ...getComposeResetState(),
    });
  },
  cancelCloseCompose: () => set({ showComposeLeaveConfirm: false, pendingView: null }),
}));
