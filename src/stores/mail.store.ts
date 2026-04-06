import { create } from "zustand";

interface MailState {
  // ─── UI State ──────────────────────────────────────────────────────────────
  activeAccountId: string | null;
  activeFolderId: string | null;
  selectedMessageId: string | null;
  selectedThreadId: string | null;
  threadView: boolean;
  // ─── Batch Selection ───────────────────────────────────────────────────────
  selectedMessageIds: Set<string>;
  batchMode: boolean;

  setActiveAccountId: (accountId: string | null) => void;
  setActiveFolderId: (folderId: string | null) => void;
  setSelectedMessage: (messageId: string | null) => void;
  setSelectedThreadId: (threadId: string | null) => void;
  toggleThreadView: () => void;
  toggleBatchMode: () => void;
  toggleMessageSelection: (messageId: string) => void;
  selectAllMessages: (messageIds: string[]) => void;
  clearSelection: () => void;
}

export const useMailStore = create<MailState>((set, get) => ({
  activeAccountId: null,
  activeFolderId: null,
  selectedMessageId: null,
  selectedThreadId: null,
  threadView: false,
  selectedMessageIds: new Set<string>(),
  batchMode: false,

  setActiveAccountId: (accountId) => {
    set({
      activeAccountId: accountId,
      activeFolderId: null,
      selectedMessageId: null,
      selectedThreadId: null,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
  },

  setActiveFolderId: (folderId) => {
    set({
      activeFolderId: folderId,
      selectedMessageId: null,
      selectedThreadId: null,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
  },

  setSelectedMessage: (messageId) => {
    set({ selectedMessageId: messageId });
  },

  setSelectedThreadId: (threadId) => {
    set({ selectedThreadId: threadId });
  },

  toggleThreadView: () => {
    set({
      threadView: !get().threadView,
      selectedThreadId: null,
      selectedMessageId: null,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
  },

  toggleBatchMode: () => {
    const current = get().batchMode;
    set({
      batchMode: !current,
      selectedMessageIds: new Set(),
    });
  },

  toggleMessageSelection: (messageId) => {
    const ids = new Set(get().selectedMessageIds);
    if (ids.has(messageId)) ids.delete(messageId);
    else ids.add(messageId);
    set({ selectedMessageIds: ids });
  },

  selectAllMessages: (messageIds) => {
    set({ selectedMessageIds: new Set(messageIds) });
  },

  clearSelection: () => {
    set({ selectedMessageIds: new Set() });
  },
}));
