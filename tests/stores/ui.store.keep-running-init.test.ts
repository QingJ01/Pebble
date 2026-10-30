import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
}));

describe("UIStore keep-running-in-background initialization", () => {
  beforeEach(() => {
    vi.resetModules();
    localStorage.clear();
  });

  it("defaults to keeping the app running in the background", async () => {
    const { readKeepRunningInBackgroundPreference } = await import("../../src/stores/ui.store");

    expect(readKeepRunningInBackgroundPreference()).toBe(true);
  });

  it.each([false, true])("preserves an explicit background preference of %s on upgrade", async (enabled) => {
    localStorage.setItem("pebble-keep-running-background", String(enabled));

    const { readKeepRunningInBackgroundPreference, useUIStore } = await import("../../src/stores/ui.store");

    expect(readKeepRunningInBackgroundPreference()).toBe(enabled);
    expect(useUIStore.getState().keepRunningInBackground).toBe(enabled);
    expect(localStorage.getItem("pebble-keep-running-background")).toBe(String(enabled));
    expect(localStorage.getItem("pebble-keep-running-background-reset-v1")).toBeNull();
  });

  it("preserves false even when an old reset marker exists", async () => {
    localStorage.setItem("pebble-keep-running-background", "false");
    localStorage.setItem("pebble-keep-running-background-reset-v1", "done");

    const { readKeepRunningInBackgroundPreference } = await import("../../src/stores/ui.store");

    expect(readKeepRunningInBackgroundPreference()).toBe(false);
  });

  it("preserves a disabled preference migrated into profile-scoped storage", async () => {
    localStorage.setItem("pebble-keep-running-background", "false");
    const { initializeProfileStorageNamespace, profileLocalStorage } = await import("../../src/lib/profileStorage");
    await initializeProfileStorageNamespace(async () => "upgrade-test");

    const { useUIStore } = await import("../../src/stores/ui.store");

    expect(useUIStore.getState().keepRunningInBackground).toBe(false);
    expect(profileLocalStorage.getItem("pebble-keep-running-background")).toBe("false");
    expect(localStorage.getItem("pebble-keep-running-background")).toBeNull();
  });
});
