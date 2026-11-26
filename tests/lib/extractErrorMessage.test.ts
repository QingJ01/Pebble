import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import {
  describeError,
  extractErrorMessage,
  formatError,
  parseError,
} from "../../src/lib/extractErrorMessage";

function readLocale(locale: string) {
  return JSON.parse(readFileSync(resolve(process.cwd(), "src", "locales", `${locale}.json`), "utf8"));
}

describe("error helpers", () => {
  beforeAll(async () => {
    await i18n.init({
      resources: { en: { translation: readLocale("en") }, zh: { translation: readLocale("zh") } },
      lng: "en",
      fallbackLng: "en",
    });
  });

  it("never stringifies a serialized PebbleError as [object Object]", () => {
    const err = { kind: "Network", message: "connection refused" };
    expect(extractErrorMessage(err)).toBe("connection refused");
    expect(parseError(err)).toEqual({ kind: "Network", message: "connection refused" });
    expect(extractErrorMessage({ kind: "Internal", message: { nested: true } })).toBe('{"nested":true}');
  });

  it("parses Error objects, plain strings and PebbleError display strings", () => {
    expect(parseError(new Error("boom"))).toEqual({ kind: "Unknown", message: "boom" });
    expect(parseError("Authentication failed: bad password")).toEqual({ kind: "Auth", message: "bad password" });
    expect(parseError("Sync cursor expired: x")).toEqual({ kind: "SyncCursorExpired", message: "x" });
    expect(parseError({ kind: "Bogus", message: "m" })).toEqual({ kind: "Unknown", message: "m" });
    expect(parseError(undefined)).toEqual({ kind: "Unknown", message: "" });
  });

  it("maps kinds to localized, actionable titles and keeps the raw text as detail", async () => {
    await i18n.changeLanguage("zh");
    const auth = describeError({ kind: "TokenExpired", message: "invalid_grant" });
    expect(auth).toEqual({ kind: "TokenExpired", title: readLocale("zh").errors.auth, detail: "invalid_grant" });
    expect(describeError({ kind: "Network", message: "timeout" }).title).toContain("网络或代理");
    await i18n.changeLanguage("en");
    expect(describeError({ kind: "Network", message: "timeout" }).title).toBe(readLocale("en").errors.network);
  });

  it("uses the caller's fallback key for kinds without specific guidance", () => {
    expect(describeError(new Error("disk"), { fallbackKey: "errors.saveDraft" }).title).toBe(readLocale("en").errors.saveDraft);
    expect(describeError({ kind: "Internal", message: "x" }).title).toBe(readLocale("en").errors.unknown);
    // Specific kinds win over the fallback.
    expect(describeError({ kind: "Auth", message: "x" }, { fallbackKey: "errors.saveDraft" }).title).toBe(readLocale("en").errors.auth);
  });

  it("formats a single inline string", () => {
    expect(formatError({ kind: "Network", message: "timeout" })).toBe(`${readLocale("en").errors.network} (timeout)`);
    expect(formatError({ kind: "Network", message: "" })).toBe(readLocale("en").errors.network);
  });

  it("has every errors.* key in both locales", () => {
    const en = readLocale("en").errors;
    const zh = readLocale("zh").errors;
    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort());
    for (const value of Object.values(zh) as string[]) {
      expect(value).not.toMatch(/您/);
      expect(value).not.toMatch(/[,;:](?!\/)/);
    }
  });
});
