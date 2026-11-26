import AccountBadge, { accountShortName } from "./ui/AccountBadge";
import type { BadgeAccount } from "@/hooks/useAccountBadges";

/**
 * The account badge in a mail row's meta area, right before the date.
 * Labelled (role="img") so screen readers hear the account once per row as
 * part of the row's name; the tooltip shows the same short name.
 */
export default function MailRowAccountBadge({ account }: { account: BadgeAccount }) {
  const name = accountShortName(account);
  return <AccountBadge account={account} size="xs" label={name} className="mail-row__account" />;
}
