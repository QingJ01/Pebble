import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_BINDINGS, formatShortcut, useShortcutStore } from "../../src/stores/shortcut.store";
import { useMailStore } from "../../src/stores/mail.store";
import { useUIStore } from "../../src/stores/ui.store";
import { useToastStore } from "../../src/stores/toast.store";
import { useCommandStore } from "../../src/stores/command.store";

const mocks = vi.hoisted(() => ({
  queryClient: {
    invalidateQueries: vi.fn(),
    getQueriesData: vi.fn(() => []),
  },
  updateMessageFlags: vi.fn(),
  getMessage: vi.fn(),
  patchMessagesCache: vi.fn(),
  readFirstCachedMessages: vi.fn(() => [] as { id: string }[]),
  findCachedMessage: vi.fn(),
  runMailAction: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("../../src/lib/query-client", () => ({
  queryClient: mocks.queryClient,
}));

vi.mock("../../src/lib/api", () => ({
  updateMessageFlags: mocks.updateMessageFlags,
  getMessage: mocks.getMessage,
}));

vi.mock("../../src/hooks/queries", () => ({
  patchMessagesCache: mocks.patchMessagesCache,
  readFirstCachedMessages: mocks.readFirstCachedMessages,
  findCachedMessage: mocks.findCachedMessage,
}));

vi.mock("../../src/hooks/mutations/mailActions", () => ({
  runMailAction: mocks.runMailAction,
  trashConfirmCopy: () => ({ title: "", message: "" }),
}));

vi.mock("../../src/stores/toast.store", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../src/stores/toast.store")>()),
  toastError: mocks.toastError,
}));

vi.mock("../../src/lib/i18n", () => ({
  default: {
    t: (_key: string, fallback: string) => fallback,
  },
}));

import { useKeyboard } from "../../src/hooks/useKeyboard";
import { clearActiveMailList, setActiveMailList } from "../../src/lib/activeMailList";
import { BATCH_MOVE_EVENT } from "../../src/lib/vimKeys";

function press(target: EventTarget, key: string, init: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

describe("useKeyboard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.runMailAction.mockResolvedValue({ ok: true, count: 1 });
    mocks.findCachedMessage.mockReturnValue({ id: "message-1", account_id: "account-1" });
    mocks.readFirstCachedMessages.mockReturnValue([{ id: "message-1" }, { id: "message-2" }]);
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

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("archives the selected message through the undoable mail action", async () => {
    renderHook(() => useKeyboard());

    press(document.body, "e");

    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient,
      "archive",
      [expect.objectContaining({ id: "message-1" })],
      expect.objectContaining({ onRemoved: expect.any(Function) }),
    ));
  });

  it("lets Enter activate a focused button instead of running open-message", () => {
    renderHook(() => useKeyboard());
    useMailStore.setState({ selectedMessageId: null });
    const button = document.createElement("button");
    document.body.appendChild(button);
    button.focus();

    const event = press(button, "Enter");

    expect(event.defaultPrevented).toBe(false);
    expect(useMailStore.getState().selectedMessageId).toBeNull();
  });

  it.each([
    ["button", () => document.createElement("button")],
    ["link", () => Object.assign(document.createElement("a"), { href: "#x" })],
    ["menu item", () => Object.assign(document.createElement("div"), { role: "menuitem" })],
    ["select", () => document.createElement("select")],
    ["tab", () => { const el = document.createElement("div"); el.setAttribute("role", "tab"); return el; }],
  ])("ignores single-key shortcuts while a %s has focus", (_name, make) => {
    renderHook(() => useKeyboard());
    const el = make();
    if (el.getAttribute("role") === null && el.tagName === "DIV") el.setAttribute("role", "menuitem");
    document.body.appendChild(el);

    press(el, "j");
    press(el, "e");

    expect(useMailStore.getState().selectedMessageId).toBe("message-1");
    expect(mocks.runMailAction).not.toHaveBeenCalled();
  });

  it("still runs single-key shortcuts from the page and from mail-list rows", () => {
    renderHook(() => useKeyboard());

    press(document.body, "j");
    expect(useMailStore.getState().selectedMessageId).toBe("message-2");

    const list = document.createElement("div");
    list.setAttribute("data-shortcut-scope", "list");
    const row = document.createElement("div");
    row.setAttribute("role", "option");
    list.appendChild(row);
    document.body.appendChild(list);

    press(row, "k");
    expect(useMailStore.getState().selectedMessageId).toBe("message-1");
  });

  it("keeps modifier shortcuts working on focused buttons", () => {
    renderHook(() => useKeyboard());
    const button = document.createElement("button");
    document.body.appendChild(button);

    const event = press(button, "K", { ctrlKey: true });

    expect(event.defaultPrevented).toBe(true);
  });

  it("skips keys a focused widget already handled", () => {
    renderHook(() => useKeyboard());
    const event = new KeyboardEvent("keydown", { key: "j", bubbles: true, cancelable: true });
    event.preventDefault();
    act(() => {
      document.body.dispatchEvent(event);
    });
    expect(useMailStore.getState().selectedMessageId).toBe("message-1");
  });
});

