import { useTranslation } from "react-i18next";
import type { AdvancedSearchQuery } from "@/lib/api";

interface Props {
  filters: AdvancedSearchQuery;
  onChange: (filters: AdvancedSearchQuery) => void;
  onClear: () => void;
}

export default function SearchFilters({ filters, onChange, onClear }: Props) {
  const { t } = useTranslation();

  function update(patch: Partial<AdvancedSearchQuery>) {
    onChange({ ...filters, ...patch });
  }

  const labelStyle: React.CSSProperties = {
    fontSize: "12px",
    color: "var(--color-text-secondary)",
    marginBottom: "4px",
    display: "block",
  };

  const inputStyle: React.CSSProperties = {
    width: "100%",
    padding: "6px 8px",
    fontSize: "13px",
    border: "1px solid var(--color-border)",
    borderRadius: "4px",
    backgroundColor: "var(--color-bg)",
    color: "var(--color-text-primary)",
    boxSizing: "border-box",
  };

  const fieldStyle: React.CSSProperties = {
    marginBottom: "10px",
  };

  return (
    <div
      style={{
        padding: "12px 14px",
        borderBottom: "1px solid var(--color-border)",
        backgroundColor: "var(--color-bg-secondary)",
      }}
    >
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "10px" }}>
        <div style={fieldStyle}>
          <label htmlFor="search-filter-from" style={labelStyle}>{t("search.from")}</label>
          <input
            id="search-filter-from"
            name="from"
            type="text"
            value={filters.from || ""}
            onChange={(e) => update({ from: e.target.value || undefined })}
            autoComplete="off"
            style={inputStyle}
          />
        </div>
        <div style={fieldStyle}>
          <label htmlFor="search-filter-to" style={labelStyle}>{t("search.to")}</label>
          <input
            id="search-filter-to"
            name="to"
            type="text"
            value={filters.to || ""}
            onChange={(e) => update({ to: e.target.value || undefined })}
            autoComplete="off"
            style={inputStyle}
          />
        </div>
        <div style={fieldStyle}>
          <label htmlFor="search-filter-subject" style={labelStyle}>{t("search.subject")}</label>
          <input
            id="search-filter-subject"
            name="subject"
            type="text"
            value={filters.subject || ""}
            onChange={(e) => update({ subject: e.target.value || undefined })}
            autoComplete="off"
            style={inputStyle}
          />
        </div>
        <div style={fieldStyle}>
          <label htmlFor="search-filter-date-from" style={labelStyle}>{t("search.dateFrom")}</label>
          <input
            id="search-filter-date-from"
            name="date_from"
            type="date"
            value={
              filters.dateFrom
                ? new Date(filters.dateFrom * 1000).toISOString().split("T")[0]
                : ""
            }
            onChange={(e) => {
              const val = e.target.value;
              update({ dateFrom: val ? Math.floor(new Date(val).getTime() / 1000) : undefined });
            }}
            style={inputStyle}
          />
        </div>
        <div style={fieldStyle}>
          <label htmlFor="search-filter-date-to" style={labelStyle}>{t("search.dateTo")}</label>
          <input
            id="search-filter-date-to"
            name="date_to"
            type="date"
            value={
              filters.dateTo
                ? new Date(filters.dateTo * 1000).toISOString().split("T")[0]
                : ""
            }
            onChange={(e) => {
              const val = e.target.value;
              update({
                dateTo: val
                  ? Math.floor(new Date(val + "T23:59:59").getTime() / 1000)
                  : undefined,
              });
            }}
            style={inputStyle}
          />
        </div>
        <div style={{ ...fieldStyle, display: "flex", alignItems: "flex-end", gap: "6px" }}>
          <input
            type="checkbox"
            checked={filters.hasAttachment || false}
            onChange={(e) =>
              update({ hasAttachment: e.target.checked ? true : undefined })
            }
            id="search-has-attachment"
          />
          <label
            htmlFor="search-has-attachment"
            style={{ fontSize: "13px", color: "var(--color-text-primary)", cursor: "pointer" }}
          >
            {t("search.hasAttachment")}
          </label>
        </div>
      </div>
      <button
        onClick={onClear}
        style={{
          marginTop: "4px",
          padding: "4px 10px",
          fontSize: "12px",
          color: "var(--color-text-secondary)",
          backgroundColor: "transparent",
          border: "1px solid var(--color-border)",
          borderRadius: "4px",
          cursor: "pointer",
        }}
      >
        {t("search.clearFilters")}
      </button>
    </div>
  );
}
