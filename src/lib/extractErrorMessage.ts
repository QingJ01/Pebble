/**
 * Error helpers for anything thrown by `invoke()` or frontend code.
 *
 * Backend commands reject with a serialised PebbleError `{ kind, message }`
 * (crates/pebble-core/src/error.rs); some legacy commands reject with a plain
 * string such as "Network error: …". Never use `String(err)` on these.
 *
 *   extractErrorMessage(err)   → raw message string (no i18n), never "[object Object]"
 *   parseError(err)            → { kind, message } with kind normalised (see ErrorKind)
 *   describeError(err, { fallbackKey }) → { kind, title, detail }
 *       title:  localized, actionable sentence for the kind (errors.* keys);
 *               for kinds without specific guidance (Internal/Unknown/Storage…)
 *               `fallbackKey` (an i18n key such as "compose.sendError") is used
 *               when given, else errors.unknown.
 *       detail: the raw backend/JS message, for "show details" / title tooltips.
 * Toast shortcut: `toastError(err, fallbackKey?)` in src/stores/toast.store.ts.
 */
// The default i18next instance (initialised in src/lib/i18n.ts). Imported
// directly so this module stays free of react-i18next side effects.
import i18n from "i18next";

export type ErrorKind =
  | "Auth"
  | "Network"
  | "Storage"
  | "Sync"
  | "SyncCursorExpired"
  | "Rule"
  | "Translate"
  | "Privacy"
  | "Internal"
  | "OAuth"
  | "TokenExpired"
  | "TokenRefreshFailed"
  | "UnsupportedProvider"
  | "Validation"
  | "Unknown";

export interface ParsedError {
  kind: ErrorKind;
  message: string;
}

export interface DescribedError {
  kind: ErrorKind;
  /** Localized, user-facing sentence. */
  title: string;
  /** Raw message (may be English / technical). Empty when nothing useful. */
  detail: string;
}

const KNOWN_KINDS: ReadonlySet<string> = new Set<ErrorKind>([
  "Auth", "Network", "Storage", "Sync", "SyncCursorExpired", "Rule", "Translate",
  "Privacy", "Internal", "OAuth", "TokenExpired", "TokenRefreshFailed",
  "UnsupportedProvider", "Validation",
]);

/** Display prefixes from PebbleError's `#[error(...)]` attributes. */
const STRING_PREFIXES: Array<[string, ErrorKind]> = [
  ["Authentication failed: ", "Auth"],
  ["Network error: ", "Network"],
  ["Storage error: ", "Storage"],
  ["Sync cursor expired: ", "SyncCursorExpired"],
  ["Sync error: ", "Sync"],
  ["Rule error: ", "Rule"],
  ["Translate error: ", "Translate"],
  ["Privacy error: ", "Privacy"],
  ["Internal error: ", "Internal"],
  ["OAuth error: ", "OAuth"],
  ["Access token expired: ", "TokenExpired"],
  ["Token refresh failed: ", "TokenRefreshFailed"],
  ["Unsupported provider: ", "UnsupportedProvider"],
  ["Validation error: ", "Validation"],
];

/** i18n key under `errors.*` for each kind (null → use fallbackKey / errors.unknown). */
const KIND_KEYS: Record<ErrorKind, string | null> = {
  Auth: "errors.auth",
  OAuth: "errors.oauth",
  TokenExpired: "errors.auth",
  TokenRefreshFailed: "errors.auth",
  Network: "errors.network",
  Sync: "errors.sync",
  SyncCursorExpired: "errors.syncCursorExpired",
  Storage: "errors.storage",
  Rule: "errors.rule",
  Translate: "errors.translate",
  Privacy: "errors.privacy",
  UnsupportedProvider: "errors.unsupportedProvider",
  Validation: "errors.validation",
  Internal: null,
  Unknown: null,
};

/** Kinds whose generic title is less useful than the caller's context message. */
const PREFER_FALLBACK: ReadonlySet<ErrorKind> = new Set<ErrorKind>(["Internal", "Unknown", "Storage", "Validation"]);

