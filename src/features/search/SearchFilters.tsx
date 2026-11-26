import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { AdvancedSearchQuery, Folder } from "@/lib/api";
import { listAccounts, listFolders } from "@/lib/api";
import Button from "@/components/ui/Button";
import Checkbox from "@/components/ui/Checkbox";

interface FolderWithAccount extends Folder {
  accountEmail: string;
}

interface Props {
  filters: AdvancedSearchQuery;
  onChange: (filters: AdvancedSearchQuery) => void;
  onClear: () => void;
}

function timestampToDateInput(timestamp?: number | null): string {
  if (!timestamp) return "";

  const date = new Date(timestamp * 1000);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function dateInputToTimestamp(value: string, endOfDay = false): number | undefined {
  if (!value) return undefined;
  return Math.floor(new Date(`${value}T${endOfDay ? "23:59:59" : "00:00:00"}`).getTime() / 1000);
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

  return (
    <section
      role="region"
      aria-label={t("search.filters")}
      className="search-filters-panel"
    >
      <div className="search-filters-grid">
        <div className="field search-filter-field">
          <label htmlFor="search-filter-from" className="field__label">{t("search.from")}</label>
          <input
            id="search-filter-from"
            name="from"
            type="text"
            value={filters.from || ""}
            onChange={(e) => update({ from: e.target.value || undefined })}
            autoComplete="off"
            className="input search-filter-control"
          />
        </div>
        <div className="field search-filter-field">
          <label htmlFor="search-filter-to" className="field__label">{t("search.to")}</label>
          <input
            id="search-filter-to"
            name="to"
            type="text"
            value={filters.to || ""}
            onChange={(e) => update({ to: e.target.value || undefined })}
            autoComplete="off"
            className="input search-filter-control"
          />
        </div>
        <div className="field search-filter-field">
          <label htmlFor="search-filter-subject" className="field__label">{t("search.subject")}</label>
          <input
            id="search-filter-subject"
            name="subject"
            type="text"
            value={filters.subject || ""}
            onChange={(e) => update({ subject: e.target.value || undefined })}
            autoComplete="off"
            className="input search-filter-control"
          />
        </div>
        <div className="field search-filter-field">
          <label htmlFor="search-filter-date-from" className="field__label">{t("search.dateFrom")}</label>
          <input
            id="search-filter-date-from"
            name="date_from"
            type="date"
            value={timestampToDateInput(filters.dateFrom)}
            onChange={(e) => {
              update({ dateFrom: dateInputToTimestamp(e.target.value) });
            }}
            className="input search-filter-control"
          />
        </div>
        <div className="field search-filter-field">
          <label htmlFor="search-filter-date-to" className="field__label">{t("search.dateTo")}</label>
          <input
            id="search-filter-date-to"
            name="date_to"
            type="date"
            value={timestampToDateInput(filters.dateTo)}
            onChange={(e) => {
              update({ dateTo: dateInputToTimestamp(e.target.value, true) });
            }}
            className="input search-filter-control"
          />
        </div>
        <div className="search-filter-toggle">
          <Checkbox
            id="search-has-attachment"
            checked={filters.hasAttachment || false}
            onChange={(e) =>
              update({ hasAttachment: e.target.checked ? true : undefined })
            }
            label={t("search.hasAttachment")}
          />
        </div>
        <div className="field search-filter-field">
          <label htmlFor="search-filter-folder" className="field__label">{t("search.folder")}</label>
          <select
            id="search-filter-folder"
            value={filters.folderId || ""}
            onChange={(e) => update({ folderId: e.target.value || undefined })}
            className="select search-filter-control"
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
      <div className="search-filters-actions">
        <Button size="sm" onClick={onClear} className="search-filters-clear">
          {t("search.clearFilters")}
        </Button>
      </div>
    </section>
  );
}
