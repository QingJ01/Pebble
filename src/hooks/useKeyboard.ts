import { useEffect, useRef } from "react";
import { eventToKeyString, useShortcutStore } from "@/stores/shortcut.store";
import { useCommandStore } from "@/stores/command.store";
import { useUIStore } from "@/stores/ui.store";
import { isComposeDirty, useComposeStore } from "@/stores/compose.store";
import { isThreadSelected, useMailStore } from "@/stores/mail.store";
import { toastError, useToastStore } from "@/stores/toast.store";
import { useConfirmStore } from "@/stores/confirm.store";
import { updateMessageFlags, getMessage } from "@/lib/api";
import { queryClient } from "@/lib/query-client";
import type { Folder, MessageSummary, ThreadSummary } from "@/lib/api";
import { findCachedMessage, patchMessagesCache, readFirstCachedMessages } from "@/hooks/queries";
import { getActiveMessages, getActiveThreads } from "@/lib/activeMailList";
import { runMailAction, trashConfirmCopy, type MailActionTarget } from "@/hooks/mutations/mailActions";
import { hasModalOverlay, hasOverlay } from "@/lib/overlayStack";
import { allAccountsFolderId, type FolderRole } from "@/lib/folderAggregation";
import { VIM_BINDINGS, VIM_PAGE_STEP, VIM_PREFIXES, VIM_SEQUENCE_TIMEOUT_MS } from "@/lib/vimKeys";
import i18n from "@/lib/i18n";

export { eventToKeyString };

const TEXT_ENTRY_SELECTOR =
  'input, textarea, select, [contenteditable=""], [contenteditable="true"], [role="textbox"], [role="combobox"], [role="spinbutton"]';
const INTERACTIVE_SELECTOR = [
  "button", "a[href]", "summary", "[role='button']", "[role='link']", "[role='menuitem']",
  "[role='menuitemcheckbox']", "[role='menuitemradio']", "[role='option']", "[role='tab']",
  "[role='checkbox']", "[role='switch']", "[role='radio']", "[role='slider']", "[role='treeitem']",
].join(", ");
/** Containers whose rows keep letter shortcuts (J/K/S/E…) while focused. */
const SHORTCUT_SCOPE_SELECTOR = "[data-shortcut-scope]";

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || Boolean(target.closest(TEXT_ENTRY_SELECTOR));
}

/**
 * Single-key shortcuts (no Ctrl/⌘/Alt) only run when focus is on the page
 * itself or on a mail-list row; on buttons, links, menus, tabs and other
 * controls the key belongs to that control (Enter/Space activate it).
 */
function allowsPlainShortcut(target: EventTarget | null, key: string): boolean {
  if (key === "Escape") return true;
  if (!(target instanceof Element)) return true;
  const control = target.closest(INTERACTIVE_SELECTOR);
  if (!control) return true;
  if (control.closest(SHORTCUT_SCOPE_SELECTOR) && control.getAttribute("role") === "option") {
    return key !== "Enter" && key !== " ";
  }
  return false;
}

/** Read the first matching cached messages from React Query */
function getCachedMessages(): MessageSummary[] {
  return getActiveMessages() ?? readFirstCachedMessages(queryClient);
}

/** Read the first matching cached threads from React Query */
function getCachedThreads(): ThreadSummary[] {
  const active = getActiveThreads();
  if (active) return active;
  const entries = queryClient.getQueriesData<ThreadSummary[]>({ queryKey: ["threads"] });
  for (const [, data] of entries) {
    if (data && data.length > 0) return data;
  }
  return [];
}

