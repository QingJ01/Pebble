import type { Command } from "@/stores/command.store";
import { useUIStore } from "@/stores/ui.store";
import { useComposeStore } from "@/stores/compose.store";
import { useMailStore } from "@/stores/mail.store";
import { addKanbanCard } from "@/hooks/mutations/useKanbanMutations";
import { toastError, useToastStore } from "@/stores/toast.store";
import { updateMessageFlags, getMessage } from "@/lib/api";
import { queryClient } from "@/lib/query-client";
import { findCachedMessage } from "@/hooks/queries";
import { runMailAction } from "@/hooks/mutations/mailActions";
import { useShortcutStore } from "@/stores/shortcut.store";

/**
 * Command id → keyboard-shortcut action id (src/stores/shortcut.store.ts).
 * The palette shows the user's current binding, not a hardcoded default.
 */
export const COMMAND_SHORTCUT_ACTIONS: Record<string, string> = {
  "nav:inbox": "toggle-view-inbox",
  "nav:kanban": "toggle-view-kanban",
  "nav:search": "open-search",
  "mail:star": "toggle-star",
  "mail:archive": "archive-message",
  "mail:compose": "compose-new",
  "settings:toggle-notifications": "toggle-notifications",
};

/** Current raw binding for a command ("Ctrl+Shift+I"); format with formatShortcut(). */
export function commandShortcut(commandId: string): string | undefined {
  const action = COMMAND_SHORTCUT_ACTIONS[commandId];
  return action ? useShortcutStore.getState().bindings[action] || undefined : undefined;
}

async function refreshAfterReadChange() {
  queryClient.invalidateQueries({ queryKey: ["messages"] });
  queryClient.invalidateQueries({ queryKey: ["threads"] });
  queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
}

export function buildCommands(t: (key: string, defaultValue: string) => string): Command[] {
  return [
    // Navigation
    {
      id: "nav:inbox",
      name: t("commands.goToInbox", "Go to Inbox"),
      shortcut: commandShortcut("nav:inbox"),
      category: t("commands.navigation", "Navigation"),
      execute: () => useUIStore.getState().setActiveView("inbox"),
    },
    {
      id: "nav:kanban",
      name: t("commands.goToKanban", "Go to Kanban"),
      shortcut: commandShortcut("nav:kanban"),
      category: t("commands.navigation", "Navigation"),
      execute: () => useUIStore.getState().setActiveView("kanban"),
    },
    {
      id: "nav:contacts",
      name: t("commands.goToContacts", "Go to Contacts"),
      category: t("commands.navigation", "Navigation"),
      execute: () => useUIStore.getState().setActiveView("contacts"),
    },
    {
      id: "nav:settings",
      name: t("commands.goToSettings", "Go to Settings"),
      category: t("commands.navigation", "Navigation"),
      execute: () => useUIStore.getState().setActiveView("settings"),
    },
    {
      id: "nav:search",
      name: t("commands.openSearch", "Open Search"),
      shortcut: commandShortcut("nav:search"),
      category: t("commands.navigation", "Navigation"),
      execute: () => useUIStore.getState().setActiveView("search"),
    },
    // View
    {
      id: "view:toggle-sidebar",
      name: t("commands.toggleSidebar", "Toggle Sidebar"),
      category: t("commands.view", "View"),
      execute: () => useUIStore.getState().toggleSidebar(),
    },
    // Mail actions
    {
      id: "mail:mark-read",
      name: t("commands.markAsRead", "Mark as Read"),
      category: t("commands.mail", "Mail"),
      execute: async () => {
        const id = useMailStore.getState().selectedMessageId;
        if (!id) return;
        try {
          await updateMessageFlags(id, true);
          await refreshAfterReadChange();
        } catch (error) {
          toastError(error, "messageActions.updateReadFailed");
        }
      },
    },
    {
      id: "mail:mark-unread",
      name: t("commands.markAsUnread", "Mark as Unread"),
      category: t("commands.mail", "Mail"),
      execute: async () => {
        const id = useMailStore.getState().selectedMessageId;
        if (!id) return;
        try {
          await updateMessageFlags(id, false);
          await refreshAfterReadChange();
        } catch (error) {
          toastError(error, "messageActions.updateReadFailed");
        }
      },
    },
    {
      id: "mail:star",
      name: t("commands.toggleStar", "Toggle Star"),
      shortcut: commandShortcut("mail:star"),
      category: t("commands.mail", "Mail"),
      execute: async () => {
        const { selectedMessageId } = useMailStore.getState();
        if (!selectedMessageId) return;
        const msg = findCachedMessage(queryClient, (m) => m.id === selectedMessageId);
        if (msg) {
          try {
            await updateMessageFlags(selectedMessageId, undefined, !msg.is_starred);
            queryClient.invalidateQueries({ queryKey: ["messages"] });
            queryClient.invalidateQueries({ queryKey: ["starred-messages"] });
          } catch (error) {
            toastError(error, msg.is_starred ? "messageActions.unstarFailed" : "messageActions.starFailed");
          }
        }
      },
    },
    {
      id: "mail:compose",
      name: t("commands.composeNew", "Compose New Message"),
      shortcut: commandShortcut("mail:compose"),
      category: t("commands.mail", "Mail"),
      execute: () => useComposeStore.getState().openCompose("new"),
    },
    // Settings
    {
      id: "mail:archive",
      name: t("commands.archiveMessage", "Archive Message"),
      shortcut: commandShortcut("mail:archive"),
      category: t("commands.mail", "Mail"),
      execute: async () => {
        const id = useMailStore.getState().selectedMessageId;
        if (!id) return;
        const target = findCachedMessage(queryClient, (m) => m.id === id) ?? (await getMessage(id).catch(() => null));
        if (!target) return;
        await runMailAction(queryClient, "archive", [target], {
          onRemoved: () => {
            if (useMailStore.getState().selectedMessageId === id) useMailStore.getState().setSelectedMessage(null);
          },
        });
      },
    },
    {
      id: "mail:add-to-kanban",
      name: t("commands.addToKanban", "Add to Kanban"),
      category: t("commands.mail", "Mail"),
      execute: async () => {
        const id = useMailStore.getState().selectedMessageId;
        if (!id) return;
        try {
          await addKanbanCard(id, "todo", queryClient);
          useToastStore.getState().addToast({ message: t("messageActions.kanbanSuccess", "Added to kanban board"), type: "success" });
        } catch (error) {
          toastError(error, "messageActions.kanbanFailed");
        }
      },
    },
    // Settings
    {
      id: "settings:toggle-notifications",
      name: t("commands.toggleNotifications", "Toggle Notifications"),
      shortcut: commandShortcut("settings:toggle-notifications"),
      category: t("commands.settings", "Settings"),
      execute: async () => {
        const { notificationsEnabled, setNotificationsEnabled } = useUIStore.getState();
        setNotificationsEnabled(!notificationsEnabled);
      },
    },
  ];
}
