import type { Command } from "@/stores/command.store";
import { useUIStore } from "@/stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import { useKanbanStore } from "@/stores/kanban.store";
import { useToastStore } from "@/stores/toast.store";
import { updateMessageFlags, archiveMessage } from "@/lib/api";
import type { MessageSummary } from "@/lib/api";
import { queryClient } from "@/lib/query-client";

const NOTIFICATIONS_KEY = "pebble-notifications-enabled";

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
        const entries = queryClient.getQueriesData<MessageSummary[]>({ queryKey: ["messages"] });
        let msg: MessageSummary | undefined;
        for (const [, data] of entries) {
          msg = data?.find((m) => m.id === selectedMessageId);
          if (msg) break;
        }
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
      execute: () => useUIStore.getState().openCompose("new"),
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
        queryClient.setQueriesData<MessageSummary[]>({ queryKey: ["messages"] }, (old) => old?.filter((m) => m.id !== id));
        useMailStore.getState().setSelectedMessage(null);
        try {
          const result = await archiveMessage(id);
          if (result === "skipped") return;
          queryClient.invalidateQueries({ queryKey: ["messages"] });
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
        const current = localStorage.getItem(NOTIFICATIONS_KEY) === "true";
        const next = !current;
        localStorage.setItem(NOTIFICATIONS_KEY, String(next));
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          await invoke("set_notifications_enabled", { enabled: next });
        } catch {
          // backend may not have this command yet — localStorage is the source of truth
        }
      },
    },
  ];
}
