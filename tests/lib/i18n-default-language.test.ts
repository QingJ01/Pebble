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

describe("i18n default language", () => {
  afterEach(() => {
    localStorage.clear();
    vi.resetModules();
    vi.restoreAllMocks();
  });

  it("defaults to Chinese when there is no saved language and the system language is Chinese", async () => {
    localStorage.removeItem("pebble-language");
    stubNavigatorLanguage("zh-CN");

    const { default: i18n } = await import("../../src/lib/i18n");

    expect(i18n.language).toBe("zh");
  });

  it("defaults to English when there is no saved language and the system language is not Chinese", async () => {
    localStorage.removeItem("pebble-language");
    stubNavigatorLanguage("fr-FR");

    const { default: i18n } = await import("../../src/lib/i18n");

    expect(i18n.language).toBe("en");
  });

  it("keeps an explicitly saved language over the system language", async () => {
    localStorage.setItem("pebble-language", "en");
    stubNavigatorLanguage("zh-CN");

    const { default: i18n } = await import("../../src/lib/i18n");

    expect(i18n.language).toBe("en");
  });
});
