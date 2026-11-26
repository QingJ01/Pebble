/**
 * Finds translation keys referenced in src/ that are missing from the en or zh
 * resources (including the feature namespace files merged in src/lib/i18n.ts).
 *
 * Usage: node scripts/check-i18n-keys.ts   (exit code 1 when keys are missing)
 * Also run by tests/i18n/missing-keys.test.ts.
 *
 * A key is "referenced" when a string literal like "ns.some.key" appears in
 * src/**\/*.ts(x) and "ns" is a top-level object in the locale resources. That
 * covers t("..."), i18n.t("..."), ternaries inside t(), and key tables such as
 * labelKey / titleKey maps. Template literals with ${} are skipped.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

type Tree = { [key: string]: string | Tree };

const IS_FILE_MODULE = import.meta.url.startsWith("file:");
// Under vitest's jsdom environment import.meta.url is not a file URL; fall back to cwd (repo root).
const ROOT = IS_FILE_MODULE ? resolve(fileURLToPath(new URL(".", import.meta.url)), "..") : process.cwd();
const LOCALE_DIR = join(ROOT, "src", "locales");
const SRC_DIR = join(ROOT, "src");

/** Feature files merged on top of the main locale file (see src/lib/i18n.ts). */
const FEATURE_FILES = ["send-controls", "attachments", "folders", "mail-archive"];

/** Literals that look like keys but are not translation keys. */
const IGNORED = new Set<string>([]);

const PLURAL_SUFFIXES = ["_zero", "_one", "_two", "_few", "_many", "_other"];

function readJson(file: string): Tree {
  return JSON.parse(readFileSync(join(LOCALE_DIR, file), "utf8")) as Tree;
}

export function loadLocale(lang: "en" | "zh"): Tree {
  // Same shallow merge as src/lib/i18n.ts.
  let merged: Tree = { ...readJson(`${lang}.json`) };
  for (const feature of FEATURE_FILES) merged = { ...merged, ...readJson(`${feature}.${lang}.json`) };
  return merged;
}

function lookup(tree: Tree, key: string): string | Tree | undefined {
  let node: string | Tree | undefined = tree;
  for (const part of key.split(".")) {
    if (node === undefined || typeof node === "string") return undefined;
    node = node[part];
  }
  return node;
}

/** True when `key` resolves to a string, either directly or via plural forms. */
export function hasKey(tree: Tree, key: string): boolean {
  if (typeof lookup(tree, key) === "string") return true;
  return PLURAL_SUFFIXES.some((suffix) => typeof lookup(tree, key + suffix) === "string");
}

function walkFiles(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) {
      if (name === "locales" || name === "generated") continue;
      walkFiles(full, out);
    } else if (/\.(ts|tsx)$/.test(name) && !/\.d\.ts$/.test(name)) {
      out.push(full);
    }
  }
  return out;
}

export interface KeyUse {
  key: string;
  file: string;
}

export function collectKeyUses(namespaces: Set<string>): KeyUse[] {
  const uses: KeyUse[] = [];
  const literal = /(["'`])([A-Za-z][\w-]*(?:\.[\w-]+)+)\1/g;
  for (const file of walkFiles(SRC_DIR)) {
    const text = readFileSync(file, "utf8");
    for (const match of text.matchAll(literal)) {
      const key = match[2];
      const ns = key.split(".")[0];
      if (!namespaces.has(ns) || IGNORED.has(key)) continue;
      uses.push({ key, file: relative(ROOT, file) });
    }
  }
  return uses;
}

/** Collects every leaf key path in a tree. */
export function leafKeys(tree: Tree, prefix = ""): string[] {
  return Object.entries(tree).flatMap(([k, v]) =>
    typeof v === "string" ? [prefix + k] : leafKeys(v, `${prefix}${k}.`),
  );
}

export interface CheckResult {
  missing: { key: string; locales: string[]; files: string[] }[];
  legacyPlural: { key: string; locale: string }[];
}

export function checkI18nKeys(): CheckResult {
  const en = loadLocale("en");
  const zh = loadLocale("zh");
  const namespaces = new Set(
    [...Object.entries(en), ...Object.entries(zh)].filter(([, v]) => typeof v === "object").map(([k]) => k),
  );
  const byKey = new Map<string, Set<string>>();
  for (const use of collectKeyUses(namespaces)) {
    if (!byKey.has(use.key)) byKey.set(use.key, new Set());
    byKey.get(use.key)!.add(use.file);
  }
  const missing: CheckResult["missing"] = [];
  for (const [key, files] of [...byKey].sort(([a], [b]) => a.localeCompare(b))) {
    // A literal naming a whole namespace subtree (e.g. "settings.tabs" used as a prefix) is fine.
    const locales = (
      [
        ["en", en],
        ["zh", zh],
      ] as const
    )
      .filter(([, tree]) => !hasKey(tree, key) && typeof lookup(tree, key) !== "object")
      .map(([name]) => name);
    if (locales.length > 0) missing.push({ key, locales, files: [...files].sort() });
  }
  const legacyPlural = (
    [
      ["en", en],
      ["zh", zh],
    ] as const
  ).flatMap(([locale, tree]) =>
    leafKeys(tree)
      .filter((k) => k.endsWith("_plural"))
      .map((key) => ({ key, locale })),
  );
  return { missing, legacyPlural };
}

const isMain = IS_FILE_MODULE && process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const { missing, legacyPlural } = checkI18nKeys();
  for (const m of missing) console.log(`missing in ${m.locales.join("+")}: ${m.key}  (${m.files.join(", ")})`);
  for (const p of legacyPlural) console.log(`legacy _plural key in ${p.locale}: ${p.key} (use _one/_other)`);
  if (missing.length === 0 && legacyPlural.length === 0) console.log("i18n keys OK");
  process.exit(missing.length + legacyPlural.length > 0 ? 1 : 0);
}
