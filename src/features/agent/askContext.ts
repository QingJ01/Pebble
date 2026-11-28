import { create } from "zustand";

/** The mail the user was reading when they chose "Ask Pebble about this". */
export interface AskContext {
  messageId: string;
  accountId: string;
  subject: string;
  from: string;
}

interface AskContextState {
  context: AskContext | null;
  setContext: (context: AskContext | null) => void;
}

export const useAskContext = create<AskContextState>((set) => ({
  context: null,
  setContext: (context) => set({ context }),
}));
