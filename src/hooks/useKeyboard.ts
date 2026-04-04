import { useEffect } from "react";
import { useShortcutStore } from "@/stores/shortcut.store";
import { useCommandStore } from "@/stores/command.store";
import { useUIStore } from "@/stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import { updateMessageFlags } from "@/lib/api";

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

export function useKeyboard() {
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Don't interfere with shortcut recording
      if (useShortcutStore.getState().recording) return;

      const target = e.target as HTMLElement;
      const isInput =
        target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable;
      const keyString = eventToKeyString(e);
      const bindings = useShortcutStore.getState().bindings;

      // Build reverse lookup
      const actionForKey = Object.entries(bindings).find(
        ([, keys]) => keys.toLowerCase() === keyString.toLowerCase(),
      );

      if (!actionForKey) return;
      const [actionId] = actionForKey;

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
          }
          break;
        case "toggle-view-inbox":
          useUIStore.getState().setActiveView("inbox");
          break;
        case "toggle-view-kanban":
          useUIStore.getState().setActiveView("kanban");
          break;
        case "toggle-star": {
          const { selectedMessageId, messages } = useMailStore.getState();
          if (selectedMessageId) {
            const msg = messages.find((m) => m.id === selectedMessageId);
            if (msg) updateMessageFlags(selectedMessageId, undefined, !msg.is_starred);
          }
          break;
        }
        case "next-message": {
          const { selectedMessageId, messages } = useMailStore.getState();
          const idx = messages.findIndex((m) => m.id === selectedMessageId);
          if (idx < messages.length - 1) {
            useMailStore.getState().setSelectedMessage(messages[idx + 1].id);
          }
          break;
        }
        case "prev-message": {
          const { selectedMessageId, messages } = useMailStore.getState();
          const idx = messages.findIndex((m) => m.id === selectedMessageId);
          if (idx > 0) {
            useMailStore.getState().setSelectedMessage(messages[idx - 1].id);
          }
          break;
        }
        case "compose-new":
          useUIStore.getState().openCompose("new");
          break;
        case "reply": {
          const { selectedMessageId: selId, messages: msgs } = useMailStore.getState();
          if (selId) {
            const msg = msgs.find((m) => m.id === selId);
            if (msg) useUIStore.getState().openCompose("reply", msg);
          }
          break;
        }
        case "open-message":
          // Message detail shows automatically when selected; Enter is a no-op confirmation
          break;
        default:
          break;
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, []);
}
