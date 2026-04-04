import { create } from "zustand";
import {
  listAccounts,
  listFolders,
  listMessages,
  listThreads,
  listThreadMessages,
  startSync,
} from "@/lib/api";
import type { Account, Folder, Message, ThreadSummary } from "@/lib/api";

interface MailState {
  // ─── UI State ──────────────────────────────────────────────────────────────
  activeAccountId: string | null;
  activeFolderId: string | null;
  selectedMessageId: string | null;
  selectedThreadId: string | null;
  threadView: boolean;

  setActiveAccountId: (accountId: string | null) => void;
  setActiveFolderId: (folderId: string | null) => void;
  setSelectedMessage: (messageId: string | null) => void;
  setSelectedThreadId: (threadId: string | null) => void;
  toggleThreadView: () => void;
  setMessages: (messages: Message[]) => void;

  // ─── Convenience cache (kept for backward compatibility) ───────────────────
  accounts: Account[];
  folders: Folder[];
  messages: Message[];
  threads: ThreadSummary[];
  threadMessages: Message[];
  loadingMessages: boolean;
  loadingFolders: boolean;
  loadingThreadMessages: boolean;

  // ─── Legacy fetch methods (kept for non-refactored consumers) ─────────────
  fetchAccounts: () => Promise<void>;
  fetchFolders: (accountId: string) => Promise<void>;
  fetchMessages: (
    folderId: string,
    limit?: number,
    offset?: number,
  ) => Promise<void>;
  setActiveAccount: (accountId: string) => Promise<void>;
  setActiveFolder: (folderId: string) => Promise<void>;
  syncAccount: (accountId: string) => Promise<void>;
  fetchThreads: (folderId: string, limit?: number, offset?: number) => Promise<void>;
  selectThread: (threadId: string) => Promise<void>;
  clearThread: () => void;
}

export const useMailStore = create<MailState>((set, get) => ({
  // ─── UI State ──────────────────────────────────────────────────────────────
  activeAccountId: null,
  activeFolderId: null,
  selectedMessageId: null,
  selectedThreadId: null,
  threadView: false,

  setActiveAccountId: (accountId) => {
    set({
      activeAccountId: accountId,
      activeFolderId: null,
      selectedMessageId: null,
      selectedThreadId: null,
      folders: [],
      messages: [],
      threads: [],
    });
  },

  setActiveFolderId: (folderId) => {
    set({
      activeFolderId: folderId,
      selectedMessageId: null,
      selectedThreadId: null,
      messages: [],
      threads: [],
    });
  },

  setSelectedMessage: (messageId) => {
    set({ selectedMessageId: messageId });
  },

  setSelectedThreadId: (threadId) => {
    set({ selectedThreadId: threadId });
  },

  toggleThreadView: () => {
    const current = get().threadView;
    set({ threadView: !current, selectedThreadId: null, threadMessages: [] });
    const folderId = get().activeFolderId;
    if (folderId) {
      if (!current) {
        get().fetchThreads(folderId);
      } else {
        get().fetchMessages(folderId);
      }
    }
  },

  // ─── Convenience cache ────────────────────────────────────────────────────
  accounts: [],
  folders: [],
  messages: [],
  threads: [],
  threadMessages: [],
  loadingMessages: false,
  loadingFolders: false,
  loadingThreadMessages: false,

  // ─── Legacy fetch methods ─────────────────────────────────────────────────
  fetchAccounts: async () => {
    const accounts = await listAccounts();
    set({ accounts });
  },

  fetchFolders: async (accountId: string) => {
    set({ loadingFolders: true });
    try {
      const folders = await listFolders(accountId);
      const sorted = [...folders].sort((a, b) => a.sort_order - b.sort_order);
      set({ folders: sorted });
    } finally {
      set({ loadingFolders: false });
    }
  },

  fetchMessages: async (folderId: string, limit = 50, offset = 0) => {
    set({ loadingMessages: true });
    try {
      const messages = await listMessages(folderId, limit, offset);
      set({ messages });
    } finally {
      set({ loadingMessages: false });
    }
  },

  setActiveAccount: async (accountId: string) => {
    set({
      activeAccountId: accountId,
      folders: [],
      messages: [],
      selectedMessageId: null,
      activeFolderId: null,
    });
    await get().fetchFolders(accountId);
    const inbox = get().folders.find((f) => f.role === "inbox");
    if (inbox) {
      await get().setActiveFolder(inbox.id);
    }
  },

  setActiveFolder: async (folderId: string) => {
    set({
      activeFolderId: folderId,
      messages: [],
      selectedMessageId: null,
    });
    await get().fetchMessages(folderId);
  },

  syncAccount: async (accountId: string) => {
    await startSync(accountId);
    const { activeFolderId } = get();
    await get().fetchFolders(accountId);
    if (activeFolderId) {
      await get().fetchMessages(activeFolderId);
    }
  },

  fetchThreads: async (folderId: string, limit = 50, offset = 0) => {
    set({ loadingMessages: true });
    try {
      const threads = await listThreads(folderId, limit, offset);
      set({ threads });
    } finally {
      set({ loadingMessages: false });
    }
  },

  selectThread: async (threadId: string) => {
    set({ selectedThreadId: threadId, loadingThreadMessages: true });
    try {
      const threadMessages = await listThreadMessages(threadId);
      set({ threadMessages });
    } finally {
      set({ loadingThreadMessages: false });
    }
  },

  clearThread: () => {
    set({ selectedThreadId: null, threadMessages: [] });
  },

  setMessages: (messages) => set({ messages }),
}));
