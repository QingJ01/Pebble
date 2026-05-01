import { afterEach, describe, expect, it, vi } from "vitest";

function stubNavigatorLanguage(language: string) {
  Object.defineProperty(window.navigator, "language", {
    value: language,
    configurable: true,
  });
  Object.defineProperty(window.navigator, "languages", {
    value: [language],
    configurable: true,
  });
}

describe("UI store language initialization", () => {
  afterEach(() => {
    localStorage.clear();
    vi.resetModules();
    vi.restoreAllMocks();
  });

  it("uses Chinese for first launch on a Chinese system", async () => {
    localStorage.removeItem("pebble-language");
    stubNavigatorLanguage("zh-Hans-CN");

    const { useUIStore } = await import("../../src/stores/ui.store");

    expect(useUIStore.getState().language).toBe("zh");
  });

  it("uses English for first launch on a non-Chinese system", async () => {
    localStorage.removeItem("pebble-language");
    stubNavigatorLanguage("ja-JP");

    const { useUIStore } = await import("../../src/stores/ui.store");

    expect(useUIStore.getState().language).toBe("en");
  });

  it("does not override a saved user language", async () => {
    localStorage.setItem("pebble-language", "en");
    stubNavigatorLanguage("zh-CN");

    const { useUIStore } = await import("../../src/stores/ui.store");

    expect(useUIStore.getState().language).toBe("en");
  });
});
