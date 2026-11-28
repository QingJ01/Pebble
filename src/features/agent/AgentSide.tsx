import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Bell, CalendarClock, Check, Hourglass, Inbox, MailQuestion, Pause, Play, X } from "lucide-react";
import IconButton from "@/components/ui/IconButton";
import { formatListDate } from "@/lib/formatDate";
import { useUIStore } from "@/stores/ui.store";
import iconUrl from "@/assets/app-icon.png";
import { markDigestSeen, minutesToTime, useDigest, useNotifySettings } from "@/features/notify/api";
import { reasonLabel } from "@/features/notify/labels";
import { NotifyFeedback } from "@/features/notify/NotifyFeedback";
import { useAgentEvents, useAgentMutations, useThreadWatches, type AgentEvent, type ThreadWatch } from "./api";
import { dueLabel } from "./AgentThread";
import { useOpenMessage } from "./useOpenMessage";

/** Open deadlines from mail, soonest first (the week ahead). */
export function upcomingDeadlines(events: AgentEvent[], now = Date.now() / 1000): AgentEvent[] {
  return events
    .filter((event) => event.kind === "deadline" && !event.resolution && !event.dismissedAt)
    .filter((event) => {
      const due = Number(event.payload.due);
      return Number.isFinite(due) && due >= now - 12 * 3600 && due <= now + 7 * 86_400;
    })
    .sort((a, b) => Number(a.payload.due) - Number(b.payload.due));
}

