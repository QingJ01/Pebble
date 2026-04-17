import { useEffect, useRef } from "react";
import { useShortcutStore } from "@/stores/shortcut.store";
import { useCommandStore } from "@/stores/command.store";
import { useUIStore } from "@/stores/ui.store";
import { useComposeStore, isComposeDirty } from "@/stores/compose.store";
import { useConfirmStore } from "@/stores/confirm.store";
import { useMailStore } from "@/stores/mail.store";
import { useToastStore } from "@/stores/toast.store";
import { updateMessageFlags, archiveMessage, getMessage } from "@/lib/api";
import { queryClient } from "@/lib/query-client";
import type { MessageSummary, ThreadSummary } from "@/lib/api";
import { patchMessagesCache, readFirstCachedMessages } from "@/hooks/queries";
import i18n from "@/lib/i18n";

function eventToKeyString(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (e.shiftKey) parts.push("Shift");
  if (e.altKey) parts.push("Alt");
  const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  if (!["Control", "Meta", "Shift", "Alt"].includes(e.key)) {
    parts.push(key);
  }
  return parts.join("+");
}

export { eventToKeyString };

/** Read the first matching cached messages from React Query */
function getCachedMessages(): MessageSummary[] {
  return readFirstCachedMessages(queryClient);
}

/** Read the first matching cached threads from React Query */
function getCachedThreads(): ThreadSummary[] {
  const entries = queryClient.getQueriesData<ThreadSummary[]>({ queryKey: ["threads"] });
  for (const [, data] of entries) {
    if (data && data.length > 0) return data;
  }
  return [];
}

async function confirmLeaveCompose(): Promise<boolean> {
  if (!isComposeDirty()) return true;
  return useConfirmStore.getState().confirm({
    title: i18n.t("compose.discardDraft", "Discard draft"),
    message: i18n.t("compose.discardDraftConfirm", "You have an unsaved draft. Discard and leave?"),
    destructive: true,
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
      // Don't interfere with shortcut recording
      if (useShortcutStore.getState().recording) return;

      const target = e.target as HTMLElement;
      const isInput =
        target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable;
      const keyString = eventToKeyString(e);

      const actionId = keyToActionRef.current.get(keyString.toLowerCase());

      if (!actionId) return;

      // Command palette always works even in inputs
      if (actionId === "command-palette") {
        e.preventDefault();
        useCommandStore.getState().open();
        return;
      }

      // Skip single-key shortcuts when in inputs
      if (isInput) return;

      e.preventDefault();

      // Execute action
      switch (actionId) {
        case "close-modal":
          if (useCommandStore.getState().isOpen) {
            useCommandStore.getState().close();
          } else if (useUIStore.getState().activeView === "compose") {
            confirmLeaveCompose().then((ok) => { if (ok) useComposeStore.getState().closeCompose(); });
          } else if (useUIStore.getState().activeView === "search") {
            useUIStore.getState().setActiveView("inbox");
          }
          break;
        case "toggle-view-inbox":
          confirmLeaveCompose().then((ok) => { if (ok) useUIStore.getState().setActiveView("inbox"); });
          break;
        case "toggle-view-kanban":
          confirmLeaveCompose().then((ok) => { if (ok) useUIStore.getState().setActiveView("kanban"); });
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
                .then(() => queryClient.invalidateQueries({ queryKey: ["messages"] }))
                .catch(() => {
                  // Rollback on error
                  patchMessagesCache(queryClient, (page) =>
                    page.map((m) =>
                      m.id === selectedMessageId ? { ...m, is_starred: !newStarred } : m,
                    ),
                  );
                });
            }
          }
          break;
        }
        case "archive-message": {
          const { selectedMessageId } = useMailStore.getState();
          if (selectedMessageId) {
            // Optimistic removal from React Query cache
            patchMessagesCache(queryClient, (page) =>
              page.filter((m) => m.id !== selectedMessageId),
            );
            useMailStore.getState().setSelectedMessage(null);
            archiveMessage(selectedMessageId)
              .then((result) => {
                if (result === "skipped") return;
                queryClient.invalidateQueries({ queryKey: ["messages"] });
                queryClient.invalidateQueries({ queryKey: ["threads"] });
                const msg = result === "unarchived" ? i18n.t("messageActions.unarchiveSuccess", "Message moved to inbox") : i18n.t("messageActions.archiveSuccess", "Message archived");
                useToastStore.getState().addToast({ message: msg, type: "success" });
              })
              .catch(() => {
                queryClient.invalidateQueries({ queryKey: ["messages"] });
                useMailStore.getState().setSelectedMessage(selectedMessageId);
                useToastStore.getState().addToast({ message: i18n.t("messageActions.archiveFailed", "Failed to archive"), type: "error" });
              });
          }
          break;
        }
        case "next-message": {
          const state = useMailStore.getState();
          if (state.threadView) {
            const threads = getCachedThreads();
            const idx = threads.findIndex((t) => t.thread_id === state.selectedThreadId);
            if (idx < threads.length - 1) {
              state.setSelectedThreadId(threads[idx + 1].thread_id);
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
            const idx = threads.findIndex((t) => t.thread_id === state.selectedThreadId);
            if (idx > 0) {
              state.setSelectedThreadId(threads[idx - 1].thread_id);
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
          confirmLeaveCompose().then((ok) => { if (ok) useUIStore.getState().setActiveView("search"); });
          break;
        case "open-message": {
          const state = useMailStore.getState();
          if (state.threadView) {
            if (!state.selectedThreadId) {
              const threads = getCachedThreads();
              if (threads.length > 0) {
                state.setSelectedThreadId(threads[0].thread_id);
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
          confirmLeaveCompose().then((ok) => { if (ok) useUIStore.getState().setActiveView("search"); });
          break;
        case "open-cloud-settings":
          confirmLeaveCompose().then((ok) => { if (ok) useUIStore.getState().setActiveView("settings"); });
          break;
        case "toggle-notifications": {
          const key = "pebble-notifications-enabled";
          const cur = localStorage.getItem(key) === "true";
          localStorage.setItem(key, String(!cur));
          break;
        }
        default:
          break;
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, []);
}
