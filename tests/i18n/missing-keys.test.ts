// @vitest-environment node
import { describe, expect, it } from "vitest";
import { createInstance } from "i18next";
import { checkI18nKeys, leafKeys, loadLocale } from "../../scripts/check-i18n-keys";

const PLURAL = /_(zero|one|two|few|many|other|plural)$/;

function flatten(lang: "en" | "zh"): Map<string, string> {
  const tree = loadLocale(lang);
  const out = new Map<string, string>();
  for (const key of leafKeys(tree)) {
    const value = key.split(".").reduce<unknown>((node, part) => (node as Record<string, unknown>)[part], tree);
    out.set(key, value as string);
  }
  return out;
}

function placeholders(value: string): string[] {
  return [...value.matchAll(/\{\{\s*([\w.]+)\s*\}\}/g)].map((m) => m[1]).sort();
}

describe("i18n keys", () => {
  const result = checkI18nKeys();

  it("every translation key referenced in src exists in en and zh", () => {
    expect(result.missing.map((m) => `${m.key} (${m.locales.join("+")}) ${m.files.join(", ")}`)).toEqual([]);
  });

  it("uses _one/_other plural suffixes, not the legacy _plural suffix", () => {
    expect(result.legacyPlural).toEqual([]);
  });

  it("en and zh define the same keys (ignoring plural suffixes)", () => {
    const en = new Set([...flatten("en").keys()].map((k) => k.replace(PLURAL, "")));
    const zh = new Set([...flatten("zh").keys()].map((k) => k.replace(PLURAL, "")));
    expect([...en].filter((k) => !zh.has(k))).toEqual([]);
    expect([...zh].filter((k) => !en.has(k))).toEqual([]);
  });

  it("en plural keys come in _one/_other pairs", () => {
    const en = flatten("en");
    const unpaired = [...en.keys()]
      .filter((k) => k.endsWith("_one") || k.endsWith("_other"))
      .filter((k) => !en.has(k.replace(/_(one|other)$/, "_one")) || !en.has(k.replace(/_(one|other)$/, "_other")));
    expect(unpaired).toEqual([]);
  });

  it("en and zh use the same interpolation placeholders", () => {
    const en = flatten("en");
    const zh = flatten("zh");
    const byBase = (map: Map<string, string>) => {
      const out = new Map<string, Set<string>>();
      for (const [key, value] of map) {
        const base = key.replace(PLURAL, "");
        const set = out.get(base) ?? new Set<string>();
        placeholders(value).forEach((p) => set.add(p));
        out.set(base, set);
      }
      return out;
    };
    const enP = byBase(en);
    const zhP = byBase(zh);
    const mismatched = [...enP.keys()].filter((key) => {
      const a = [...(enP.get(key) ?? [])].sort().join(",");
      const b = [...(zhP.get(key) ?? [])].sort().join(",");
      return a !== b;
    });
    expect(mismatched).toEqual([]);
  });

  it("resolves count strings in both languages", async () => {
    const make = async (lng: "en" | "zh") => {
      const instance = createInstance();
      await instance.init({
        resources: { en: { translation: loadLocale("en") }, zh: { translation: loadLocale("zh") } },
        lng,
        fallbackLng: "en",
        interpolation: { escapeValue: false },
      });
      return instance;
    };
    const en = await make("en");
    const zh = await make("zh");
    expect(en.t("contacts.count", { count: 1 })).toBe("1 contact");
    expect(en.t("contacts.count", { count: 3 })).toBe("3 contacts");
    expect(en.t("privacy.tracker", { count: 2 })).toBe("2 trackers");
    expect(zh.t("contacts.count", { count: 1 })).toBe("1 位联系人");
    expect(zh.t("contacts.count", { count: 3 })).toBe("3 位联系人");
    expect(zh.t("batch.deleteTitle", { count: 2 })).toBe("删除 2 封邮件？");
    expect(zh.t("sendControls.countdown", { count: 5 })).toBe("将在 5 秒后发送");
  });
});
