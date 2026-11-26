import { useEffect, useRef } from "react";
import { eventToKeyString, useShortcutStore } from "@/stores/shortcut.store";
import { useCommandStore } from "@/stores/command.store";
import { useUIStore } from "@/stores/ui.store";
import { useComposeStore } from "@/stores/compose.store";
import { isThreadSelected, useMailStore } from "@/stores/mail.store";
import { toastError } from "@/stores/toast.store";
import { updateMessageFlags, getMessage } from "@/lib/api";
import { queryClient } from "@/lib/query-client";
import type { MessageSummary, ThreadSummary } from "@/lib/api";
import { findCachedMessage, patchMessagesCache, readFirstCachedMessages } from "@/hooks/queries";
import { getActiveMessages, getActiveThreads } from "@/lib/activeMailList";
import { runMailAction } from "@/hooks/mutations/mailActions";
import { hasModalOverlay, hasOverlay } from "@/lib/overlayStack";

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

      const isInput = isTextEntry(e.target);
      const keyString = eventToKeyString(e);

      const actionId = keyToActionRef.current.get(keyString.toLowerCase());

      if (!actionId) return;

      // Command palette always works even in inputs
      if (actionId === "command-palette") {
        e.preventDefault();
        useCommandStore.getState().open();
        return;
      }

      // Skip shortcuts while typing
      if (isInput) return;
      const hasPrimaryModifier = e.ctrlKey || e.metaKey || e.altKey;
      if (!hasPrimaryModifier && !allowsPlainShortcut(e.target, e.key)) return;

      e.preventDefault();

      // Execute action
      switch (actionId) {
        case "close-modal":
          if (useCommandStore.getState().isOpen) {
            useCommandStore.getState().close();
          } else if (useUIStore.getState().activeView === "compose") {
            useComposeStore.getState().closeCompose();
          } else if (useUIStore.getState().activeView === "search") {
            useUIStore.getState().setActiveView("inbox");
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
        default:
          break;
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, []);
}
