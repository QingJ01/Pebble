import { create } from "zustand";
import { isAllAccountsFolderId } from "@/lib/folderAggregation";

/**
 * A thread row is identified by (account_id, thread_id): thread ids are only
 * unique within one account, so the unified view can show two accounts'
 * threads with the same id side by side.
 */
export interface ThreadKey {
  thread_id: string;
  account_id?: string | null;
}

/** Whether `thread` is the selected thread (account-aware when known). */
export function isThreadSelected(
  state: Pick<MailState, "selectedThreadId" | "selectedThreadAccountId">,
  thread: ThreadKey,
): boolean {
  if (state.selectedThreadId === null || thread.thread_id !== state.selectedThreadId) return false;
  return !state.selectedThreadAccountId
    || !thread.account_id
    || thread.account_id === state.selectedThreadAccountId;
}

export interface MailState {
  // ─── UI State ──────────────────────────────────────────────────────────────
  activeAccountId: string | null;
  activeFolderId: string | null;
  /**
   * Unified view only: restrict the merged list to one account (null = all).
   * Cleared whenever the view leaves the unified ("all:*") folders.
   */
  unifiedAccountFilter: string | null;
  selectedMessageId: string | null;
  selectedThreadId: string | null;
  /** Account owning the selected thread (null when unknown). */
  selectedThreadAccountId: string | null;
  threadView: boolean;
  /** Show only unread messages / threads with unread messages. */
  unreadOnly: boolean;
  // ─── Batch Selection ───────────────────────────────────────────────────────
  selectedMessageIds: Set<string>;
  batchMode: boolean;

  setActiveAccountId: (accountId: string | null) => void;
  setActiveFolderId: (folderId: string | null) => void;
  setUnifiedAccountFilter: (accountId: string | null) => void;
  setSelectedMessage: (messageId: string | null) => void;
  setSelectedThreadId: (threadId: string | null, accountId?: string | null) => void;
  /** Select a thread row by its (account_id, thread_id) key. */
  selectThread: (thread: ThreadKey | null) => void;
  toggleThreadView: () => void;
  toggleUnreadOnly: () => void;
  toggleBatchMode: () => void;
  toggleMessageSelection: (messageId: string) => void;
  selectAllMessages: (messageIds: string[]) => void;
  clearSelection: () => void;
}

export const useMailStore = create<MailState>((set, get) => ({
  activeAccountId: null,
  activeFolderId: null,
  unifiedAccountFilter: null,
  selectedMessageId: null,
  selectedThreadId: null,
  selectedThreadAccountId: null,
  threadView: false,
  unreadOnly: false,
  selectedMessageIds: new Set<string>(),
  batchMode: false,

  setActiveAccountId: (accountId) => {
    set({
      activeAccountId: accountId,
      activeFolderId: null,
      unifiedAccountFilter: null,
      selectedMessageId: null,
      selectedThreadId: null,
      selectedThreadAccountId: null,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
  },

  setActiveFolderId: (folderId) => {
    set({
      activeFolderId: folderId,
      ...(isAllAccountsFolderId(folderId) ? {} : { unifiedAccountFilter: null }),
      selectedMessageId: null,
      selectedThreadId: null,
      selectedThreadAccountId: null,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
  },

  setUnifiedAccountFilter: (accountId) => {
    if (get().unifiedAccountFilter === accountId) return;
    set({
      unifiedAccountFilter: accountId,
      selectedMessageId: null,
      selectedThreadId: null,
      selectedThreadAccountId: null,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
  },

  setSelectedMessage: (messageId) => {
    set({ selectedMessageId: messageId });
  },

  setSelectedThreadId: (threadId, accountId = null) => {
    set({ selectedThreadId: threadId, selectedThreadAccountId: threadId ? accountId : null });
  },

  selectThread: (thread) => {
    set({
      selectedThreadId: thread?.thread_id ?? null,
      selectedThreadAccountId: thread?.account_id ?? null,
    });
  },

  toggleThreadView: () => {
    set({
      threadView: !get().threadView,
      selectedThreadId: null,
      selectedThreadAccountId: null,
      selectedMessageId: null,
      selectedMessageIds: new Set(),
      batchMode: false,
    });
  },

  toggleUnreadOnly: () => {
    set({
      unreadOnly: !get().unreadOnly,
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
