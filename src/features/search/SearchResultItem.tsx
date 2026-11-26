import { useTranslation } from "react-i18next";
import { formatListDate } from "@/lib/formatDate";
import type { SearchHit, Message } from "@/lib/api";
import type { BadgeAccount } from "@/hooks/useAccountBadges";
import MailRowAccountBadge from "@/components/MailRowAccountBadge";
import "@/components/mail-list.css";

/** Highlight all occurrences of `terms` in `text` by wrapping in <mark>. */
function highlightTerms(text: string, query: string): React.ReactNode {
  if (!query.trim()) return text;
  const terms = query.trim().split(/\s+/).filter(Boolean).map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  if (terms.length === 0) return text;
  const regex = new RegExp(`(${terms.join("|")})`, "gi");
  const parts = text.split(regex);
  return parts.map((part, i) =>
    regex.test(part) ? (
      <mark key={i} className="search-result-row__mark">{part}</mark>
    ) : (
      part
    ),
  );
}

interface Props {
  hit: SearchHit;
  message?: Message | null;
  isSelected: boolean;
  onClick: () => void;
  query?: string;
  /** Roving tabindex: 0 for the list's active row, -1 for the others. */
  tabIndex?: number;
  onRowFocus?: (messageId: string) => void;
  /** Account to badge the row with (results spanning several accounts only). */
  account?: BadgeAccount;
}

export default function SearchResultItem({ hit, message, isSelected, onClick, query = "", tabIndex = 0, onRowFocus, account }: Props) {
  const { t, i18n } = useTranslation();
  const subject = hit.subject || message?.subject || hit.snippet || t("common.noSubject");
  // Sender name like the message list, falling back to the address.
  const from = hit.from_name || message?.from_name || hit.from_address || message?.from_address || "";
  const date = (hit.date || message?.date) ? formatListDate(hit.date || message!.date, i18n?.language) : "";

  return (
    <div
      role="option"
      aria-selected={isSelected}
      tabIndex={tabIndex}
      data-row-id={hit.message_id}
      className="search-result-row"
      onClick={onClick}
      onFocus={(event) => {
        if (event.target === event.currentTarget) onRowFocus?.(hit.message_id);
      }}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onClick(); }
      }}
    >
      <div className="mail-row__line">
        <span className="mail-row__primary">
          <span className="mail-row__ellipsis" title={from}>{highlightTerms(from, query)}</span>
        </span>
        {(date || account) && (
          <div className="mail-row__meta">
            {account && <MailRowAccountBadge account={account} />}
            {date && <span className="mail-row__date">{date}</span>}
          </div>
        )}
      </div>
      <div className="mail-row__subject mail-row__ellipsis" title={subject}>
        {highlightTerms(subject, query)}
      </div>
      <div className="mail-row__snippet mail-row__ellipsis">
        {highlightTerms(hit.snippet, query)}
      </div>
    </div>
  );
}
