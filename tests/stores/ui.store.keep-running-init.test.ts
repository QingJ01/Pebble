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

  it("corrects a stored false once for existing profiles", async () => {
    localStorage.setItem("pebble-keep-running-background", "false");

    const { readKeepRunningInBackgroundPreference } = await import("../../src/stores/ui.store");

    expect(readKeepRunningInBackgroundPreference()).toBe(true);
    expect(localStorage.getItem("pebble-keep-running-background")).toBe("true");
    expect(localStorage.getItem("pebble-keep-running-background-reset-v1")).toBe("done");
  });

  it("respects a false stored after the one-time correction", async () => {
    localStorage.setItem("pebble-keep-running-background", "false");
    localStorage.setItem("pebble-keep-running-background-reset-v1", "done");

    const { readKeepRunningInBackgroundPreference } = await import("../../src/stores/ui.store");

    expect(readKeepRunningInBackgroundPreference()).toBe(false);
  });
});