async function archiveSelected(messageId: string) {
  let target: { id: string; account_id: string; is_draft?: boolean; remote_id?: string } | undefined =
    findCachedMessage(queryClient, (m) => m.id === messageId);
  if (!target) {
    try {
      target = (await getMessage(messageId)) ?? undefined;
    } catch {
      target = undefined;
    }
  }
  if (!target) return;
  await runMailAction(queryClient, "archive", [target], {
    onRemoved: () => {
      if (useMailStore.getState().selectedMessageId === messageId) {
        useMailStore.getState().setSelectedMessage(null);
      }
    },
    onFailed: () => {
      if (useMailStore.getState().selectedMessageId === null) {
        useMailStore.getState().setSelectedMessage(messageId);
      }
    },
  });
}

const confirmTrash = (plan: Parameters<typeof trashConfirmCopy>[0]) =>
  useConfirmStore.getState().confirm({ ...trashConfirmCopy(plan), destructive: true });

/** Selected messages in bulk-selection mode, or null when nothing is selected */
function bulkTargets(): MailActionTarget[] | null {
  const { batchMode, selectedMessageIds } = useMailStore.getState();
  if (!batchMode || selectedMessageIds.size === 0) return null;
  const messages = getCachedMessages();
  return messages.filter((m) => selectedMessageIds.has(m.id));
}

/** dd / Delete / e on a bulk selection: same action and confirmation as the batch toolbar */
async function runBulkAction(kind: "archive" | "trash", targets: MailActionTarget[]) {
  const result = await runMailAction(queryClient, kind, targets, {
    confirm: confirmTrash,
    onRemoved: (ids) => {
      const current = useMailStore.getState().selectedMessageId;
      if (current && ids.includes(current)) useMailStore.getState().setSelectedMessage(null);
    },
  });
  // Keep the messages that failed selected so they can be retried
  if (result.ok) {
    if (result.failedIds?.length) useMailStore.getState().selectAllMessages(result.failedIds);
    else useMailStore.getState().clearSelection();
  }
}

/**
 * Delete / dd: move the selected message to Trash and keep reading with the
 * next one. Undo comes from the shared mail-action toast; anything that can't
 * be undone (already in Trash, local-only drafts) asks first.
 */
async function trashSelected(messageId: string) {
  const messages = getCachedMessages();
  const idx = messages.findIndex((m) => m.id === messageId);
  const neighbour = idx >= 0 ? (messages[idx + 1] ?? messages[idx - 1]) : undefined;
  const target: MailActionTarget | undefined =
    findCachedMessage(queryClient, (m) => m.id === messageId) ?? (await getMessage(messageId).catch(() => null)) ?? undefined;
  if (!target) return;
  await runMailAction(queryClient, "trash", [target], {
    confirm: confirmTrash,
    onRemoved: () => {
      if (useMailStore.getState().selectedMessageId === messageId) {
        useMailStore.getState().setSelectedMessage(neighbour?.id ?? null);
      }
    },
    onFailed: () => useMailStore.getState().setSelectedMessage(messageId),
  });
}

/** u: run the Undo of the most recent toast that offers one */
function undoLastAction() {
  const undoLabel = i18n.t("common.undo", "Undo");
  const toast = [...useToastStore.getState().toasts].reverse().find((t) => !t.exiting && t.action?.label === undoLabel);
  if (!toast?.action) {
    useToastStore.getState().addToast({
      message: i18n.t("shortcuts.vim.nothingToUndo", "Nothing to undo"),
      type: "info",
    });
    return;
  }
  useToastStore.getState().removeToast(toast.id);
  toast.action.onClick();
}

/** All folders currently held in the React Query cache, across accounts */
function getCachedFolders(): Folder[] {
  return queryClient
    .getQueriesData<Folder[]>({ queryKey: ["folders"] })
    .flatMap(([, data]) => (Array.isArray(data) ? data : []));
}

/** Select the message (or thread) at `index` of the loaded list, clamped */
function selectIndex(index: (length: number, current: number) => number) {
  const state = useMailStore.getState();
  if (state.threadView) {
    const threads = getCachedThreads();
    if (threads.length === 0) return;
    const current = threads.findIndex((t) => isThreadSelected(state, t));
    state.selectThread(threads[Math.min(Math.max(index(threads.length, current), 0), threads.length - 1)]);
  } else {
    const messages = getCachedMessages();
    if (messages.length === 0) return;
    const current = messages.findIndex((m) => m.id === state.selectedMessageId);
    state.setSelectedMessage(messages[Math.min(Math.max(index(messages.length, current), 0), messages.length - 1)].id);
  }
}

