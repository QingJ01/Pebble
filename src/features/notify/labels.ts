import type { TFunction } from "i18next";
import type { NotifyTier } from "./api";

const REASONS: Record<string, string> = {
  backfill: "Not new mail",
  moved: "Filed away by a rule or the server",
  spam: "In spam or trash",
  rule_read: "A rule marked it read",
  muted: "Muted sender",
  self_sent: "Sent by you",
  watch: "Reply on a followed thread",
  reply_to_me: "Reply to your mail",
  vip: "VIP",
  bulk: "Newsletter or automated mail",
  direct: "Someone you've written to",
  default: "Notification mode",
  model_fallback: "Model unavailable, used the rules",
};

const MODEL_LABELS: Record<string, string> = {
  needs_action: "Needs your action",
  time_sensitive: "Time-sensitive",
  personal: "Personal",
  notification: "Notification",
  newsletter: "Newsletter",
  promotion: "Promotion",
  other: "Other",
};

/** Human label for a stored reason code (`rule_read`, `model:needs_action`…). */
export function reasonLabel(t: TFunction, code: string): string {
  if (code.startsWith("model:")) {
    const label = code.slice("model:".length);
    return t("notify.reasonModel", "AI: {{label}}", {
      label: t(`notify.modelLabel.${label}`, MODEL_LABELS[label] ?? label),
    });
  }
  return t(`notify.reason.${code}`, REASONS[code] ?? code);
}

export function tierLabel(t: TFunction, tier: NotifyTier): string {
  switch (tier) {
    case "interrupt":
      return t("notify.tierInterrupt", "Notified");
    case "digest":
      return t("notify.tierDigest", "Digest");
    default:
      return t("notify.tierSilent", "Silent");
  }
}

export const REASON_CODES = Object.keys(REASONS);
export const MODEL_LABEL_CODES = Object.keys(MODEL_LABELS);
