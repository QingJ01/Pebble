// Date/time formatting that follows the app language (Settings → Language),
// not the operating-system locale. Use these instead of calling
// toLocaleString()/toLocaleDateString() without a locale.
//
// Numbers are Unix timestamps in SECONDS (the backend's format); pass a Date
// for anything else.
import i18n from "i18next";

export type DateInput = Date | number;

/** BCP 47 locale for the current (or given) app language, e.g. "zh-CN" or "en". */
export function appLocale(language: string | undefined = i18n.language): string | undefined {
  if (!language) return undefined;
  return language.toLowerCase().startsWith("zh") ? "zh-CN" : language;
}

function toDate(value: DateInput): Date {
  return value instanceof Date ? value : new Date(value * 1000);
}

/** Date and time, e.g. "Mar 5, 2026, 14:30" / "2026年3月5日 14:30". */
export function formatDateTime(
  value: DateInput,
  options: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" },
  language?: string,
): string {
  return toDate(value).toLocaleString(appLocale(language), options);
}

/** Date only, e.g. "Mar 5, 2026" / "2026年3月5日". */
export function formatDate(
  value: DateInput,
  options: Intl.DateTimeFormatOptions = { dateStyle: "medium" },
  language?: string,
): string {
  return toDate(value).toLocaleDateString(appLocale(language), options);
}

/** Time only, e.g. "14:30" / "2:30 PM". */
export function formatTime(
  value: DateInput,
  options: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit" },
  language?: string,
): string {
  return toDate(value).toLocaleTimeString(appLocale(language), options);
}

/** Compact list date: time for today, otherwise month and day. */
export function formatListDate(value: DateInput, language?: string, now: Date = new Date()): string {
  const date = toDate(value);
  const isToday =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return isToday
    ? formatTime(date, undefined, language)
    : formatDate(date, { month: "short", day: "numeric" }, language);
}
