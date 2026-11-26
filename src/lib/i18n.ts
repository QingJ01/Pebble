import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "@/locales/en.json";
import zh from "@/locales/zh.json";
import sendControlsEn from "@/locales/send-controls.en.json";
import sendControlsZh from "@/locales/send-controls.zh.json";
import attachmentsEn from "@/locales/attachments.en.json";
import attachmentsZh from "@/locales/attachments.zh.json";
import foldersEn from "@/locales/folders.en.json";
import foldersZh from "@/locales/folders.zh.json";
import mailArchiveEn from "@/locales/mail-archive.en.json";
import mailArchiveZh from "@/locales/mail-archive.zh.json";
import { getInitialLanguage } from "@/lib/language";

const initialLanguage = getInitialLanguage();

i18n.use(initReactI18next).init({
  resources: {
    en: { translation: { ...en, ...sendControlsEn, ...attachmentsEn, ...foldersEn, ...mailArchiveEn } },
    zh: { translation: { ...zh, ...sendControlsZh, ...attachmentsZh, ...foldersZh, ...mailArchiveZh } },
  },
  lng: initialLanguage,
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

/** Keep <html lang> in sync so the webview picks CJK glyphs / hyphenation correctly. */
export function documentLangFor(language: string | undefined): string {
  if (!language) return "en";
  return language.toLowerCase().startsWith("zh") ? "zh-CN" : language;
}

function syncDocumentLang(language: string | undefined) {
  if (typeof document !== "undefined") {
    document.documentElement.lang = documentLangFor(language);
  }
}

syncDocumentLang(i18n.language ?? initialLanguage);
if (typeof i18n.on === "function") i18n.on("languageChanged", syncDocumentLang);

export default i18n;
