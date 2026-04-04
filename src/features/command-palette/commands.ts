import type { Command } from "@/stores/command.store";
import { useUIStore } from "@/stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import { updateMessageFlags } from "@/lib/api";

const NOTIFICATIONS_KEY = "pebble-notifications-enabled";

export function buildCommands(): Command[] {
  return [
    // Navigation
    {
      id: "nav:inbox",
      name: "Go to Inbox",
      shortcut: "Ctrl+Shift+I",
      category: "Navigation",
      execute: () => useUIStore.getState().setActiveView("inbox"),
    },
    {
      id: "nav:kanban",
      name: "Go to Kanban",
      shortcut: "Ctrl+Shift+K",
      category: "Navigation",
      execute: () => useUIStore.getState().setActiveView("kanban"),
    },
    {
      id: "nav:settings",
      name: "Go to Settings",
      category: "Navigation",
      execute: () => useUIStore.getState().setActiveView("settings"),
    },
    {
      id: "nav:search",
      name: "Open Search",
      shortcut: "Ctrl+Shift+F",
      category: "Navigation",
      execute: () => useUIStore.getState().setActiveView("search"),
    },
    // View
    {
      id: "view:toggle-sidebar",
      name: "Toggle Sidebar",
      category: "View",
      execute: () => useUIStore.getState().toggleSidebar(),
    },
    // Mail actions
    {
      id: "mail:mark-read",
      name: "Mark as Read",
      category: "Mail",
      execute: async () => {
        const id = useMailStore.getState().selectedMessageId;
        if (id) await updateMessageFlags(id, true);
      },
    },
    {
      id: "mail:mark-unread",
      name: "Mark as Unread",
      category: "Mail",
      execute: async () => {
        const id = useMailStore.getState().selectedMessageId;
        if (id) await updateMessageFlags(id, false);
      },
    },
    {
      id: "mail:star",
      name: "Toggle Star",
      shortcut: "S",
      category: "Mail",
      execute: async () => {
        const { selectedMessageId, messages } = useMailStore.getState();
        if (!selectedMessageId) return;
        const msg = messages.find((m) => m.id === selectedMessageId);
        if (msg) await updateMessageFlags(selectedMessageId, undefined, !msg.is_starred);
      },
    },
    {
      id: "mail:compose",
      name: "Compose New Message",
      category: "Mail",
      execute: () => useUIStore.getState().openCompose("new"),
    },
    // Cloud sync
    {
      id: "sync:backup",
      name: "Backup to Cloud",
      shortcut: "Ctrl+Shift+B",
      category: "Cloud Sync",
      execute: () => useUIStore.getState().setActiveView("settings"),
    },
    {
      id: "sync:restore",
      name: "Restore from Cloud",
      category: "Cloud Sync",
      execute: () => useUIStore.getState().setActiveView("settings"),
    },
    // Settings
    {
      id: "settings:toggle-notifications",
      name: "Toggle Notifications",
      category: "Settings",
      execute: () => {
        const current = localStorage.getItem(NOTIFICATIONS_KEY) === "true";
        localStorage.setItem(NOTIFICATIONS_KEY, String(!current));
      },
    },
  ];
}
