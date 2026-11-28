import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { setNotificationLabels, setTrayMenuLabels } from "@/lib/api";
import type { NotificationLabels } from "@/lib/generated/ipc";
import { useUIStore } from "@/stores/ui.store";

/**
 * Localized strings for OS notifications and the tray tooltip. `{n}` and
 * `{latest}` stay literal: the backend fills them per notification.
 */
/** Keep `{name}` placeholders literal for the backend to fill. */
function raw(...names: string[]) {
  return {
    ...Object.fromEntries(names.map((name) => [name, `{${name}}`])),
    interpolation: { escapeValue: false },
  };
}

export function buildNotificationLabels(t: TFunction): NotificationLabels {
  return {
    new_mail_title: t("notifications.newMailTitle", "Pebble - New mail"),
    new_message_fallback: t("notifications.newMessageFallback", "New message"),
    new_mail_summary: t("notifications.newMailSummary", "{{n}} new messages — latest: {{latest}}", {
      n: "{n}",
      latest: "{latest}",
      interpolation: { escapeValue: false },
    }),
    open_action: t("notifications.open", "Open"),
    snoozed_title: t("notifications.snoozedTitle", "Pebble - Snoozed message"),
    test_title: t("notifications.testTitle", "Pebble - Test notification"),
    test_body: t("notifications.testBody", "Desktop notifications are working."),
    tray_attention_tooltip: t("notifications.trayAttention", "Pebble - New mail"),
    agent_title: t("notifications.agentTitle", "Pebble"),
    watch_reply: t("notifications.watchReply", "{{from}} replied: {{title}}", raw("from", "title")),
    watch_overdue: t("notifications.watchOverdue", "Still no reply: {{title}}", raw("title")),
    awaiting_one: t("notifications.awaitingOne", "{{from}} is waiting for your reply: {{subject}}", raw("from", "subject")),
    awaiting_many: t("notifications.awaitingMany", "{{n}} messages are waiting for your reply", raw("n")),
    briefing_body: t(
      "notifications.briefingBody",
      "Today: {{important}} important, {{awaiting}} waiting for your reply, {{followups}} follow-ups",
      raw("important", "awaiting", "followups"),
    ),
    agent_many: t("notifications.agentMany", "{{n}} new messages from Pebble", raw("n")),
    deadline_soon: t("notifications.deadlineSoon", "Due {{time}}: {{subject}}", raw("time", "subject")),
  };
}

export function useTrayI18n() {
  const language = useUIStore((s) => s.language);
  const { t } = useTranslation();

  useEffect(() => {
    setTrayMenuLabels(
      t("tray.show", "Show Window"),
      t("tray.hide", "Hide Window"),
      t("tray.quit", "Quit Pebble"),
    ).catch((err) => console.warn("Failed to sync tray menu labels", err));
    setNotificationLabels(buildNotificationLabels(t))
      .catch((err) => console.warn("Failed to sync notification labels", err));
  }, [language, t]);
}
