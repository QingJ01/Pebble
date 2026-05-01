export type Language = "en" | "zh";

export const LANGUAGE_STORAGE_KEY = "pebble-language";

type LanguageStorage = Pick<Storage, "getItem">;
type NavigatorLanguageSource = Pick<Navigator, "language" | "languages">;

function normalizeSavedLanguage(value: string | null): Language | null {
  return value === "en" || value === "zh" ? value : null;
}

export function detectSystemLanguage(source: NavigatorLanguageSource = navigator): Language {
  const primaryLanguage = source.languages?.[0] || source.language || "";
  return primaryLanguage.toLowerCase().startsWith("zh") ? "zh" : "en";
}

export function getInitialLanguage(
  storage: LanguageStorage = localStorage,
  source: NavigatorLanguageSource = navigator,
): Language {
  return normalizeSavedLanguage(storage.getItem(LANGUAGE_STORAGE_KEY)) ?? detectSystemLanguage(source);
}
