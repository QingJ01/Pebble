import { useId } from "react";
import { useTranslation } from "react-i18next";
import { inputStyle, labelStyle } from "../styles/form";

export interface SyncRangeDraft {
  preset: "all" | "30" | "90" | "365" | "custom";
  customDays: string;
}

export function syncRangeDraft(days: number | null): SyncRangeDraft {
  const value = String(days);
  return {
    preset: days === null ? "all" : value === "30" || value === "90" || value === "365" ? value : "custom",
    customDays: days === null ? "" : value,
  };
}

export function syncRangeDays(draft: SyncRangeDraft): number | null | undefined {
  if (draft.preset === "all") return null;
  const raw = draft.preset === "custom" ? draft.customDays : draft.preset;
  if (!/^\d+$/.test(raw)) return undefined;
  const days = Number(raw);
  return Number.isInteger(days) && days >= 1 && days <= 36500 ? days : undefined;
}

export default function SyncRangeFields({ value, onChange, disabled = false, unsupported = false }: {
  value: SyncRangeDraft;
  onChange: (value: SyncRangeDraft) => void;
  disabled?: boolean;
  unsupported?: boolean;
}) {
  const { t } = useTranslation();
  const id = useId();
  const invalid = syncRangeDays(value) === undefined;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "6px", marginBottom: "16px" }}>
      <label htmlFor={`${id}-range`} style={labelStyle}>{t("syncRange.label", "Sync range")}</label>
      <select
        id={`${id}-range`}
        style={inputStyle}
        value={unsupported ? "all" : value.preset}
        disabled={disabled || unsupported}
        aria-describedby={`${id}-help`}
        onChange={(event) => onChange({ ...value, preset: event.target.value as SyncRangeDraft["preset"] })}
      >
        <option value="all">{t("syncRange.all", "All mail")}</option>
        <option value="30">{t("syncRange.month", "Last month (30 days)")}</option>
        <option value="90">{t("syncRange.quarter", "Last 3 months (90 days)")}</option>
        <option value="365">{t("syncRange.year", "Last year (365 days)")}</option>
        <option value="custom">{t("syncRange.custom", "Custom days")}</option>
      </select>
      {!unsupported && value.preset === "custom" && (
        <>
          <label htmlFor={`${id}-days`} style={labelStyle}>{t("syncRange.days", "Number of days")}</label>
          <input id={`${id}-days`} type="number" min={1} max={36500} step={1} required
            style={inputStyle} value={value.customDays} disabled={disabled}
            aria-invalid={invalid} aria-describedby={invalid ? `${id}-error` : `${id}-help`}
            onChange={(event) => onChange({ ...value, customDays: event.target.value })} />
          {invalid && <div id={`${id}-error`} role="alert" style={{ color: "var(--color-danger)", fontSize: "12px" }}>
            {t("syncRange.invalid", "Enter a whole number from 1 to 36500.")}
          </div>}
        </>
      )}
      <div id={`${id}-help`} style={{ color: "var(--color-text-secondary)", fontSize: "12px", lineHeight: 1.5 }}>
        {unsupported
          ? t("syncRange.pop3", "POP3 does not support server-side date filtering; all mail will be synced.")
          : t("syncRange.help", "Limits history downloaded from the server, starting at midnight UTC. New mail continues to sync; existing local mail is kept. To load earlier mail, choose a wider range or All mail and save.")}
      </div>
    </div>
  );
}
