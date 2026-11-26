import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_BINDINGS, formatShortcut, useShortcutStore } from "../../src/stores/shortcut.store";
import { useMailStore } from "../../src/stores/mail.store";

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
}));

vi.mock("../../src/stores/toast.store", () => ({
  toastError: mocks.toastError,
}));

vi.mock("../../src/lib/i18n", () => ({
  default: {
    t: (_key: string, fallback: string) => fallback,
  },
}));

import { useKeyboard } from "../../src/hooks/useKeyboard";

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
