import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { ArrowDown, ArrowUp, Check, Sun } from "lucide-react";
import IconButton from "@/components/ui/IconButton";
import { formatDate, formatTime } from "@/lib/formatDate";
import iconUrl from "@/assets/app-icon.png";
import {
  useAgentEvents,
  useAgentMutations,
  type AgentAction,
  type AgentEvent,
  type AgentResolution,
} from "./api";
import { useOpenMessage } from "./useOpenMessage";

// ---------------------------------------------------------------- helpers

function str(value: unknown): string {
  return typeof value === "string" ? value : value == null ? "" : String(value);
}

function num(value: unknown): number {
  return typeof value === "number" ? value : Number(value) || 0;
}

/** "3 h" / "2 days" for a span of `seconds`. */
export function waitedLabel(t: TFunction, seconds: number): string {
  const hours = Math.max(1, Math.round(seconds / 3600));
  if (hours < 48) return t("agent.waitedHours", "{{n}} h", { n: hours });
  return t("agent.waitedDays", "{{n}} days", { n: Math.round(hours / 24) });
}

/** "Today 09:04" / "Yesterday 16:17" / "Nov 24 10:11". */
export function dayLabel(t: TFunction, timestamp: number, language: string | undefined, now = new Date()): string {
  const date = new Date(timestamp * 1000);
  const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((startOfDay(now) - startOfDay(date)) / 86_400_000);
  const time = formatTime(timestamp, undefined, language);
  if (days === 0) return t("agent.today", "Today {{time}}", { time });
  if (days === 1) return t("agent.yesterday", "Yesterday {{time}}", { time });
  return `${formatDate(timestamp, { month: "short", day: "numeric" }, language)} ${time}`;
}

function greeting(t: TFunction, timestamp: number): string {
  const hour = new Date(timestamp * 1000).getHours();
  if (hour < 12) return t("agent.goodMorning", "Good morning.");
  if (hour < 18) return t("agent.goodAfternoon", "Good afternoon.");
  return t("agent.goodEvening", "Good evening.");
}

const nowSeconds = () => Math.floor(Date.now() / 1000);

/** A run of messages shown as one: reminders found in the same scan merge. */
export type Block =
  | { kind: "awaiting"; events: AgentEvent[]; at: number }
  | { kind: "single"; event: AgentEvent; at: number };

/** Merge "waiting for your reply" reminders raised within 10 minutes. */
export function buildBlocks(events: AgentEvent[]): Block[] {
  const ordered = [...events].sort((a, b) => a.createdAt - b.createdAt);
  const blocks: Block[] = [];
  for (const event of ordered) {
    const last = blocks[blocks.length - 1];
    const awaiting = event.kind === "awaiting_reply" && !event.payload.again;
    if (awaiting && last?.kind === "awaiting" && event.createdAt - last.events[last.events.length - 1].createdAt <= 600) {
      last.events.push(event);
      continue;
    }
    blocks.push(awaiting ? { kind: "awaiting", events: [event], at: event.createdAt } : { kind: "single", event, at: event.createdAt });
  }
  return blocks;
}