describe("formatShortcut", () => {
  it("renders the primary modifier as ⌘ symbols on macOS", () => {
    expect(formatShortcut("Ctrl+Shift+K", true)).toBe("⌘⇧K");
    expect(formatShortcut("Ctrl+K", true)).toBe("⌘K");
  });

  it("keeps Ctrl+ text elsewhere", () => {
    expect(formatShortcut("Ctrl+Shift+K", false)).toBe("Ctrl+Shift+K");
    expect(formatShortcut("Escape", false)).toBe("Esc");
  });
});

describe("delete and vim keys", () => {
  const many = Array.from({ length: 25 }, (_, i) => ({ id: `m${i}`, account_id: "acc-1" }));
  const folders = [
    { id: "inbox-1", account_id: "acc-1", role: "inbox" },
    { id: "sent-1", account_id: "acc-1", role: "sent" },
    { id: "trash-1", account_id: "acc-1", role: "trash" },
  ];

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.runMailAction.mockResolvedValue({ ok: true, count: 1 });
    mocks.readFirstCachedMessages.mockReturnValue(many);
    mocks.findCachedMessage.mockImplementation((_client: unknown, match: (m: { id: string }) => boolean) => many.find(match));
    mocks.queryClient.getQueriesData.mockImplementation(
      ({ queryKey }: { queryKey: string[] }) => (queryKey[0] === "folders" ? [[["folders", "acc-1"], folders]] : []),
    );
    useShortcutStore.setState({ bindings: { ...DEFAULT_BINDINGS }, recording: null, vimMode: true });
    useMailStore.setState({
      activeAccountId: "acc-1",
      activeFolderId: "inbox-1",
      selectedMessageId: "m5",
      selectedThreadId: null,
      threadView: false,
      batchMode: false,
      selectedMessageIds: new Set(),
    });
    useToastStore.setState({ toasts: [] });
    useUIStore.setState({ activeView: "inbox" });
    useCommandStore.getState().close();
    setActiveMailList({ messages: many as never, threads: null });
  });

  afterEach(() => {
    clearActiveMailList();
    document.body.innerHTML = "";
    mocks.queryClient.getQueriesData.mockImplementation(() => []);
  });

  const selected = () => useMailStore.getState().selectedMessageId;

  it("binds Delete by default", () => {
    expect(DEFAULT_BINDINGS["delete-message"]).toBe("Delete");
  });

  it("Delete and dd trash the selected message and select the next one", async () => {
    renderHook(() => useKeyboard());
    press(document.body, "Delete");
    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient, "trash", [expect.objectContaining({ id: "m5" })],
      expect.objectContaining({ confirm: expect.any(Function) }),
    ));
    mocks.runMailAction.mock.calls[0][3].onRemoved(["m5"]);
    expect(selected()).toBe("m6");

    press(document.body, "d");
    expect(mocks.runMailAction).toHaveBeenCalledTimes(1);
    press(document.body, "d");
    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledTimes(2));
  });

  it("forgets a pending d after the timeout", () => {
    vi.useFakeTimers();
    try {
      renderHook(() => useKeyboard());
      press(document.body, "d");
      act(() => vi.advanceTimersByTime(1100));
      press(document.body, "d");
      expect(mocks.runMailAction).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("handles a non-matching second key on its own", () => {
    renderHook(() => useKeyboard());
    press(document.body, "d");
    press(document.body, "j");
    expect(selected()).toBe("m6");
  });

  it("gg, G, Ctrl+d and Ctrl+u move through the list", () => {
    renderHook(() => useKeyboard());
    press(document.body, "d", { ctrlKey: true });
    expect(selected()).toBe("m15");
    press(document.body, "d", { ctrlKey: true });
    expect(selected()).toBe("m24");
    press(document.body, "u", { ctrlKey: true });
    expect(selected()).toBe("m14");
    press(document.body, "g");
    press(document.body, "g");
    expect(selected()).toBe("m0");
    press(document.body, "Shift", { shiftKey: true });
    press(document.body, "G", { shiftKey: true });
    expect(selected()).toBe("m24");
  });

  it("h closes and l opens the first message", () => {
    renderHook(() => useKeyboard());
    press(document.body, "h");
    expect(selected()).toBeNull();
    press(document.body, "l");
    expect(selected()).toBe("m0");
  });

  it("g-prefixed keys open the active account's folders, or the combined one", () => {
    renderHook(() => useKeyboard());
    press(document.body, "g");
    press(document.body, "s");
    expect(useMailStore.getState().activeFolderId).toBe("sent-1");
    press(document.body, "g");
    press(document.body, "t");
    expect(useMailStore.getState().activeFolderId).toBe("trash-1");
    useMailStore.setState({ activeAccountId: null });
    press(document.body, "g");
    press(document.body, "i");
    expect(useMailStore.getState().activeFolderId).toBe("all:inbox");
  });

  it("x builds a bulk selection that dd, Delete and e hand to the batch toolbar, and Escape leaves it", () => {
    const requests: unknown[] = [];
    const onBatchMove = (event: Event) => requests.push((event as CustomEvent).detail);
    document.addEventListener(BATCH_MOVE_EVENT, onBatchMove);
    try {
      renderHook(() => useKeyboard());
      press(document.body, "x");
      press(document.body, "j");
      press(document.body, "x");
      expect([...useMailStore.getState().selectedMessageIds].sort()).toEqual(["m5", "m6"]);

      press(document.body, "d");
      press(document.body, "d");
      press(document.body, "Delete");
      press(document.body, "e");
      expect(requests).toEqual(["trash", "trash", "archive"]);
      expect(mocks.runMailAction).not.toHaveBeenCalled();

      press(document.body, "Escape");
      expect(useMailStore.getState().batchMode).toBe(false);
    } finally {
      document.removeEventListener(BATCH_MOVE_EVENT, onBatchMove);
    }
  });

  it("leaves bulk selections and Escape to the toolbar when vim keys are off", async () => {
    const requests: unknown[] = [];
    const onBatchMove = (event: Event) => requests.push((event as CustomEvent).detail);
    document.addEventListener(BATCH_MOVE_EVENT, onBatchMove);
    try {
      useShortcutStore.setState({ vimMode: false });
      useMailStore.setState({ batchMode: true, selectedMessageIds: new Set(["m5", "m6"]) });
      renderHook(() => useKeyboard());
      press(document.body, "Escape");
      expect(useMailStore.getState().batchMode).toBe(true);
      press(document.body, "Delete");
      expect(requests).toEqual([]);
      await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
        mocks.queryClient, "trash", [expect.objectContaining({ id: "m5" })], expect.anything(),
      ));
    } finally {
      document.removeEventListener(BATCH_MOVE_EVENT, onBatchMove);
    }
  });

  it.each(["kanban", "settings", "compose"] as const)("Delete, dd, e and x do nothing in %s", (view) => {
    useUIStore.setState({ activeView: view });
    renderHook(() => useKeyboard());
    press(document.body, "Delete");
    press(document.body, "d");
    press(document.body, "d");
    press(document.body, "e");
    press(document.body, "x");
    expect(mocks.runMailAction).not.toHaveBeenCalled();
    expect(useMailStore.getState().selectedMessageIds.size).toBe(0);
  });

  it("ignores key repeat for Delete, dd and e", async () => {
    renderHook(() => useKeyboard());
    press(document.body, "Delete");
    press(document.body, "Delete", { repeat: true });
    press(document.body, "Delete", { repeat: true });
    press(document.body, "e", { repeat: true });
    press(document.body, "d");
    press(document.body, "d", { repeat: true });
    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledTimes(1));
  });

  it("drops a pending d when a key in between is handled elsewhere", () => {
    renderHook(() => useKeyboard());
    const row = document.createElement("div");
    row.addEventListener("keydown", (event) => event.preventDefault());
    document.body.appendChild(row);
    press(document.body, "d");
    press(row, "ArrowDown");
    press(document.body, "d");
    expect(mocks.runMailAction).not.toHaveBeenCalled();
  });

  it("reads printable vim keys as typed: Caps Lock G, AltGr ?", () => {
    renderHook(() => useKeyboard());
    press(document.body, "G");
    expect(selected()).toBe("m24");
    press(document.body, "?", { ctrlKey: true, altKey: true });
    expect(useUIStore.getState().activeView).toBe("settings");
  });

  it("u runs the latest Undo toast once, then reports nothing to undo", () => {
    const undo = vi.fn();
    useToastStore.getState().addToast({ message: "Retry?", type: "error", action: { label: "Retry", onClick: vi.fn() } });
    useToastStore.getState().addToast({ message: "Moved to Trash", type: "success", undoable: true, action: { label: "Undo", onClick: undo } });
    renderHook(() => useKeyboard());
    press(document.body, "u");
    expect(undo).toHaveBeenCalledTimes(1);
    press(document.body, "u");
    expect(undo).toHaveBeenCalledTimes(1);
    expect(useToastStore.getState().toasts.at(-1)?.message).toBe("Nothing to undo");
  });

  it("? opens the shortcuts settings", () => {
    renderHook(() => useKeyboard());
    press(document.body, "?", { shiftKey: true });
    expect(useUIStore.getState().activeView).toBe("settings");
    expect(useUIStore.getState().settingsTab).toBe("shortcuts");
  });

  it("is off when vim keys are disabled, and user bindings win", async () => {
    useShortcutStore.setState({ vimMode: false });
    const { unmount } = renderHook(() => useKeyboard());
    press(document.body, "d");
    press(document.body, "d");
    press(document.body, "h");
    expect(mocks.runMailAction).not.toHaveBeenCalled();
    expect(selected()).toBe("m5");
    unmount();

    useShortcutStore.setState({ vimMode: true, bindings: { ...DEFAULT_BINDINGS, "archive-message": "D" } });
    renderHook(() => useKeyboard());
    press(document.body, "d");
    await waitFor(() => expect(mocks.runMailAction).toHaveBeenCalledWith(
      mocks.queryClient, "archive", [expect.objectContaining({ id: "m5" })], expect.anything(),
    ));
  });

  it("ignores vim keys while typing", () => {
    renderHook(() => useKeyboard());
    const input = document.createElement("input");
    document.body.appendChild(input);
    press(input, "d");
    press(input, "d");
    expect(mocks.runMailAction).not.toHaveBeenCalled();
  });
});