function closeSelectedMessage() {
  const state = useMailStore.getState();
  if (state.threadView) state.selectThread(null);
  else state.setSelectedMessage(null);
}

/** x: add or remove the current message from the bulk selection */
function toggleCurrentInSelection() {
  const state = useMailStore.getState();
  if (state.threadView || !state.selectedMessageId) return;
  if (!state.batchMode) state.toggleBatchMode();
  useMailStore.getState().toggleMessageSelection(state.selectedMessageId);
}

/**
 * Open the system folder with this role for the active account, or the
 * combined folder when viewing all accounts. Respects unsaved compose drafts
 * the same way a sidebar click does.
 */
function goToFolderRole(role: FolderRole) {
  const { activeAccountId } = useMailStore.getState();
  const folderId = activeAccountId
    ? getCachedFolders().find((f) => f.account_id === activeAccountId && f.role === role)?.id
    : allAccountsFolderId(role);
  if (!folderId) return;

  if (isComposeDirty()) {
    useComposeStore.setState({
      showComposeLeaveConfirm: true,
      pendingView: "inbox",
      pendingNavigation: () => useMailStore.getState().setActiveFolderId(folderId),
    });
    return;
  }
  useUIStore.getState().setActiveView("inbox");
  useMailStore.getState().setActiveFolderId(folderId);
}

