import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  mailState: {
    selectedMessageId: "message-1" as string | null,
    setSelectedMessage: vi.fn(),
  },
  uiState: {
    setActiveView: vi.fn(),
    toggleSidebar: vi.fn(),
    notificationsEnabled: true,
    setNotificationsEnabled: vi.fn(),
  },
  composeState: {
    openCompose: vi.fn(),
  },
  toastState: {
    addToast: vi.fn(),
  },
  queryClient: {
    invalidateQueries: vi.fn(),
  },
  updateMessageFlags: vi.fn(),
  archiveMessage: vi.fn(),
  patchMessagesCache: vi.fn(),
  findCachedMessage: vi.fn(),
  runMailAction: vi.fn(),
  toastError: vi.fn(),
  getMessage: vi.fn(),
}));

vi.mock("@/stores/mail.store", () => ({
  useMailStore: {
    getState: () => mocks.mailState,
  },
}));

vi.mock("@/stores/ui.store", () => ({
  useUIStore: {
    getState: () => mocks.uiState,
  },
}));

vi.mock("@/stores/compose.store", () => ({
  useComposeStore: {
    getState: () => mocks.composeState,
  },
}));

vi.mock("@/stores/toast.store", () => ({
  useToastStore: {
    getState: () => mocks.toastState,
  },
  toastError: mocks.toastError,
}));

vi.mock("@/hooks/mutations/mailActions", () => ({
  runMailAction: mocks.runMailAction,
}));

vi.mock("@/lib/query-client", () => ({
  queryClient: mocks.queryClient,
}));

vi.mock("@/lib/api", () => ({
  updateMessageFlags: mocks.updateMessageFlags,
  archiveMessage: mocks.archiveMessage,
  getMessage: mocks.getMessage,
}));

vi.mock("@/hooks/queries", () => ({
  patchMessagesCache: mocks.patchMessagesCache,
  findCachedMessage: mocks.findCachedMessage,
}));

import { buildCommands } from "@/features/command-palette/commands";

function t(_key: string, defaultValue: string) {
  return defaultValue;
}

function command(id: string) {
  const found = buildCommands(t).find((cmd) => cmd.id === id);
  if (!found) throw new Error(`Missing command ${id}`);
  return found;
}

describe("command palette mail commands", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.mailState.selectedMessageId = "message-1";
    mocks.updateMessageFlags.mockResolvedValue(undefined);
    mocks.archiveMessage.mockResolvedValue("archived");
    mocks.findCachedMessage.mockReturnValue({
      id: "message-1",
      is_starred: false,
    });
  });

  it("refreshes unread-derived queries after mark read", async () => {
    await command("mail:mark-read").execute();

    expect(mocks.updateMessageFlags).toHaveBeenCalledWith("message-1", true);
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["threads"] });
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });

  it("refreshes unread-derived queries after mark unread", async () => {
    await command("mail:mark-unread").execute();

    expect(mocks.updateMessageFlags).toHaveBeenCalledWith("message-1", false);
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["threads"] });
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });

  it("archives through the shared undoable mail action", async () => {
    await command("mail:archive").execute();

    expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient,
      "archive",
      [expect.objectContaining({ id: "message-1" })],
      expect.anything(),
    );
  });

  it("shows the user's current binding instead of a hardcoded shortcut", async () => {
    const { useShortcutStore } = await import("@/stores/shortcut.store");
    useShortcutStore.setState({ bindings: { ...useShortcutStore.getState().bindings, "archive-message": "Shift+A" } });
    expect(command("mail:archive").shortcut).toBe("Shift+A");
    const { commandShortcut } = await import("@/features/command-palette/commands");
    expect(commandShortcut("nav:inbox")).toBe("Ctrl+Shift+I");
  });

  it("shows an error toast when marking read fails", async () => {
    mocks.updateMessageFlags.mockRejectedValueOnce({ kind: "Network", message: "offline" });
    await command("mail:mark-read").execute();
    expect(mocks.toastError).toHaveBeenCalledWith({ kind: "Network", message: "offline" }, "messageActions.updateReadFailed");
  });

  it("does not refresh folder unread counts for star-only changes", async () => {
    await command("mail:star").execute();

    expect(mocks.updateMessageFlags).toHaveBeenCalledWith("message-1", undefined, true);
    expect(mocks.queryClient.invalidateQueries).not.toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });

  it("navigates to contacts from the command palette", async () => {
    await command("nav:contacts").execute();

    expect(mocks.uiState.setActiveView).toHaveBeenCalledWith("contacts");
  });
});
