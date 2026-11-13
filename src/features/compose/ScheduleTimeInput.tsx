import { useTranslation } from "react-i18next";
import { scheduledInstant, timezoneLabel } from "./send-control-time";

export default function ScheduleTimeInput({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const { t, i18n } = useTranslation();
  const instant = scheduledInstant(value);
  return <label className="send-controls-time">
    <span>{t("sendControls.localTime", { zone: timezoneLabel() })}</span>
    <input aria-label={t("sendControls.localTime", { zone: timezoneLabel() })} type="datetime-local" value={value} onChange={(event) => onChange(event.target.value)} />
    {instant !== null && <small>{t("sendControls.exactTime", { time: new Date(instant * 1000).toLocaleString(i18n.language, { timeZoneName: "longOffset" }) })}</small>}
    <small>{t("sendControls.dst")}</small>
  </label>;
}
