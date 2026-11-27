import { afterEach, describe, expect, it, vi } from "vitest";
import { profileLocalStorage } from "../../src/lib/profileStorage";

const platform = Object.getOwnPropertyDescriptor(Navigator.prototype, "platform");

async function loadStore() {
  vi.resetModules();
  return import("../../src/stores/shortcut.store");
}

describe("shortcut store defaults", () => {
  afterEach(() => {
    profileLocalStorage.removeItem("pebble-shortcuts");
    if (platform) Object.defineProperty(Navigator.prototype, "platform", platform);
  });

  it("doesn't let a new default take a key the user already saved", async () => {
    profileLocalStorage.setItem("pebble-shortcuts", JSON.stringify({ "archive-message": "Delete" }));
    const { useShortcutStore } = await loadStore();
    const { bindings } = useShortcutStore.getState();
    expect(bindings["archive-message"]).toBe("Delete");
    expect(bindings["delete-message"]).toBe("");
    expect(bindings["compose-new"]).toBe("C");
  });

  it("keeps a saved binding for the new action itself", async () => {
    profileLocalStorage.setItem("pebble-shortcuts", JSON.stringify({ "delete-message": "X" }));
    const { useShortcutStore } = await loadStore();
    expect(useShortcutStore.getState().bindings["delete-message"]).toBe("X");
  });

  it("uses Backspace (the Mac delete key) for delete on macOS", async () => {
    Object.defineProperty(Navigator.prototype, "platform", { configurable: true, get: () => "MacIntel" });
    const { DEFAULT_BINDINGS } = await loadStore();
    expect(DEFAULT_BINDINGS["delete-message"]).toBe("Backspace");
  });
});
