import type { Command } from "@/stores/command.store";
import { useUIStore } from "@/stores/ui.store";
import { useComposeStore } from "@/stores/compose.store";
import { useMailStore } from "@/stores/mail.store";
import { useKanbanStore } from "@/stores/kanban.store";
import { useToastStore } from "@/stores/toast.store";
import { updateMessageFlags, archiveMessage } from "@/lib/api";
import { queryClient } from "@/lib/query-client";
import { patchMessagesCache, findCachedMessage } from "@/hooks/queries";

export function buildCommands(t: (key: string, defaultValue: string) => string): Command[] {
  return [
    // Navigation
    {
      id: "nav:inbox",
      name: t("commands.goToInbox", "Go to Inbox"),
      shortcut: "Ctrl+Shift+I",
      category: t("commands.navigation", "Navigation"),
      execute: () => useUIStore.getState().setActiveView("inbox"),
    },
    {
      id: "nav:kanban",
      name: t("commands.goToKanban", "Go to Kanban"),
      shortcut: "Ctrl+Shift+K",
      category: t("commands.navigation", "Navigation"),
      execute: () => useUIStore.getState().setActiveView("kanban"),
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
      shortcut: "Ctrl+Shift+F",
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
        if (id) {
          await updateMessageFlags(id, true);
          queryClient.invalidateQueries({ queryKey: ["messages"] });
          queryClient.invalidateQueries({ queryKey: ["threads"] });
          queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
        }
      },
    },
    {
      id: "mail:mark-unread",
      name: t("commands.markAsUnread", "Mark as Unread"),
      category: t("commands.mail", "Mail"),
      execute: async () => {
        const id = useMailStore.getState().selectedMessageId;
        if (id) {
          await updateMessageFlags(id, false);
          queryClient.invalidateQueries({ queryKey: ["messages"] });
          queryClient.invalidateQueries({ queryKey: ["threads"] });
          queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
        }
      },
    },
    {
      id: "mail:star",
      name: t("commands.toggleStar", "Toggle Star"),
      shortcut: "S",
      category: t("commands.mail", "Mail"),
      execute: async () => {
        const { selectedMessageId } = useMailStore.getState();
        if (!selectedMessageId) return;
        const msg = findCachedMessage(queryClient, (m) => m.id === selectedMessageId);
        if (msg) {
          await updateMessageFlags(selectedMessageId, undefined, !msg.is_starred);
          queryClient.invalidateQueries({ queryKey: ["messages"] });
        }
      },
    },
    {
      id: "mail:compose",
      name: t("commands.composeNew", "Compose New Message"),
      category: t("commands.mail", "Mail"),
      execute: () => useComposeStore.getState().openCompose("new"),
    },
    // Settings
    {
      id: "mail:archive",
      name: t("commands.archiveMessage", "Archive Message"),
      shortcut: "E",
      category: t("commands.mail", "Mail"),
      execute: async () => {
        const id = useMailStore.getState().selectedMessageId;
        if (!id) return;
        patchMessagesCache(queryClient, (page) => page.filter((m) => m.id !== id));
        useMailStore.getState().setSelectedMessage(null);
        try {
          const result = await archiveMessage(id);
          if (result === "skipped") return;
          queryClient.invalidateQueries({ queryKey: ["messages"] });
          queryClient.invalidateQueries({ queryKey: ["threads"] });
          queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
          const msg = result === "unarchived" ? t("messageActions.unarchiveSuccess", "Message moved to inbox") : t("messageActions.archiveSuccess", "Message archived");
          useToastStore.getState().addToast({ message: msg, type: "success" });
        } catch {
          queryClient.invalidateQueries({ queryKey: ["messages"] });
          useToastStore.getState().addToast({ message: t("messageActions.archiveFailed", "Failed to archive"), type: "error" });
        }
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
          await useKanbanStore.getState().addCard(id, "todo");
          useToastStore.getState().addToast({ message: t("messageActions.kanbanSuccess", "Added to kanban board"), type: "success" });
        } catch {
          useToastStore.getState().addToast({ message: t("messageActions.kanbanFailed", "Failed to add to kanban"), type: "error" });
        }
      },
    },
    // Settings
    {
      id: "settings:toggle-notifications",
      name: t("commands.toggleNotifications", "Toggle Notifications"),
      category: t("commands.settings", "Settings"),
      execute: async () => {
        const { notificationsEnabled, setNotificationsEnabled } = useUIStore.getState();
        setNotificationsEnabled(!notificationsEnabled);
      },
    },
  ];
}
