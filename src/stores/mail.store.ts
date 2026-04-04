import { create } from "zustand";
import {
  listAccounts,
  listFolders,
  listMessages,
  startSync,
} from "@/lib/api";
import type { Account, Folder, Message } from "@/lib/api";

interface MailState {
  accounts: Account[];
  folders: Folder[];
  messages: Message[];
  selectedMessageId: string | null;
  activeAccountId: string | null;
  activeFolderId: string | null;
  loadingMessages: boolean;
  loadingFolders: boolean;

  fetchAccounts: () => Promise<void>;
  fetchFolders: (accountId: string) => Promise<void>;
  fetchMessages: (
    folderId: string,
    limit?: number,
    offset?: number,
  ) => Promise<void>;
  setActiveAccount: (accountId: string) => Promise<void>;
  setActiveFolder: (folderId: string) => Promise<void>;
  setSelectedMessage: (messageId: string | null) => void;
  syncAccount: (accountId: string) => Promise<void>;
}

export const useMailStore = create<MailState>((set, get) => ({
  accounts: [],
  folders: [],
  messages: [],
  selectedMessageId: null,
  activeAccountId: null,
  activeFolderId: null,
  loadingMessages: false,
  loadingFolders: false,

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
    // Auto-select inbox folder
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

  setSelectedMessage: (messageId: string | null) => {
    set({ selectedMessageId: messageId });
  },

  syncAccount: async (accountId: string) => {
    await startSync(accountId);
    const { activeFolderId } = get();
    await get().fetchFolders(accountId);
    if (activeFolderId) {
      await get().fetchMessages(activeFolderId);
    }
  },
}));
