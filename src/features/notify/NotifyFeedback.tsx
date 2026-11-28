import { useTranslation } from "react-i18next";
import { useToastStore } from "@/stores/toast.store";
import { setSenderPref, useNotifyFeedback, type NotificationEntry } from "./api";

/**
 * "Not important" / "Should notify" for one decision. Corrections stay on
 * this device and guide AI help with notifications; repeated ones offer a
 * VIP or mute rule instead.
 */
export function NotifyFeedback({ entry }: { entry: NotificationEntry }) {
  const { t } = useTranslation();
  const feedback = useNotifyFeedback();
  const addToast = useToastStore((s) => s.addToast);
  if (entry.tier === "silent") return null;
  if (entry.feedback !== null && entry.feedback !== undefined) {
    return (
      <span className="notify-feedback__done">
        {entry.feedback ? t("notify.feedbackImportant", "You said: should notify") : t("notify.feedbackNot", "You said: not important")}
      </span>
    );
  }
  const important = entry.tier !== "interrupt";
  return (
    <button
      type="button"
      className="notify-feedback"
      disabled={feedback.isPending}
      onClick={(event) => {
        event.stopPropagation();
        feedback.mutate(
          { accountId: entry.accountId, messageId: entry.messageId, important },
          {
            onSuccess: (result) => {
              if (!result.suggest) return;
              const kind = result.suggest;
              addToast({
                type: "info",
                duration: 10_000,
                message: kind === "vip"
                  ? t("notify.suggestVip", "You often want mail from {{sender}}. Always notify for them?", { sender: result.sender })
                  : t("notify.suggestMute", "Mail from {{sender}} is rarely important to you. Mute them?", { sender: result.sender }),
                action: {
                  label: kind === "vip" ? t("notify.makeVip", "Make VIP") : t("notify.mute", "Mute"),
                  onClick: () => void setSenderPref(entry.accountId, result.sender, kind),
                },
              });
            },
            onError: () => addToast({ type: "error", message: t("notify.feedbackFailed", "Couldn't save that") }),
          },
        );
      }}
    >
      {important ? t("notify.shouldNotify", "Should notify") : t("notify.notImportant", "Not important")}
    </button>
  );
}
