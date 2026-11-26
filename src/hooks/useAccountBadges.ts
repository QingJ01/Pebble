import { useMemo } from "react";
import type { Account } from "@/lib/ipc-types";
import { assignAccountColors } from "@/lib/accountColors";
import { useAccountsQuery } from "@/hooks/queries";

/** An account as an AccountBadge draws it: its colour already resolved. */
export type BadgeAccount = Pick<Account, "id" | "email" | "display_name" | "color" | "account_label">;

/** Resolve every account's badge colour the same way everywhere. */
export function withResolvedColors<T extends BadgeAccount>(accounts: readonly T[]): T[] {
  const colors = assignAccountColors([...accounts]);
  return accounts.map((account) => ({ ...account, color: colors.get(account.id) ?? account.color }));
}

/**
 * All accounts with their badge colour resolved (assignAccountColors), so an
 * account without a saved colour looks the same in the sidebar, on mail rows
 * and in the unified-inbox filter bar.
 */
export function useResolvedAccounts(): Account[] {
  const { data: accounts = EMPTY_ACCOUNTS } = useAccountsQuery();
  return useMemo(() => withResolvedColors(accounts), [accounts]);
}

const EMPTY_ACCOUNTS: Account[] = [];

/**
 * Accounts to badge mail rows with, or null when rows need no account badge.
 *
 * Rows only carry an account badge when the list spans more than one account:
 * either `spansAccounts` says so (the unfiltered unified inbox with several
 * accounts), or the rows themselves come from more than one account (Starred,
 * Search, a page of the unified list). Colours come from assignAccountColors,
 * so two accounts without a saved colour never share one.
 */
export function useAccountBadges(
  rowAccountIds: readonly (string | null | undefined)[],
  spansAccounts = false,
): Map<string, BadgeAccount> | null {
  const accounts = useResolvedAccounts();
  const distinctCount = useMemo(
    () => new Set(rowAccountIds.filter(Boolean)).size,
    [rowAccountIds],
  );
  const show = accounts.length > 1 && (spansAccounts || distinctCount > 1);
  return useMemo(() => {
    if (!show) return null;
    return new Map<string, BadgeAccount>(accounts.map((account) => [account.id, account]));
  }, [accounts, show]);
}