const EN_FALLBACKS: Record<string, string> = {
  "errors.auth": "Sign-in has expired. Reconnect this account in Settings → Accounts.",
  "errors.oauth": "Couldn't complete sign-in with the provider. Try connecting the account again.",
  "errors.network": "Can't reach the mail server. Check your network or proxy settings.",
  "errors.sync": "Sync failed. Pebble will retry; check the account settings if it keeps failing.",
  "errors.syncCursorExpired": "The sync position expired. Pebble will run a full resync.",
  "errors.storage": "Couldn't read or write local data. Check free disk space and try again.",
  "errors.rule": "This rule couldn't be applied. Check its conditions and actions.",
  "errors.translate": "Translation failed. Check the translation service in Settings → Translate.",
  "errors.privacy": "Couldn't apply privacy settings to this message.",
  "errors.unsupportedProvider": "This mail provider isn't supported for this action.",
  "errors.validation": "Some input is invalid. Check it and try again.",
  "errors.unknown": "Something went wrong. Please try again.",
  "errors.loadMessage": "Couldn't load this message. Try selecting it again.",
  "errors.saveDraft": "Couldn't save the draft. Your text is still here; try again.",
  "errors.saveRule": "Couldn't save the rule. Check its conditions and try again.",
  "errors.retrySend": "Couldn't retry sending. Try again later.",
};

function isErrorKind(value: unknown): value is ErrorKind {
  return typeof value === "string" && KNOWN_KINDS.has(value);
}

function stringifyMessage(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === undefined || value === null) return "";
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

/** Normalise any thrown value into `{ kind, message }`. */
export function parseError(err: unknown): ParsedError {
  if (err && typeof err === "object" && !(err instanceof Error)) {
    const obj = err as { kind?: unknown; message?: unknown };
    if ("kind" in obj && isErrorKind(obj.kind)) {
      return { kind: obj.kind, message: stringifyMessage(obj.message) };
    }
    if ("message" in obj) return { kind: "Unknown", message: stringifyMessage(obj.message) };
    return { kind: "Unknown", message: stringifyMessage(err) };
  }
  const text = err instanceof Error ? err.message : typeof err === "string" ? err : "";
  for (const [prefix, kind] of STRING_PREFIXES) {
    if (text.startsWith(prefix)) return { kind, message: text.slice(prefix.length) };
  }
  return { kind: "Unknown", message: text };
}

/** Extract a human-readable (raw, unlocalized) error message from an unknown catch value. */
export function extractErrorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    return stringifyMessage((err as { message: unknown }).message);
  }
  return "Unknown error";
}

export interface DescribeErrorOptions {
  /** i18n key used as title when the kind has no specific guidance. */
  fallbackKey?: string;
  /** Default text for fallbackKey if it's missing from the locale files. */
  fallback?: string;
}

function translate(key: string, fallback: string): string {
  try {
    const value = i18n.t?.(key, { defaultValue: fallback });
    return typeof value === "string" && value && value !== key ? value : fallback;
  } catch {
    return fallback;
  }
}

/** Localized, actionable description of any thrown value. */
export function describeError(err: unknown, options: DescribeErrorOptions = {}): DescribedError {
  const { kind, message } = parseError(err);
  const kindKey = KIND_KEYS[kind];
  let title: string;
  if (options.fallbackKey && (PREFER_FALLBACK.has(kind) || !kindKey)) {
    title = translate(options.fallbackKey, options.fallback ?? EN_FALLBACKS[options.fallbackKey] ?? EN_FALLBACKS["errors.unknown"]);
  } else {
    const key = kindKey ?? "errors.unknown";
    title = translate(key, EN_FALLBACKS[key]);
  }
  return { kind, title, detail: message.trim() };
}

/**
 * One-line inline form for places that can only show a string:
 * "<localized title> (<raw detail>)", or just the title when there is no detail.
 * Prefer showing `detail` separately (toast detail, title tooltip) when you can.
 */
export function formatError(err: unknown, options: DescribeErrorOptions = {}): string {
  const { title, detail } = describeError(err, options);
  return detail && detail !== title ? `${title} (${detail})` : title;
}
