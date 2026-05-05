import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_BINDINGS, useShortcutStore } from "../../src/stores/shortcut.store";
import { useMailStore } from "../../src/stores/mail.store";

const mocks = vi.hoisted(() => ({
  queryClient: {
    invalidateQueries: vi.fn(),
    getQueriesData: vi.fn(() => []),
  },
  archiveMessage: vi.fn(),
  updateMessageFlags: vi.fn(),
  getMessage: vi.fn(),
  patchMessagesCache: vi.fn(),
  readFirstCachedMessages: vi.fn(() => []),
}));

vi.mock("../../src/lib/query-client", () => ({
  queryClient: mocks.queryClient,
}));

vi.mock("../../src/lib/api", () => ({
  archiveMessage: mocks.archiveMessage,
  updateMessageFlags: mocks.updateMessageFlags,
  getMessage: mocks.getMessage,
}));

vi.mock("../../src/hooks/queries", () => ({
  patchMessagesCache: mocks.patchMessagesCache,
  readFirstCachedMessages: mocks.readFirstCachedMessages,
}));

vi.mock("../../src/lib/i18n", () => ({
  default: {
    t: (_key: string, fallback: string) => fallback,
  },
}));

import { useKeyboard } from "../../src/hooks/useKeyboard";

describe("useKeyboard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.archiveMessage.mockResolvedValue("archived");
    useShortcutStore.setState({
      bindings: { ...DEFAULT_BINDINGS, "archive-message": "E" },
      recording: null,
    });
    useMailStore.setState({
      selectedMessageId: "message-1",
      selectedThreadId: null,
      threadView: false,
    });
  });

  it("refreshes folder unread counts after the archive shortcut succeeds", async () => {
    renderHook(() => useKeyboard());

    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "e", bubbles: true }));
    });

    await waitFor(() => expect(mocks.archiveMessage).toHaveBeenCalledWith("message-1"));
    expect(mocks.queryClient.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });
});