/** Execute a shortcut action. Shared by rebindable shortcuts and vim keys. */
function runAction(actionId: string) {
  switch (actionId) {
    case "close-modal":
      if (useCommandStore.getState().isOpen) {
        useCommandStore.getState().close();
      } else if (useUIStore.getState().activeView === "compose") {
        useComposeStore.getState().closeCompose();
      } else if (useUIStore.getState().activeView === "search") {
        useUIStore.getState().setActiveView("inbox");
      } else if (useMailStore.getState().batchMode) {
        useMailStore.getState().toggleBatchMode();
      }
      break;
    case "toggle-view-inbox":
      useUIStore.getState().setActiveView("inbox");
      break;
    case "toggle-view-kanban":
      useUIStore.getState().setActiveView("kanban");
      break;
    case "toggle-star": {
      const { selectedMessageId } = useMailStore.getState();
      if (selectedMessageId) {
        const messages = getCachedMessages();
        const msg = messages.find((m) => m.id === selectedMessageId);
        if (msg) {
          const newStarred = !msg.is_starred;
          // Optimistic update in React Query cache
          patchMessagesCache(queryClient, (page) =>
            page.map((m) =>
              m.id === selectedMessageId ? { ...m, is_starred: newStarred } : m,
            ),
          );
          updateMessageFlags(selectedMessageId, undefined, newStarred)
            .then(() => {
              queryClient.invalidateQueries({ queryKey: ["messages"] });
              queryClient.invalidateQueries({ queryKey: ["starred-messages"] });
            })
            .catch((error) => {
              // Rollback on error
              patchMessagesCache(queryClient, (page) =>
                page.map((m) =>
                  m.id === selectedMessageId ? { ...m, is_starred: !newStarred } : m,
                ),
              );
              toastError(error, newStarred ? "messageActions.starFailed" : "messageActions.unstarFailed");
            });
        }
      }
      break;
    }
    case "archive-message": {
      const bulk = bulkTargets();
      if (bulk) {
        void runBulkAction("archive", bulk);
        break;
      }
      const { selectedMessageId } = useMailStore.getState();
      if (selectedMessageId) {
        void archiveSelected(selectedMessageId);
      }
      break;
    }
    case "next-message": {
      const state = useMailStore.getState();
      if (state.threadView) {
        const threads = getCachedThreads();
        const idx = threads.findIndex((t) => isThreadSelected(state, t));
        if (idx < threads.length - 1) {
          state.selectThread(threads[idx + 1]);
        }
      } else {
        const messages = getCachedMessages();
        const idx = messages.findIndex((m) => m.id === state.selectedMessageId);
        if (idx < messages.length - 1) {
          state.setSelectedMessage(messages[idx + 1].id);
        }
      }
      break;
    }
    case "prev-message": {
      const state = useMailStore.getState();
      if (state.threadView) {
        const threads = getCachedThreads();
        const idx = threads.findIndex((t) => isThreadSelected(state, t));
        if (idx > 0) {
          state.selectThread(threads[idx - 1]);
        }
      } else {
        const messages = getCachedMessages();
        const idx = messages.findIndex((m) => m.id === state.selectedMessageId);
        if (idx > 0) {
          state.setSelectedMessage(messages[idx - 1].id);
        }
      }
      break;
    }
    case "compose-new":
      useComposeStore.getState().openCompose("new");
      break;
    case "reply": {
      const { selectedMessageId: selId } = useMailStore.getState();
      if (selId) {
        getMessage(selId).then((msg) => {
          if (msg) useComposeStore.getState().openCompose("reply", msg);
        });
      }
      break;
    }
    case "reply-all": {
      const { selectedMessageId: selId } = useMailStore.getState();
      if (selId) {
        getMessage(selId).then((msg) => {
          if (msg) useComposeStore.getState().openCompose("reply-all", msg);
        });
      }
      break;
    }
    case "forward": {
      const { selectedMessageId: selId } = useMailStore.getState();
      if (selId) {
        getMessage(selId).then((msg) => {
          if (msg) useComposeStore.getState().openCompose("forward", msg);
        });
      }
      break;
    }
    case "focus-search":
      useUIStore.getState().setActiveView("search");
      break;
    case "open-message": {
      const state = useMailStore.getState();
      if (state.threadView) {
        if (!state.selectedThreadId) {
          const threads = getCachedThreads();
          if (threads.length > 0) {
            state.selectThread(threads[0]);
          }
        }
      } else {
        if (!state.selectedMessageId) {
          const messages = getCachedMessages();
          if (messages.length > 0) {
            state.setSelectedMessage(messages[0].id);
          }
        }
      }
      break;
    }
    case "open-search":
      useUIStore.getState().setActiveView("search");
      break;
    case "open-cloud-settings":
      useUIStore.getState().setActiveView("settings");
      break;
    case "toggle-notifications": {
      const { notificationsEnabled, setNotificationsEnabled } = useUIStore.getState();
      setNotificationsEnabled(!notificationsEnabled);
      break;
    }
    case "translate-selection":
      document.dispatchEvent(new CustomEvent("pebble:translate-selection"));
      break;
    case "toggle-bilingual":
      document.dispatchEvent(new CustomEvent("pebble:toggle-bilingual"));
      break;
    case "delete-message": {
      const bulk = bulkTargets();
      if (bulk) {
        void runBulkAction("trash", bulk);
        break;
      }
      const { selectedMessageId } = useMailStore.getState();
      if (selectedMessageId) void trashSelected(selectedMessageId);
      break;
    }
    case "first-message":
      selectIndex(() => 0);
      break;
    case "last-message":
      selectIndex((length) => length - 1);
      break;
    case "page-down":
      selectIndex((_, current) => (current < 0 ? 0 : current + VIM_PAGE_STEP));
      break;
    case "page-up":
      selectIndex((_, current) => (current < 0 ? 0 : current - VIM_PAGE_STEP));
      break;
    case "close-message":
      closeSelectedMessage();
      break;
    case "toggle-select":
      toggleCurrentInSelection();
      break;
    case "undo":
      undoLastAction();
      break;
    case "show-shortcuts":
      useUIStore.getState().setSettingsTab("shortcuts");
      useUIStore.getState().setActiveView("settings");
      break;
    case "go-inbox":
      goToFolderRole("inbox");
      break;
    case "go-sent":
      goToFolderRole("sent");
      break;
    case "go-drafts":
      goToFolderRole("drafts");
      break;
    case "go-archive":
      goToFolderRole("archive");
      break;
    case "go-trash":
      goToFolderRole("trash");
      break;
    default:
      break;
  }
}

