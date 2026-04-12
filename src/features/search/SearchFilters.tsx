import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { AdvancedSearchQuery, Folder } from "@/lib/api";
import { listAccounts, listFolders } from "@/lib/api";
import {
  inputStyle as baseInputStyle,
  labelStyle,
} from "../../styles/form";

interface FolderWithAccount extends Folder {
  accountEmail: string;
}

interface Props {
  filters: AdvancedSearchQuery;
  onChange: (filters: AdvancedSearchQuery) => void;
  onClear: () => void;
}

export default function SearchFilters({ filters, onChange, onClear }: Props) {
  const { t } = useTranslation();
  const [folders, setFolders] = useState<FolderWithAccount[]>([]);

  useEffect(() => {
    listAccounts().then((accounts) => {
      const emailByAccountId = new Map(accounts.map((a) => [a.id, a.email]));
      Promise.all(accounts.map((a) => listFolders(a.id))).then((results) => {
        const all: FolderWithAccount[] = results.flat().map((f) => ({
          ...f,
          accountEmail: emailByAccountId.get(f.account_id) ?? "",
        }));
        setFolders(all);
      });
    });
  }, []);

  function update(patch: Partial<AdvancedSearchQuery>) {
    onChange({ ...filters, ...patch });
  }

  const inputStyle: React.CSSProperties = {
    ...baseInputStyle,
    padding: "6px 8px",
    borderRadius: "4px",
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
                ? (() => { const d = new Date(filters.dateFrom * 1000); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`; })()
                : ""
            }
            onChange={(e) => {
              const val = e.target.value;
              update({ dateFrom: val ? Math.floor(new Date(val + "T00:00:00").getTime() / 1000) : undefined });
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
                ? (() => { const d = new Date(filters.dateTo * 1000); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`; })()
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
        <div style={fieldStyle}>
          <label htmlFor="search-filter-folder" style={labelStyle}>{t("search.folder")}</label>
          <select
            id="search-filter-folder"
            value={filters.folderId || ""}
            onChange={(e) => update({ folderId: e.target.value || undefined })}
            style={{ ...inputStyle, appearance: "auto" }}
          >
            <option value="">{t("search.allFolders")}</option>
            {folders.map((f) => (
              <option key={f.id} value={f.id}>
                {f.accountEmail ? `${f.accountEmail} / ${f.name}` : f.name}
              </option>
            ))}
          </select>
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
