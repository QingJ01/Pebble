/**
 * AccountBadge — the one visual for "which account is this".
 *
 * A rounded square with the account's monogram on a tint of the account
 * colour. Used on mail rows in the unified inbox (size "xs", 16px), in the
 * unified-inbox account filter chips ("xs"), and as the account button in the
 * collapsed sidebar rail ("md", 28px).
 *
 *   <AccountBadge account={account} />                       // 16px, decorative
 *   <AccountBadge account={account} size="md" selected />    // 28px rail button face
 *
 * The badge is decorative by default (aria-hidden); pass `label` when it is the
 * only thing naming the account, and it becomes role="img" with that label.
 */
import type { CSSProperties } from "react";
import type { Account } from "@/lib/ipc-types";
import { getAccountColor } from "@/lib/accountColors";
import "./account-badge.css";

type AccountLike = Pick<Account, "id" | "email" | "display_name" | "color"> & {
  account_label?: string | null;
};

export interface AccountBadgeProps {
  account: AccountLike;
  size?: "xs" | "sm" | "md";
  /** Accessible name; omit when the badge sits next to text that names the account. */
  label?: string;
  /** Hover tooltip; defaults to `label`. */
  title?: string;
  selected?: boolean;
  /** Small unread dot in the corner (rail). */
  unread?: boolean;
  className?: string;
}

/** First visible character of the account label, else of the email's local part. */
export function accountMonogram(account: AccountLike): string {
  const source = account.account_label?.trim() || account.email?.trim() || account.display_name?.trim() || "?";
  const first = Array.from(source)[0] ?? "?";
  return first.toLocaleUpperCase();
}

/** Short human name for an account: its label, else its email. */
export function accountShortName(account: AccountLike): string {
  return account.account_label?.trim() || account.email?.trim() || account.display_name?.trim() || "";
}

export default function AccountBadge({
  account,
  size = "xs",
  label,
  title,
  selected = false,
  unread = false,
  className,
}: AccountBadgeProps) {
  const style = { "--account-color": getAccountColor(account) } as CSSProperties;
  const classes = [
    "account-badge",
    `account-badge--${size}`,
    selected ? "is-selected" : "",
    className ?? "",
  ].filter(Boolean).join(" ");
  return (
    <span
      className={classes}
      style={style}
      title={title ?? label}
      {...(label ? { role: "img", "aria-label": label } : { "aria-hidden": true })}
    >
      {accountMonogram(account)}
      {unread && <span className="account-badge__dot" aria-hidden="true" />}
    </span>
  );
}