export function useKeyboard() {
  // Reverse lookup: keyString (lowercase) -> actionId, rebuilt only when bindings change
  const keyToActionRef = useRef<Map<string, string>>(new Map());

  useEffect(() => {
    const bindings = useShortcutStore.getState().bindings;
    const map = new Map<string, string>();
    for (const [actionId, keys] of Object.entries(bindings)) {
      map.set(keys.toLowerCase(), actionId);
    }
    keyToActionRef.current = map;

    // Also subscribe to future binding changes
    const unsubscribe = useShortcutStore.subscribe((state) => {
      const updated = new Map<string, string>();
      for (const [actionId, keys] of Object.entries(state.bindings)) {
        updated.set(keys.toLowerCase(), actionId);
      }
      keyToActionRef.current = updated;
    });

    return unsubscribe;
  }, []);

  useEffect(() => {
    // First key of a pending vim sequence ("d", "g"), forgotten after a timeout
    let pendingKey: string | null = null;
    let pendingTimer: ReturnType<typeof setTimeout> | undefined;
    const clearPending = () => {
      pendingKey = null;
      if (pendingTimer !== undefined) clearTimeout(pendingTimer);
      pendingTimer = undefined;
    };

    function handleKeyDown(e: KeyboardEvent) {
      // Modal overlays own the keyboard. Non-modal popovers don't block global
      // shortcuts; ones registered on the overlay stack get Escape first (the
      // stack consumes it in the capture phase), so only skip Escape here.
      if (hasModalOverlay() || document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      if (e.key === "Escape" && hasOverlay()) return;
      // Don't interfere with shortcut recording
      if (useShortcutStore.getState().recording) return;

      // A focused widget already handled this key (list navigation, menus…).
      if (e.defaultPrevented) return;
      // A bare modifier press (Shift before G) must not break a sequence
      if (["Control", "Meta", "Shift", "Alt"].includes(e.key)) return;

      const isInput = isTextEntry(e.target);
      const keyString = eventToKeyString(e).toLowerCase();
      const actionId = keyToActionRef.current.get(keyString);

      // Command palette always works even in inputs
      if (actionId === "command-palette") {
        clearPending();
        e.preventDefault();
        useCommandStore.getState().open();
        return;
      }

      // Skip shortcuts while typing, and plain keys meant for a focused control
      const hasPrimaryModifier = e.ctrlKey || e.metaKey || e.altKey;
      if (isInput || (!hasPrimaryModifier && !allowsPlainShortcut(e.target, e.key))) {
        clearPending();
        return;
      }

      if (useShortcutStore.getState().vimMode) {
        const previousKey = pendingKey;
        clearPending();

        if (previousKey) {
          const sequenceAction = VIM_BINDINGS[`${previousKey} ${keyString}`];
          if (sequenceAction) {
            e.preventDefault();
            runAction(sequenceAction);
            return;
          }
          // Not a known sequence: handle this key on its own below
        }

        // User bindings take priority over vim keys
        if (!actionId) {
          if (VIM_PREFIXES.has(keyString)) {
            e.preventDefault();
            pendingKey = keyString;
            pendingTimer = setTimeout(clearPending, VIM_SEQUENCE_TIMEOUT_MS);
            return;
          }
          const vimAction = VIM_BINDINGS[keyString];
          if (vimAction) {
            e.preventDefault();
            runAction(vimAction);
            return;
          }
        }
      }

      if (!actionId) return;
      e.preventDefault();
      runAction(actionId);
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => {
      clearPending();
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, []);
}
