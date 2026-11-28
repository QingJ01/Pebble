import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BellRing, BellPlus } from "lucide-react";
import IconButton from "@/components/ui/IconButton";
import Popover, { handleMenuKeyDown } from "@/components/ui/Popover";
import { formatListDate } from "@/lib/formatDate";
import { useToastStore } from "@/stores/toast.store";
import { deadlineInDays, useAgentMutations, useThreadWatchFor } from "./api";

interface Props {
  accountId: string;
  messageId: string;
  threadId: string | null | undefined;
}

const DEADLINES: Array<{ key: string; days: number | null; labelKey: string; fallback: string }> = [
  { key: "none", days: null, labelKey: "agent.followNoDeadline", fallback: "Tell me when they reply" },
  { key: "1", days: 1, labelKey: "agent.followOneDay", fallback: "…or remind me in 1 day" },
  { key: "3", days: 3, labelKey: "agent.followThreeDays", fallback: "…or remind me in 3 days" },
  { key: "7", days: 7, labelKey: "agent.followWeek", fallback: "…or remind me in a week" },
];

/** Reader toolbar: follow this thread ("tell me when they reply"). */
export default function FollowUpButton({ accountId, messageId, threadId }: Props) {
  const { t, i18n } = useTranslation();
  const buttonRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const addToast = useToastStore((s) => s.addToast);
  const { data: watch } = useThreadWatchFor(accountId, threadId);
  const { createWatch, updateWatch } = useAgentMutations();
  const following = watch?.status === "active" || watch?.status === "paused";

  const label = following
    ? watch?.deadline
      ? t("agent.followingUntil", "Following until {{date}}", { date: formatListDate(watch.deadline, i18n.language) })
      : t("agent.following", "Following this thread")
    : t("agent.follow", "Follow up");

  function follow(days: number | null) {
    setOpen(false);
    createWatch.mutate(
      { accountId, messageId, deadline: days === null ? null : deadlineInDays(days) },
      {
        onSuccess: () => addToast({ message: t("agent.followStarted", "Pebble will tell you when they reply"), type: "success" }),
        onError: () => addToast({ message: t("agent.followFailed", "Couldn't follow this thread"), type: "error" }),
      },
    );
  }

  return (
    <>
      <IconButton
        ref={buttonRef}
        size="md"
        label={label}
        icon={following ? <BellRing size={16} /> : <BellPlus size={16} />}
        pressed={following}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      />
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={buttonRef}
        placement="bottom-end"
        role="menu"
        aria-label={t("agent.followMenu", "Follow up")}
        className="menu"
        onKeyDown={handleMenuKeyDown}
      >
        <div className="menu-label" aria-hidden="true">{t("agent.followMenu", "Follow up")}</div>
        {DEADLINES.map((option) => (
          <button key={option.key} type="button" role="menuitem" className="menu-item" onClick={() => follow(option.days)}>
            <span>{t(option.labelKey, option.fallback)}</span>
          </button>
        ))}
        {following && watch && (
          <button
            type="button"
            role="menuitem"
            className="menu-item"
            onClick={() => {
              setOpen(false);
              updateWatch.mutate({ id: watch.id, change: { status: "cancelled" } });
            }}
          >
            <span>{t("agent.stopFollowing", "Stop following")}</span>
          </button>
        )}
      </Popover>
    </>
  );
}
