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
  kanbanState: {
    addCard: vi.fn(),
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

vi.mock("@/stores/kanban.store", () => ({
  useKanbanStore: {
    getState: () => mocks.kanbanState,
  },
}));

vi.mock("@/stores/toast.store", () => ({
  useToastStore: {
    getState: () => mocks.toastState,
  },
}));

vi.mock("@/lib/query-client", () => ({
  queryClient: mocks.queryClient,
}));

vi.mock("@/lib/api", () => ({
  updateMessageFlags: mocks.updateMessageFlags,
  archiveMessage: mocks.archiveMessage,
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

  it("refreshes folder unread counts after archive", async () => {
    await command("mail:archive").execute();

    expect(mocks.archiveMessage).toHaveBeenCalledWith("message-1");
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });

  it("does not refresh folder unread counts for star-only changes", async () => {
    await command("mail:star").execute();

    expect(mocks.updateMessageFlags).toHaveBeenCalledWith("message-1", undefined, true);
    expect(mocks.queryClient.invalidateQueries).not.toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });
});
