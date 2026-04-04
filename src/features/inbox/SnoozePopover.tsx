import { useState } from "react";
import { useTranslation } from "react-i18next";
import { snoozeMessage } from "@/lib/api";

interface Props {
  messageId: string;
  onClose: () => void;
  onSnoozed: () => void;
}

function getPresets(): { label: string; getTimestamp: () => number }[] {
  return [
    {
      label: "1 hour",
      getTimestamp: () => Math.floor(Date.now() / 1000) + 3600,
    },
    {
      label: "Tonight (8 PM)",
      getTimestamp: () => {
        const d = new Date();
        d.setHours(20, 0, 0, 0);
        if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1);
        return Math.floor(d.getTime() / 1000);
      },
    },
    {
      label: "Tomorrow (9 AM)",
      getTimestamp: () => {
        const d = new Date();
        d.setDate(d.getDate() + 1);
        d.setHours(9, 0, 0, 0);
        return Math.floor(d.getTime() / 1000);
      },
    },
    {
      label: "Next Monday (9 AM)",
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

export default function SnoozePopover({ messageId, onClose, onSnoozed }: Props) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

  async function handleSnooze(getTimestamp: () => number) {
    setLoading(true);
    try {
      await snoozeMessage(messageId, getTimestamp(), "inbox");
      onSnoozed();
    } catch (err) {
      console.error("Snooze failed:", err);
      setError(true);
      setTimeout(() => {
        setError(false);
        onClose();
      }, 2000);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div
      style={{
        position: "absolute",
        top: "100%",
        right: 0,
        marginTop: "4px",
        backgroundColor: "var(--color-bg)",
        border: "1px solid var(--color-border)",
        borderRadius: "8px",
        boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
        padding: "4px 0",
        zIndex: 100,
        minWidth: "200px",
      }}
    >
      <div
        style={{
          padding: "8px 12px",
          fontSize: "12px",
          fontWeight: 600,
          color: "var(--color-text-secondary)",
          borderBottom: "1px solid var(--color-border)",
        }}
      >
        Snooze until...
      </div>
      {getPresets().map((preset) => (
        <button
          key={preset.label}
          disabled={loading}
          onClick={() => handleSnooze(preset.getTimestamp)}
          style={{
            display: "block",
            width: "100%",
            textAlign: "left",
            padding: "8px 12px",
            border: "none",
            background: "none",
            cursor: loading ? "wait" : "pointer",
            fontSize: "13px",
            color: "var(--color-text-primary)",
          }}
          onMouseEnter={(e) =>
            (e.currentTarget.style.backgroundColor =
              "var(--color-bg-hover, rgba(0,0,0,0.05))")
          }
          onMouseLeave={(e) =>
            (e.currentTarget.style.backgroundColor = "transparent")
          }
        >
          {preset.label}
        </button>
      ))}
      {error && (
        <div style={{
          padding: "6px 10px",
          fontSize: "12px",
          color: "#ef4444",
          textAlign: "center",
        }}>
          {t("snooze.failed", "Snooze failed")}
        </div>
      )}
    </div>
  );
}
