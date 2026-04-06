import { useTranslation } from "react-i18next";
import type { SearchHit, Message } from "@/lib/api";

/** Highlight all occurrences of `terms` in `text` by wrapping in <mark>. */
function highlightTerms(text: string, query: string): React.ReactNode {
  if (!query.trim()) return text;
  const terms = query.trim().split(/\s+/).filter(Boolean).map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  if (terms.length === 0) return text;
  const regex = new RegExp(`(${terms.join("|")})`, "gi");
  const parts = text.split(regex);
  return parts.map((part, i) =>
    regex.test(part) ? (
      <mark key={i} style={{ background: "var(--color-accent)", color: "#fff", borderRadius: "2px", padding: "0 1px" }}>{part}</mark>
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
}

export default function SearchResultItem({ hit, message, isSelected, onClick, query = "" }: Props) {
  const { t } = useTranslation();
  const subject = hit.subject || message?.subject || hit.snippet || t("common.noSubject");
  const from = hit.from_address || message?.from_address || "";
  const date = (hit.date || message?.date) ? new Date((hit.date || message!.date) * 1000).toLocaleDateString() : "";

  return (
    <div
      role="option"
      aria-selected={isSelected}
      tabIndex={0}
      onClick={onClick}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onClick(); } }}
      style={{
        padding: "10px 14px",
        cursor: "pointer",
        borderBottom: "1px solid var(--color-border)",
        backgroundColor: isSelected ? "var(--color-bg-hover)" : "transparent",
      }}
      onMouseEnter={(e) => {
        if (!isSelected) e.currentTarget.style.backgroundColor = "var(--color-bg-hover)";
      }}
      onMouseLeave={(e) => {
        if (!isSelected) e.currentTarget.style.backgroundColor = "transparent";
      }}
    >
      <div
        style={{
          fontSize: "13px",
          fontWeight: 600,
          color: "var(--color-text-primary)",
          marginBottom: "2px",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {highlightTerms(subject, query)}
      </div>
      <div
        style={{
          fontSize: "12px",
          color: "var(--color-text-secondary)",
          marginBottom: "2px",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {highlightTerms(from, query)}
      </div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <div
          style={{
            fontSize: "12px",
            color: "var(--color-text-tertiary)",
            flex: 1,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            marginRight: "8px",
          }}
        >
          {highlightTerms(hit.snippet, query)}
        </div>
        {date && (
          <div style={{ fontSize: "11px", color: "var(--color-text-tertiary)", flexShrink: 0 }}>
            {date}
          </div>
        )}
      </div>
    </div>
  );
}