function Deadlines() {
  const { t, i18n } = useTranslation();
  const { data: events = [] } = useAgentEvents();
  const { resolve } = useAgentMutations();
  const open = useOpenMessage();
  const items = upcomingDeadlines(events);
  if (items.length === 0) return null;
  return (
    <section className="agent-side__section" aria-labelledby="agent-deadlines-title">
      <h3 id="agent-deadlines-title" className="agent-side__title">
        {t("agent.deadlinesTitle", "Coming up")}
        <span className="agent-side__count">{items.length}</span>
      </h3>
      <ul className="agent-side__list">
        {items.map((event) => {
          const due = Number(event.payload.due);
          const late = due * 1000 <= Date.now();
          return (
            <li key={event.id} className={`agent-side__row${late ? " is-overdue" : ""}`}>
              <Hourglass size={14} className="agent-side__icon" aria-hidden="true" />
              <div className="agent-side__row-main">
                <button type="button" className="agent-side__row-title" onClick={() => open(event.accountId, event.messageId)}>
                  {String(event.payload.subject ?? "") || t("inbox.noSubject", "(no subject)")}
                </button>
                <span className={`agent-side__row-meta${late ? " is-overdue" : ""}`}>
                  {t("agent.dueBy", "{{from}} · due {{when}}", {
                    from: String(event.payload.from ?? ""),
                    when: dueLabel(t, due, event.payload.allDay === true, i18n.language),
                  })}
                </span>
              </div>
              <IconButton
                label={t("agent.deadlineMarkDone", "It's done")}
                icon={<Check size={14} />}
                onClick={() => resolve.mutate({ id: event.id, action: "done" })}
              />
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function Following({ watches }: { watches: ThreadWatch[] }) {
  const { t, i18n } = useTranslation();
  const { updateWatch } = useAgentMutations();
  const open = useOpenMessage();
  const live = watches.filter((watch) => watch.status === "active" || watch.status === "paused");
  return (
    <section className="agent-side__section" aria-labelledby="agent-following-title">
      <h3 id="agent-following-title" className="agent-side__title">
        {t("agent.followUpsTitle", "Following")}
        {live.length > 0 && <span className="agent-side__count">{live.length}</span>}
      </h3>
      {live.length === 0 ? (
        <p className="agent-side__empty">
          {t("agent.followingEmpty", "Use the bell in a message, or “remind me if there's no reply” when sending, and Pebble keeps track here.")}
        </p>
      ) : (
        <ul className="agent-side__list">
          {live.map((watch) => {
            const overdue = watch.status === "active" && !!watch.deadline && watch.deadline * 1000 <= Date.now();
            const openable = watch.linkMessageId
              ?? (watch.anchorMessageId && !watch.anchorMessageId.startsWith("send:") ? watch.anchorMessageId : null);
            return (
              <li key={watch.id} className={`agent-side__row${overdue ? " is-overdue" : ""}`}>
                <span className={`agent-side__dot${watch.status === "paused" ? " is-paused" : overdue ? " is-alert" : ""}`} aria-hidden="true" />
                <div className="agent-side__row-main">
                  <button
                    type="button"
                    className="agent-side__row-title"
                    disabled={!openable}
                    title={openable ? t("agent.openInInbox", "Open in the inbox") : undefined}
                    onClick={() => open(watch.accountId, openable)}
                  >
                    {watch.title || t("inbox.noSubject", "(no subject)")}
                  </button>
                  <span className={`agent-side__row-meta${overdue ? " is-overdue" : ""}`}>
                    {watch.status === "paused"
                      ? t("agent.paused", "Paused")
                      : overdue
                        ? t("agent.overdue", "No reply yet")
                        : watch.deadline
                          ? t("agent.until", "until {{date}}", { date: formatListDate(watch.deadline, i18n.language) })
                          : t("agent.sinceDate", "since {{date}}", { date: formatListDate(watch.createdAt, i18n.language) })}
                  </span>
                </div>
                <IconButton
                  label={watch.status === "paused" ? t("agent.resume", "Resume") : t("agent.pause", "Pause")}
                  icon={watch.status === "paused" ? <Play size={14} /> : <Pause size={14} />}
                  onClick={() => updateWatch.mutate({ id: watch.id, change: { status: watch.status === "paused" ? "active" : "paused" } })}
                />
                <IconButton
                  label={t("agent.stopFollowing", "Stop following")}
                  icon={<X size={14} />}
                  onClick={() => updateWatch.mutate({ id: watch.id, change: { status: "cancelled" } })}
                />
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

function Schedules() {
  const { t } = useTranslation();
  const { data } = useNotifySettings();
  const setActiveView = useUIStore((s) => s.setActiveView);
  const setSettingsTab = useUIStore((s) => s.setSettingsTab);
  const prefs = data?.defaults;
  if (!prefs) return null;
  const openSettings = () => {
    setSettingsTab("notifications");
    setActiveView("settings");
  };
  const hours = prefs.awaitingReplyHours;
  const now = new Date();
  const nowMinute = now.getHours() * 60 + now.getMinutes();
  const nextBriefing = nowMinute < prefs.briefingMinute
    ? t("agent.nextToday", "Next: today {{time}}", { time: minutesToTime(prefs.briefingMinute) })
    : t("agent.nextTomorrow", "Next: tomorrow {{time}}", { time: minutesToTime(prefs.briefingMinute) });
  return (
    <section className="agent-side__section" aria-labelledby="agent-schedules-title">
      <h3 id="agent-schedules-title" className="agent-side__title">{t("agent.schedulesTitle", "Routines")}</h3>
      <ul className="agent-side__list">
        <li>
          <button type="button" className="agent-side__routine" onClick={openSettings}>
            <CalendarClock size={16} aria-hidden="true" />
            <span className="agent-side__row-main">
              <span>{t("agent.routineBriefing", "Daily briefing")}</span>
              <span className="agent-side__row-meta">
                {prefs.briefingEnabled ? nextBriefing : t("agent.routineOff", "Off")}
              </span>
            </span>
          </button>
        </li>
        <li>
          <button type="button" className="agent-side__routine" onClick={openSettings}>
            <MailQuestion size={16} aria-hidden="true" />
            <span className="agent-side__row-main">
              <span>{t("agent.routineAwaiting", "Mail waiting for your reply")}</span>
              <span className="agent-side__row-meta">
                {prefs.awaitingReplyEnabled
                  ? hours < 24
                    ? t("agent.routineAfterHours", "After {{n}} hours", { n: hours })
                    : t("agent.routineAfterDays", { count: hours / 24, defaultValue: "After {{count}} days" })
                  : t("agent.routineOff", "Off")}
              </span>
            </span>
          </button>
        </li>
      </ul>
    </section>
  );
}

function Digest() {
  const { t } = useTranslation();
  const { data: digest = [] } = useDigest();
  const open = useOpenMessage();
  const [expanded, setExpanded] = useState(false);
  if (digest.length === 0) return null;
  return (
    <section className="agent-side__section" aria-labelledby="agent-digest-title">
      <div className="agent-side__title-row">
        <h3 id="agent-digest-title" className="agent-side__title">
          {t("agent.digestTitle", "Digest · {{n}}", { n: digest.length })}
        </h3>
        <button type="button" className="agent-side__text-btn" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>
          {expanded ? t("agent.hide", "Hide") : t("agent.show", "Show")}
        </button>
        <button type="button" className="agent-side__text-btn" onClick={() => void markDigestSeen()}>
          {t("agent.markSeen", "Mark all seen")}
        </button>
      </div>
      {expanded ? (
        <ul className="agent-side__list">
          {digest.map((entry) => (
            <li key={`${entry.accountId}:${entry.messageId}`}>
              <div className="agent-side__digest-row">
                <button type="button" className="agent-side__routine" onClick={() => open(entry.accountId, entry.messageId)}>
                  <Inbox size={14} aria-hidden="true" />
                  <span className="agent-side__row-main">
                    <span className="agent-side__ellipsis">{entry.subject || t("inbox.noSubject", "(no subject)")}</span>
                    <span className="agent-side__row-meta">
                      {entry.fromName || entry.fromAddress} · {entry.why ? `✦ ${entry.why}` : reasonLabel(t, entry.reason)}
                    </span>
                  </span>
                </button>
                <NotifyFeedback entry={entry} />
              </div>
            </li>
          ))}
        </ul>
      ) : (
        <p className="agent-side__empty">{t("agent.digestHint", "Mail that didn't need a notification waits here.")}</p>
      )}
    </section>
  );
}

/** Right column of the assistant view. */
export default function AgentSide() {
  const { t } = useTranslation();
  const { data: watches = [] } = useThreadWatches();
  const notificationsEnabled = useUIStore((s) => s.notificationsEnabled);
  const setNotificationsEnabled = useUIStore((s) => s.setNotificationsEnabled);
  const following = watches.filter((watch) => watch.status === "active").length;
  return (
    <aside className="agent-side scroll-region" aria-label={t("agent.sideLabel", "What Pebble is keeping track of")}>
      <div className="agent-side__card">
        <div className="agent-side__identity">
          <img src={iconUrl} alt="" aria-hidden="true" className="agent-side__avatar" draggable={false} />
          <div className="agent-side__who">
            <div className="agent-side__name">Pebble</div>
            <div className="agent-side__status">
              <span className="agent-side__live" aria-hidden="true" />
              {following > 0
                ? t("agent.statusFollowing", "Watching your mail · following {{n}}", { n: following })
                : t("agent.statusIdle", "Watching your mail, on this device")}
            </div>
          </div>
        </div>
        <div className="agent-side__setting">
          <Bell size={16} aria-hidden="true" />
          <span>{t("agent.desktopNotifications", "Desktop notifications")}</span>
          <button
            type="button"
            className={`agent-side__pill${notificationsEnabled ? " is-on" : ""}`}
            aria-pressed={notificationsEnabled}
            onClick={() => setNotificationsEnabled(!notificationsEnabled)}
          >
            {notificationsEnabled ? t("agent.on", "On") : t("agent.turnOn", "Turn on")}
          </button>
        </div>
        <Deadlines />
        <Following watches={watches} />
        <Schedules />
        <Digest />
      </div>
    </aside>
  );
}
