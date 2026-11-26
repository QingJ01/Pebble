import { useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { snoozeMessage } from "@/lib/api";
import { useUIStore } from "@/stores/ui.store";
import { describeError } from "@/lib/extractErrorMessage";
import Popover, { handleMenuKeyDown } from "@/components/ui/Popover";

interface Props {
  messageId: string;
  open?: boolean;
  anchorRef?: RefObject<HTMLElement | null>;
  onClose: () => void;
  onSnoozed: () => void;
}

interface Preset {
  key: string;
  label: string;
  getTimestamp: () => number;
}

function getPresets(t: (key: string, fallback: string) => string): Preset[] {
  return [
    {
      key: "hour",
      label: t("snooze.oneHour", "1 hour"),
      getTimestamp: () => Math.floor(Date.now() / 1000) + 3600,
    },
    {
      key: "tonight",
      label: t("snooze.tonightShort", "Tonight"),
      getTimestamp: () => {
        const d = new Date();
        d.setHours(20, 0, 0, 0);
        if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1);
        return Math.floor(d.getTime() / 1000);
      },
    },
    {
      key: "tomorrow",
      label: t("snooze.tomorrowShort", "Tomorrow"),
      getTimestamp: () => {
        const d = new Date();
        d.setDate(d.getDate() + 1);
        d.setHours(9, 0, 0, 0);
        return Math.floor(d.getTime() / 1000);
      },
    },
    {
      key: "monday",
      label: t("snooze.nextMondayShort", "Next Monday"),
      getTimestamp: () => {
        const d = new Date();
        const day = d.getDay();
        const daysUntilMonday = day === 0 ? 1 : 8 - day;
        d.setDate(d.getDate() + daysUntilMonday);
        d.setHours(9, 0, 0, 0);
        return Math.floor(d.getTime() / 1000);
      },
    },
  ];
}

function formatHint(timestamp: number, language: string | undefined) {
  try {
    return new Date(timestamp * 1000).toLocaleTimeString(language || undefined, { hour: "2-digit", minute: "2-digit" });
  } catch {
    return "";
  }
}

/** Snooze presets as an ARIA menu (↑/↓/Home/End, Escape closes, focus returns to the trigger). */
export default function SnoozePopover({ messageId, open = true, anchorRef, onClose, onSnoozed }: Props) {
  const { t, i18n } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<{ title: string; detail: string } | null>(null);

  async function handleSnooze(getTimestamp: () => number) {
    setLoading(true);
    setError(null);
    try {
      const returnTo = useUIStore.getState().activeView;
      await snoozeMessage(messageId, getTimestamp(), returnTo);
      onSnoozed();
    } catch (err) {
      console.error("Snooze failed:", err);
      const described = describeError(err, { fallbackKey: "snooze.failed", fallback: "Snooze failed" });
      setError({ title: described.title, detail: described.detail });
    } finally {
      setLoading(false);
    }
  }

  const menuLabel = t("snooze.menuLabel", "Snooze until");

  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      placement="bottom-end"
      role="menu"
      aria-label={menuLabel}
      aria-busy={loading || undefined}
      className="menu"
      onKeyDown={handleMenuKeyDown}
    >
      <div className="menu-label" aria-hidden="true">{menuLabel}</div>
      {getPresets(t).map((preset) => {
        const hint = preset.key === "hour" ? "" : formatHint(preset.getTimestamp(), i18n?.language);
        return (
          <button
            key={preset.key}
            type="button"
            role="menuitem"
            className="menu-item"
            disabled={loading}
            onClick={() => handleSnooze(preset.getTimestamp)}
          >
            <span>{preset.label}</span>
            {hint && <span className="menu-item__hint">{hint}</span>}
          </button>
        );
      })}
      {error && (
        <div role="alert" className="popover-error" title={error.detail || undefined}>
          {error.title}
        </div>
      )}
    </Popover>
  );
}
