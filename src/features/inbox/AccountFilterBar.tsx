import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Folder } from "@/lib/api";
import type { FolderRole } from "@/lib/folderAggregation";
import type { BadgeAccount } from "@/hooks/useAccountBadges";
import { useFolderUnreadCountsForAccounts } from "@/hooks/queries/useFolderUnreadCounts";
import AccountBadge, { accountShortName } from "@/components/ui/AccountBadge";
import "./account-filter.css";

interface Props {
  /** Accounts in sidebar order, colours resolved. */
  accounts: BadgeAccount[];
  /** Folders of all accounts (for the per-account unread counts). */
  folders: Folder[];
  /** Role of the unified folder being shown (inbox, sent, …). */
  role: FolderRole | null;
  /** Selected account id, null = all accounts. */
  value: string | null;
  onChange: (accountId: string | null) => void;
}

/**
 * Unified inbox account filter: "All N" + one chip per account. A radio group
 * (arrow keys move and select, one tab stop); scrolls sideways instead of
 * wrapping, with a fade on the side that has more chips.
 */
export default function AccountFilterBar({ accounts, folders, role, value, onChange }: Props) {
  const { t } = useTranslation();
  const { data: countsByFolderId } = useFolderUnreadCountsForAccounts(
    accounts.map((account) => account.id),
    { always: true },
  );
  const barRef = useRef<HTMLDivElement>(null);
  const [overflow, setOverflow] = useState<{ start: boolean; end: boolean }>({ start: false, end: false });

  const unreadFor = (accountId: string | null) => folders
    .filter((folder) => folder.role === role && (!accountId || folder.account_id === accountId))
    .reduce((sum, folder) => sum + ((countsByFolderId as Record<string, number>)[folder.id] ?? 0), 0);

  const options: { id: string | null; name: string; account?: BadgeAccount }[] = [
    { id: null, name: t("inbox.accountFilter.all", "All") },
    ...accounts.map((account) => ({ id: account.id, name: accountShortName(account), account })),
  ];
  const selectedIndex = Math.max(0, options.findIndex((option) => option.id === value));

  const updateOverflow = useCallback(() => {
    const bar = barRef.current;
    if (!bar) return;
    const start = bar.scrollLeft > 1;
    const end = bar.scrollLeft + bar.clientWidth < bar.scrollWidth - 1;
    setOverflow((prev) => (prev.start === start && prev.end === end ? prev : { start, end }));
  }, []);

  useEffect(() => {
    updateOverflow();
    const bar = barRef.current;
    if (!bar || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(updateOverflow);
    observer.observe(bar);
    return () => observer.disconnect();
  }, [updateOverflow, accounts.length]);

  function select(index: number) {
    const clamped = (index + options.length) % options.length;
    onChange(options[clamped].id);
    const chip = barRef.current?.querySelectorAll<HTMLElement>("[role=radio]")[clamped];
    chip?.focus();
    chip?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    let next: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") next = selectedIndex + 1;
    else if (event.key === "ArrowLeft" || event.key === "ArrowUp") next = selectedIndex - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = options.length - 1;
    if (next === null) return;
    event.preventDefault();
    select(next);
  }

  const fade = [overflow.start ? "start" : "", overflow.end ? "end" : ""].filter(Boolean).join(" ");

  return (
    <div
      ref={barRef}
      className="account-filter-bar"
      role="radiogroup"
      aria-label={t("inbox.accountFilter.label", "Filter by account")}
      data-fade={fade || undefined}
      onKeyDown={handleKeyDown}
      onScroll={updateOverflow}
    >
      {options.map((option, index) => {
        const checked = index === selectedIndex;
        const unread = unreadFor(option.id);
        const accessibleName = unread > 0
          ? `${option.name}, ${t("inbox.accountFilter.unread", { count: unread, defaultValue: "{{count}} unread" })}`
          : option.name;
        return (
          <button
            key={option.id ?? "all"}
            type="button"
            role="radio"
            aria-checked={checked}
            aria-label={accessibleName}
            title={option.account ? option.name : undefined}
            tabIndex={checked ? 0 : -1}
            className="chip chip--md account-filter-chip"
            onClick={() => select(index)}
          >
            {option.account && <AccountBadge account={option.account} size="xs" />}
            <span className="account-filter-chip__name">{option.name}</span>
            {unread > 0 && <span className="account-filter-chip__count" aria-hidden="true">{unread}</span>}
          </button>
        );
      })}
    </div>
  );
}
