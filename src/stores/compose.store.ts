import { create } from "zustand";
import type { Message } from "@/lib/api";
import type { DraftData } from "@/hooks/useComposeDraft";
import { useUIStore, type ActiveView } from "./ui.store";

export type ComposeMode = "new" | "reply" | "reply-all" | "forward";

export interface ComposePrefill {
  to?: string[];
  cc?: string[];
  bcc?: string[];
  subject?: string;
  body?: string;
}

interface ComposeState {
  composeMode: ComposeMode | null;
  composeReplyTo: Message | null;
  composePrefill: ComposePrefill | null;
  composeDraft: DraftData | null;
  openDraft: (draft: DraftData) => void;
  pendingNavigation: (() => void) | null;
  composeKey: number;
  composeDirty: boolean;
  showComposeLeaveConfirm: boolean;
  pendingView: ActiveView | null;
  setComposeDirty: (dirty: boolean) => void;
  openCompose: (mode: ComposeMode, replyTo?: Message | null, prefill?: ComposePrefill | null) => void;
  closeCompose: () => void;
  confirmCloseCompose: () => void;
  cancelCloseCompose: () => void;
  discardComposeAndSetActiveView: (view: ActiveView) => void;
}

function getComposeResetState() {
  return {
    composeMode: null as ComposeMode | null,
    composeReplyTo: null as Message | null,
    composePrefill: null as ComposePrefill | null,
    composeDraft: null as DraftData | null,
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
  composePrefill: null,
  composeDraft: null,
  pendingNavigation: null,
  openDraft: (draft) => {
    useComposeStore.getState().openCompose("new");
    set({ composeDraft: draft });
  },
  composeKey: 0,
  composeDirty: false,
  showComposeLeaveConfirm: false,
  pendingView: null,
  setComposeDirty: (dirty) => set({ composeDirty: dirty }),
  openCompose: (mode, replyTo = null, prefill = null) => {
    const uiState = useUIStore.getState();
    if (isComposeDirty()) {
      set({ showComposeLeaveConfirm: true, pendingView: uiState.previousView,
        pendingNavigation: () => useComposeStore.getState().openCompose(mode, replyTo, prefill) });
      return;
    }
    useUIStore.setState({
      previousView: uiState.activeView === "compose" ? uiState.previousView : uiState.activeView,
      activeView: "compose" as ActiveView,
    });
    set((state) => ({
      composeMode: mode,
      composeReplyTo: replyTo,
      composePrefill: prefill,
      composeDraft: null,
      composeKey: state.composeKey + 1,
      composeDirty: false,
      showComposeLeaveConfirm: false,
      pendingView: null,
      pendingNavigation: null,
    }));
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
    useComposeStore.getState().discardComposeAndSetActiveView(targetView);
  },
  cancelCloseCompose: () => set({ showComposeLeaveConfirm: false, pendingView: null, pendingNavigation: null }),
  discardComposeAndSetActiveView: (view) => {
    const navigate = useComposeStore.getState().pendingNavigation;
    useUIStore.setState({ activeView: view });
    set({
      showComposeLeaveConfirm: false,
      pendingView: null,
      pendingNavigation: null,
      ...getComposeResetState(),
    });
    navigate?.();
  },
}));