function initials(name: string): string {
  const clean = name.replace(/["<>]/g, "").trim();
  if (!clean) return "?";
  const first = [...clean][0];
  return /[a-z]/i.test(first) ? first.toUpperCase() : first;
}

function hue(text: string): number {
  let h = 0;
  for (const ch of text) h = (h * 31 + ch.charCodeAt(0)) % 360;
  return h;
}

// ------------------------------------------------------------- primitives

function Avatar({ hidden }: { hidden?: boolean }) {
  return hidden ? <span className="agent-msg__avatar-spacer" aria-hidden="true" /> : (
    <img src={iconUrl} alt="" aria-hidden="true" className="agent-msg__avatar" draggable={false} />
  );
}

/** A message from Pebble. */
function AgentMessage({ children, compact, wide }: { children: ReactNode; compact?: boolean; wide?: boolean }) {
  return (
    <div className={`agent-msg${compact ? " is-continued" : ""}`}>
      <Avatar hidden={compact} />
      <div className={`agent-msg__body${wide ? " is-wide" : ""}`}>{children}</div>
    </div>
  );
}

function Bubble({ children, tone }: { children: ReactNode; tone?: "card" }) {
  return <div className={`agent-msg__bubble${tone === "card" ? " is-card" : ""}`}>{children}</div>;
}

/** The user's quick reply, right-aligned. */
function UserReply({ children }: { children: ReactNode }) {
  return (
    <div className="agent-user">
      <div className="agent-user__bubble">{children}</div>
    </div>
  );
}

function Chips({ children, label }: { children: ReactNode; label: string }) {
  return <div className="agent-chips" role="group" aria-label={label}>{children}</div>;
}

function Chip({ onClick, primary, children, disabled }: { onClick: () => void; primary?: boolean; children: ReactNode; disabled?: boolean }) {
  return (
    <button type="button" className={`agent-chip${primary ? " is-primary" : ""}`} onClick={onClick} disabled={disabled}>
      {children}
    </button>
  );
}

function resolutionText(t: TFunction, resolution: AgentResolution): string {
  const days = resolution.days ?? 1;
  switch (resolution.action) {
    case "wait":
      return days === 7 ? t("agent.replyWaitWeek", "Wait another week") : t("agent.replyWaitDays", "Wait {{n}} more days", { n: days });
    case "stop":
      return t("agent.replyStop", "Stop following");
    case "later":
      return days === 1 ? t("agent.replyLater", "Remind me tomorrow") : t("agent.replyLaterDays", "Remind me in {{n}} days", { n: days });
    case "no_reply":
      return t("agent.replyNoReply", "No reply needed");
    default:
      return "";
  }
}

function MailCard({ event }: { event: AgentEvent }) {
  const { t } = useTranslation();
  const open = useOpenMessage();
  const { resolve } = useAgentMutations();
  const p = event.payload;
  const from = str(p.from) || str(p.fromAddress);
  const resolution = event.resolution;
  const act = (action: AgentAction, days?: number) => resolve.mutate({ id: event.id, action, days });
  return (
    <article className={`agent-mail${resolution ? " is-settled" : ""}`}>
      <button type="button" className="agent-mail__main" onClick={() => open(event.accountId, event.messageId)}>
        <span className="agent-mail__avatar" style={{ ["--avatar-hue" as string]: hue(str(p.fromAddress) || from) }} aria-hidden="true">
          {initials(from)}
        </span>
        <span className="agent-mail__text">
          <span className="agent-mail__top">
            <span className="agent-mail__from">{from}</span>
            <span className="agent-mail__time">
              {t("agent.waitingFor", "waiting {{time}}", { time: waitedLabel(t, nowSeconds() - num(p.receivedAt)) })}
            </span>
          </span>
          <span className="agent-mail__subject">{str(p.subject) || t("inbox.noSubject", "(no subject)")}</span>
          {str(p.snippet) && <span className="agent-mail__snippet">{str(p.snippet)}</span>}
          {typeof p.why === "string" && <span className="agent-mail__why agent-ai-text">{p.why}</span>}
        </span>
      </button>
      {resolution ? (
        <div className="agent-mail__status">
          <Check size={13} aria-hidden="true" />
          {resolution.action === "answered" ? t("agent.settledAnswered", "You replied") : resolutionText(t, resolution)}
        </div>
      ) : (
        <Chips label={t("agent.quickReplies", "Quick replies")}>
          <Chip primary onClick={() => open(event.accountId, event.messageId)}>{t("agent.reply", "Reply")}</Chip>
          <Chip onClick={() => act("later", 1)}>{t("agent.replyLater", "Remind me tomorrow")}</Chip>
          <Chip onClick={() => act("no_reply")}>{t("agent.replyNoReply", "No reply needed")}</Chip>
        </Chips>
      )}
    </article>
  );
}

// --------------------------------------------------------------- messages

function AwaitingBlock({ events, compact }: { events: AgentEvent[]; compact: boolean }) {
  const { t } = useTranslation();
  const open = events.filter((event) => !event.resolution);
  return (
    <AgentMessage compact={compact} wide>
      <Bubble>
        <p>
          {events.length === 1
            ? t("agent.awaitingOneLead", "{{from}} seems to be waiting for your reply:", { from: str(events[0].payload.from) })
            : t("agent.awaitingManyLead", "{{n}} messages seem to be waiting for your reply:", { n: events.length })}
        </p>
      </Bubble>
      <div className="agent-mail-list">
        {events.map((event) => <MailCard key={event.id} event={event} />)}
      </div>
      {open.length === 0 && events.length > 1 && (
        <p className="agent-msg__note">{t("agent.allHandled", "All handled.")}</p>
      )}
    </AgentMessage>
  );
}

function AgainBlock({ event, compact }: { event: AgentEvent; compact: boolean }) {
  const { t } = useTranslation();
  return (
    <AgentMessage compact={compact} wide>
      <Bubble>
        <p>{t("agent.againLead", "As you asked, a reminder: “{{subject}}” still has no reply.", { subject: str(event.payload.subject) })}</p>
      </Bubble>
      <div className="agent-mail-list"><MailCard event={event} /></div>
    </AgentMessage>
  );
}

function Briefing({ event, compact }: { event: AgentEvent; compact: boolean }) {
  const { t, i18n } = useTranslation();
  const open = useOpenMessage();
  const p = event.payload;
  const awaiting = Array.isArray(p.awaitingReply) ? (p.awaitingReply as Array<Record<string, unknown>>) : [];
  const deadlines = Array.isArray(p.deadlines) ? (p.deadlines as Array<Record<string, unknown>>) : [];
  const stats: Array<{ key: string; value: number; label: string; alert?: boolean }> = [
    { key: "important", value: num(p.importantUnread), label: t("agent.statImportant", "Important unread") },
    { key: "awaiting", value: awaiting.length, label: t("agent.statAwaiting", "Waiting on you") },
    { key: "followups", value: num(p.followUpCount), label: t("agent.statFollowUps", "Following"), alert: num(p.overdueCount) > 0 },
    { key: "digest", value: num(p.digestCount), label: t("agent.statDigest", "In the digest") },
  ];
  const quiet = stats[0].value === 0 && awaiting.length === 0 && num(p.overdueCount) === 0 && deadlines.length === 0;
  const summary = typeof p.summary === "string" ? p.summary : null;
  return (
    <AgentMessage compact={compact} wide>
      <Bubble tone="card">
        <div className="agent-brief__head">
          <Sun size={16} aria-hidden="true" />
          <span className="agent-brief__title">{t("agent.briefingTitle", "Daily briefing")}</span>
          <span className="agent-brief__date">{formatDate(event.createdAt, { weekday: "short", month: "short", day: "numeric" }, i18n.language)}</span>
        </div>
        <p className="agent-brief__lead">
          {greeting(t, event.createdAt)}{" "}
          {quiet ? t("agent.briefingQuiet", "Nothing in your mail needs you today.") : t("agent.briefingLead", "Here's where your mail stands:")}
        </p>
        <div className="agent-brief__stats">
          {stats.map((stat) => (
            <div key={stat.key} className={`agent-brief__stat${stat.value === 0 ? " is-zero" : ""}${stat.alert ? " is-alert" : ""}`}>
              <span className="agent-brief__value">{stat.value}</span>
              <span className="agent-brief__label">{stat.label}</span>
            </div>
          ))}
        </div>
        {num(p.overdueCount) > 0 && (
          <p className="agent-brief__note">
            {t("agent.briefingOverdue", { count: num(p.overdueCount), defaultValue: "{{count}} follow-ups got no reply by their deadline." })}
          </p>
        )}
        {summary && <p className="agent-ai-text">{summary}</p>}
        {awaiting.length > 0 && (
          <ul className="agent-brief__list" aria-label={t("agent.statAwaiting", "Waiting on you")}>
            {awaiting.map((item) => (
              <li key={str(item.messageId)}>
                <button type="button" onClick={() => open(str(item.accountId), str(item.messageId))}>
                  <span className="agent-brief__from">{str(item.from)}</span>
                  <span className="agent-brief__subject">{str(item.subject) || t("inbox.noSubject", "(no subject)")}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {deadlines.length > 0 && (
          <ul className="agent-brief__list">
            {deadlines.map((item) => (
              <li key={`${str(item.messageId)}:${str(item.at)}`}>
                <button type="button" onClick={() => open(str(item.accountId), str(item.messageId))}>
                  <span className="agent-brief__from">{formatDate(num(item.at), { month: "short", day: "numeric" }, i18n.language)}</span>
                  <span className="agent-brief__subject">{str(item.what)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </Bubble>
    </AgentMessage>
  );
}

function OverdueBlock({ event, compact }: { event: AgentEvent; compact: boolean }) {
  const { t, i18n } = useTranslation();
  const { resolve } = useAgentMutations();
  const title = str(event.payload.title);
  const resolution = event.resolution;
  const act = (action: AgentAction, days?: number) => resolve.mutate({ id: event.id, action, days });
  return (
    <>
      <AgentMessage compact={compact}>
        <Bubble>
          <p>{t("agent.watchOverdueMsg", "Still no reply to “{{title}}”. Wait a little longer, or stop following it?", { title })}</p>
        </Bubble>
        {!resolution && event.watchId && (
          <Chips label={t("agent.quickReplies", "Quick replies")}>
            <Chip primary onClick={() => act("wait", 2)}>{t("agent.replyWaitDays", "Wait {{n}} more days", { n: 2 })}</Chip>
            <Chip onClick={() => act("wait", 7)}>{t("agent.replyWaitWeek", "Wait another week")}</Chip>
            <Chip onClick={() => act("stop")}>{t("agent.replyStop", "Stop following")}</Chip>
          </Chips>
        )}
      </AgentMessage>
      {resolution && (
        <>
          <UserReply>{resolutionText(t, resolution)}</UserReply>
          <AgentMessage>
            <Bubble>
              <p>
                {resolution.action === "stop"
                  ? t("agent.ackStop", "OK, I've stopped following “{{title}}”.", { title })
                  : t("agent.ackWait", "OK, I'll check for a reply again on {{date}}.", {
                    date: formatDate(resolution.until ?? nowSeconds(), { weekday: "short", month: "short", day: "numeric" }, i18n.language),
                  })}
              </p>
            </Bubble>
          </AgentMessage>
        </>
      )}
    </>
  );
}

function ReplyBlock({ event, compact }: { event: AgentEvent; compact: boolean }) {
  const { t } = useTranslation();
  const open = useOpenMessage();
  return (
    <AgentMessage compact={compact}>
      <Bubble>
        <p>{t("agent.watchReplyMsg", "{{from}} replied to “{{title}}”, the thread you're following.", { from: str(event.payload.from), title: str(event.payload.title) })}</p>
      </Bubble>
      <Chips label={t("agent.quickReplies", "Quick replies")}>
        <Chip primary onClick={() => open(event.accountId, event.messageId)}>{t("agent.openMail", "Open mail")}</Chip>
      </Chips>
    </AgentMessage>
  );
}

function DeadlineBlock({ event, compact }: { event: AgentEvent; compact: boolean }) {
  const { t } = useTranslation();
  const open = useOpenMessage();
  return (
    <AgentMessage compact={compact}>
      <Bubble>
        <p>{t("agent.deadlineMsg", "Heads-up: {{what}} ({{subject}}).", { what: str(event.payload.what), subject: str(event.payload.subject) })}</p>
      </Bubble>
      <Chips label={t("agent.quickReplies", "Quick replies")}>
        <Chip primary onClick={() => open(event.accountId, event.messageId)}>{t("agent.openMail", "Open mail")}</Chip>
      </Chips>
    </AgentMessage>
  );
}

function BlockView({ block, compact }: { block: Block; compact: boolean }) {
  if (block.kind === "awaiting") return <AwaitingBlock events={block.events} compact={compact} />;
  const { event } = block;
  switch (event.kind) {
    case "briefing":
      return <Briefing event={event} compact={compact} />;
    case "watch_reply":
      return <ReplyBlock event={event} compact={compact} />;
    case "watch_overdue":
      return <OverdueBlock event={event} compact={compact} />;
    case "awaiting_reply":
      return <AgainBlock event={event} compact={compact} />;
    case "deadline":
      return <DeadlineBlock event={event} compact={compact} />;
    default:
      return null;
  }
}

/** Composer. Chat needs a configured model (#107, M3). */
function Composer() {
  const { t } = useTranslation();
  return (
    <div className="agent-composer">
      <div className="agent-composer__box is-disabled">
        <textarea
          className="agent-composer__input"
          rows={1}
          disabled
          placeholder={t("agent.composerPlaceholder", "Ask Pebble about your mail…")}
          aria-label={t("agent.composerPlaceholder", "Ask Pebble about your mail…")}
          aria-describedby="agent-composer-hint"
        />
        <IconButton label={t("agent.send", "Send")} icon={<ArrowUp size={16} />} variant="secondary" disabled />
      </div>
      <p id="agent-composer-hint" className="agent-composer__hint">
        {t("agent.composerNeedsModel", "Chat becomes available once an AI model is set up. Pebble's reminders work without one.")}
      </p>
    </div>
  );
}

// ------------------------------------------------------------------ thread

/** The conversation: Pebble's messages, oldest first, then the composer. */
export default function AgentThread() {
  const { t, i18n } = useTranslation();
  const { data: events = [], isLoading } = useAgentEvents();
  const { markRead } = useAgentMutations();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [atBottom, setAtBottom] = useState(true);
  const blocks = useMemo(() => buildBlocks(events), [events]);
  const hasUnread = events.some((event) => !event.readAt);
  // Where the "new" divider goes: frozen when the view opens so it doesn't
  // vanish the moment the messages are marked read.
  const dividerAt = useRef<number | null>(null);
  if (dividerAt.current === null && hasUnread) {
    dividerAt.current = Math.min(...events.filter((event) => !event.readAt).map((event) => event.createdAt));
  }

  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (node && atBottom) node.scrollTop = node.scrollHeight;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [blocks.length, events]);

  useEffect(() => {
    if (!hasUnread) return;
    const timer = window.setTimeout(() => markRead.mutate(), 2500);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasUnread]);

  const onScroll = () => {
    const node = scrollRef.current;
    if (!node) return;
    setAtBottom(node.scrollHeight - node.scrollTop - node.clientHeight < 48);
  };
  const jumpToLatest = () => {
    const node = scrollRef.current;
    node?.scrollTo({ top: node.scrollHeight, behavior: "smooth" });
  };

  const dayKey = (ts: number) => new Date(ts * 1000).toDateString();
  let previous: number | null = null;
  let dividerShown = false;
  return (
    <section className="agent-thread" aria-label={t("agent.conversation", "Conversation with Pebble")}>
      <div ref={scrollRef} className="agent-thread__scroll scroll-region" role="log" aria-live="polite" onScroll={onScroll}>
        <div className="agent-thread__inner">
          <AgentMessage>
            <Bubble>
              <p>
                {t(
                  "agent.intro",
                  "Hi, I'm Pebble. I'll let you know here when someone replies to mail you're following, when mail is waiting for your reply, and every morning with a briefing.",
                )}
              </p>
            </Bubble>
          </AgentMessage>
          {!isLoading && blocks.length === 0 && (
            <p className="agent-thread__empty">{t("agent.activityEmpty", "Nothing yet.")}</p>
          )}
          {blocks.map((block) => {
            const separator = previous === null || dayKey(previous) !== dayKey(block.at) || block.at - previous > 2 * 3600;
            const compact = !separator && previous !== null && block.at - previous < 5 * 60;
            const divider = !dividerShown && dividerAt.current !== null && block.at >= dividerAt.current;
            if (divider) dividerShown = true;
            previous = block.kind === "awaiting" ? block.events[block.events.length - 1].createdAt : block.at;
            const key = block.kind === "awaiting" ? block.events[0].id : block.event.id;
            return (
              <div key={key} className="agent-thread__item">
                {divider && <div className="agent-thread__divider" role="separator">{t("agent.newMessages", "New")}</div>}
                {separator && !divider && <div className="agent-thread__separator">{dayLabel(t, block.at, i18n.language)}</div>}
                <BlockView block={block} compact={compact && !divider} />
              </div>
            );
          })}
        </div>
      </div>
      {!atBottom && (
        <button type="button" className="agent-thread__jump" onClick={jumpToLatest} aria-label={t("agent.jumpLatest", "Jump to the latest message")}>
          <ArrowDown size={16} />
        </button>
      )}
      <Composer />
    </section>
  );
}
