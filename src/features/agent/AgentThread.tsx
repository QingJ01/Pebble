import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { ArrowDown, ArrowUp, ArrowUpRight, BellRing, Check, FileText, Square, Sun } from "lucide-react";
import IconButton from "@/components/ui/IconButton";
import { formatDate, formatDateTime, formatTime } from "@/lib/formatDate";
import iconUrl from "@/assets/app-icon.png";
import {
  useAgentEvents,
  useAgentMutations,
  type AgentAction,
  type AgentEvent,
  type AgentResolution,
} from "./api";
import { useOpenMessage, useReplyToMessage } from "./useOpenMessage";
import { useAgentModel } from "./model-api";
import { useAccountsQuery } from "@/hooks/queries";
import { useMailStore } from "@/stores/mail.store";
import { useComposeStore } from "@/stores/compose.store";
import { getMessage } from "@/lib/api";
import {
  useChat,
  useChatInvalidation,
  useChatMutations,
  useChatProgress,
  useRunningChats,
  type ChatDraft,
  type ChatEntry,
  type ChatProgress,
} from "./chat-api";
import { useUIStore } from "@/stores/ui.store";

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

interface MailRef {
  accountId?: string | null;
  messageId?: string | null;
  from: string;
  fromAddress?: string;
  subject: string;
  snippet?: string;
}

/** A clickable mail inside a message: opens it in the inbox. */
function MailCard({
  mail,
  meta,
  why,
  unread,
  settled,
  children,
}: {
  mail: MailRef;
  meta?: ReactNode;
  why?: string | null;
  unread?: boolean;
  settled?: boolean;
  children?: ReactNode;
}) {
  const { t } = useTranslation();
  const open = useOpenMessage();
  const canOpen = !!mail.messageId;
  const body = (
    <>
      <span className="agent-mail__avatar" style={{ ["--avatar-hue" as string]: hue(mail.fromAddress || mail.from) }} aria-hidden="true">
        {initials(mail.from)}
      </span>
      <span className="agent-mail__text">
        <span className="agent-mail__top">
          {unread && <span className="agent-mail__unread" aria-label={t("agent.unread", "New")} />}
          <span className="agent-mail__from">{mail.from}</span>
          {meta && <span className="agent-mail__time">{meta}</span>}
        </span>
        <span className="agent-mail__subject">
          {mail.subject || t("inbox.noSubject", "(no subject)")}
          {canOpen && <ArrowUpRight size={13} className="agent-mail__open-icon" aria-hidden="true" />}
        </span>
        {mail.snippet && <span className="agent-mail__snippet">{mail.snippet}</span>}
        {why && <span className="agent-mail__why agent-ai-text">{why}</span>}
      </span>
    </>
  );
  return (
    <article className={`agent-mail${settled ? " is-settled" : ""}`}>
      {canOpen ? (
        <button
          type="button"
          className="agent-mail__main"
          onClick={() => open(mail.accountId, mail.messageId)}
          title={t("agent.openInInbox", "Open in the inbox")}
        >
          {body}
        </button>
      ) : (
        <div className="agent-mail__main is-static">{body}</div>
      )}
      {children}
    </article>
  );
}

function mailOf(event: AgentEvent): MailRef {
  const p = event.payload;
  return {
    accountId: event.accountId,
    messageId: event.messageId,
    from: str(p.from) || str(p.fromAddress),
    fromAddress: str(p.fromAddress),
    subject: str(p.subject),
    snippet: str(p.snippet),
  };
}

/** A "waiting for your reply" mail with its quick replies or outcome. */
function AwaitingCard({ event }: { event: AgentEvent }) {
  const { t } = useTranslation();
  const reply = useReplyToMessage();
  const open = useOpenMessage();
  const { resolve } = useAgentMutations();
  const resolution = event.resolution;
  const act = (action: AgentAction, days?: number) => resolve.mutate({ id: event.id, action, days });
  return (
    <MailCard
      mail={mailOf(event)}
      meta={t("agent.waitingFor", "waiting {{time}}", { time: waitedLabel(t, nowSeconds() - num(event.payload.receivedAt)) })}
      why={typeof event.payload.why === "string" ? event.payload.why : null}
      unread={!event.readAt && !resolution}
      settled={!!resolution}
    >
      {resolution ? (
        <div className="agent-mail__status">
          <Check size={13} aria-hidden="true" />
          {resolution.action === "answered" ? t("agent.settledAnswered", "You replied") : resolutionText(t, resolution)}
        </div>
      ) : (
        <Chips label={t("agent.quickReplies", "Quick replies")}>
          <Chip
            primary
            onClick={() => {
              void reply(event.accountId, event.messageId).then((ok) => {
                if (!ok) open(event.accountId, event.messageId);
              });
            }}
          >
            {t("agent.reply", "Reply")}
          </Chip>
          <Chip onClick={() => act("later", 1)}>{t("agent.replyLater", "Remind me tomorrow")}</Chip>
          <Chip onClick={() => act("no_reply")}>{t("agent.replyNoReply", "No reply needed")}</Chip>
        </Chips>
      )}
    </MailCard>
  );
}

/** A compact clickable mail line (briefing lists). */
function MailRow({ item }: { item: Record<string, unknown> }) {
  const { t, i18n } = useTranslation();
  const open = useOpenMessage();
  const from = str(item.from) || str(item.fromAddress);
  return (
    <li>
      <button type="button" className="agent-row" onClick={() => open(str(item.accountId), str(item.messageId))}>
        <span className="agent-mail__avatar is-small" style={{ ["--avatar-hue" as string]: hue(str(item.fromAddress) || from) }} aria-hidden="true">
          {initials(from)}
        </span>
        <span className="agent-row__from">{from}</span>
        <span className="agent-row__subject">{str(item.subject) || t("inbox.noSubject", "(no subject)")}</span>
        {num(item.receivedAt) > 0 && <span className="agent-row__time">{formatTime(num(item.receivedAt), undefined, i18n.language)}</span>}
        <ArrowUpRight size={13} className="agent-row__icon" aria-hidden="true" />
      </button>
    </li>
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
        {events.map((event) => <AwaitingCard key={event.id} event={event} />)}
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
      <div className="agent-mail-list"><AwaitingCard event={event} /></div>
    </AgentMessage>
  );
}

function Briefing({ event, compact }: { event: AgentEvent; compact: boolean }) {
  const { t, i18n } = useTranslation();
  const open = useOpenMessage();
  const p = event.payload;
  const awaiting = Array.isArray(p.awaitingReply) ? (p.awaitingReply as Array<Record<string, unknown>>) : [];
  const deadlines = Array.isArray(p.deadlines) ? (p.deadlines as Array<Record<string, unknown>>) : [];
  const important = Array.isArray(p.important) ? (p.important as Array<Record<string, unknown>>) : [];
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
        {important.length > 0 && (
          <div className="agent-brief__section">
            <div className="agent-brief__section-title">{t("agent.statImportant", "Important unread")}</div>
            <ul className="agent-brief__list">
              {important.map((item) => <MailRow key={str(item.messageId)} item={item} />)}
            </ul>
          </div>
        )}
        {awaiting.length > 0 && (
          <div className="agent-brief__section">
            <div className="agent-brief__section-title">{t("agent.statAwaiting", "Waiting on you")}</div>
            <ul className="agent-brief__list">
              {awaiting.map((item) => <MailRow key={str(item.messageId)} item={item} />)}
            </ul>
          </div>
        )}
        {deadlines.length > 0 && (
          <div className="agent-brief__section">
            <div className="agent-brief__section-title">{t("agent.statDeadlines", "Coming up")}</div>
            <ul className="agent-brief__list">
              {deadlines.map((item) => (
                <li key={`${str(item.messageId)}:${str(item.at)}`}>
                  <button type="button" className="agent-row" onClick={() => open(str(item.accountId), str(item.messageId))}>
                    <span className="agent-row__from">{formatDate(num(item.at), { month: "short", day: "numeric" }, i18n.language)}</span>
                    <span className="agent-row__subject">{str(item.what)}</span>
                    <ArrowUpRight size={13} className="agent-row__icon" aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </Bubble>
    </AgentMessage>
  );
}

function OverdueBlock({ event, compact }: { event: AgentEvent; compact: boolean }) {
  const { t, i18n } = useTranslation();
  const { resolve } = useAgentMutations();
  const reply = useReplyToMessage();
  const participants = Array.isArray(event.payload.participants) ? (event.payload.participants as string[]) : [];
  const sentAt = num(event.payload.sentAt) || num(event.payload.createdAt);
  const title = str(event.payload.title);
  const resolution = event.resolution;
  const act = (action: AgentAction, days?: number) => resolve.mutate({ id: event.id, action, days });
  return (
    <>
      <AgentMessage compact={compact} wide={!!event.messageId}>
        <Bubble>
          <p>{t("agent.watchOverdueMsg", "Still no reply to “{{title}}”. Wait a little longer, or stop following it?", { title })}</p>
        </Bubble>
        {event.messageId && (
          <div className="agent-mail-list">
            <MailCard
              mail={{
                accountId: event.accountId,
                messageId: event.messageId,
                from: participants.length
                  ? t("agent.sentTo", "To {{who}}", { who: participants.slice(0, 2).join(", ") + (participants.length > 2 ? "…" : "") })
                  : t("agent.yourMail", "Your mail"),
                fromAddress: participants[0],
                subject: str(event.payload.subject) || title,
                snippet: str(event.payload.snippet),
              }}
              meta={sentAt ? t("agent.sentAgo", "sent {{time}} ago", { time: waitedLabel(t, nowSeconds() - sentAt) }) : undefined}
              settled={!!resolution}
            />
          </div>
        )}
        {!resolution && event.watchId && (
          <Chips label={t("agent.quickReplies", "Quick replies")}>
            <Chip primary onClick={() => act("wait", 2)}>{t("agent.replyWaitDays", "Wait {{n}} more days", { n: 2 })}</Chip>
            <Chip onClick={() => act("wait", 7)}>{t("agent.replyWaitWeek", "Wait another week")}</Chip>
            {event.messageId && (
              <Chip onClick={() => void reply(event.accountId, event.messageId)}>{t("agent.writeNudge", "Write a follow-up")}</Chip>
            )}
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
  const { t, i18n } = useTranslation();
  const reply = useReplyToMessage();
  const received = num(event.payload.receivedAt);
  return (
    <AgentMessage compact={compact} wide>
      <Bubble>
        <p>{t("agent.watchReplyMsg", "{{from}} replied to “{{title}}”, the thread you're following.", { from: str(event.payload.from), title: str(event.payload.title) })}</p>
      </Bubble>
      <div className="agent-mail-list">
        <MailCard
          mail={{ ...mailOf(event), subject: str(event.payload.subject) || str(event.payload.title) }}
          meta={received ? formatTime(received, undefined, i18n.language) : undefined}
          unread={!event.readAt}
        >
          <Chips label={t("agent.quickReplies", "Quick replies")}>
            <Chip primary onClick={() => void reply(event.accountId, event.messageId)}>{t("agent.reply", "Reply")}</Chip>
          </Chips>
        </MailCard>
      </div>
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

// ------------------------------------------------------------------ chat

/** Paragraphs, "- " / "1. " lists and **bold**; everything as text nodes. */
export function RichText({ text }: { text: string }) {
  const blocks = text.replace(/\r\n/g, "\n").split(/\n{2,}/);
  const inline = (line: string, key: string) =>
    line.split(/(\*\*[^*]+\*\*)/g).map((part, i) =>
      part.startsWith("**") && part.endsWith("**") && part.length > 4
        ? <strong key={`${key}-${i}`}>{part.slice(2, -2)}</strong>
        : <span key={`${key}-${i}`}>{part}</span>,
    );
  return (
    <>
      {blocks.map((block, index) => {
        const lines = block.split("\n").filter((line) => line.trim() !== "");
        if (lines.length === 0) return null;
        const bullet = /^\s*(?:[-*•]|\d+[.)])\s+/;
        if (lines.every((line) => bullet.test(line))) {
          const ordered = /^\s*\d/.test(lines[0]);
          const items = lines.map((line, i) => <li key={i}>{inline(line.replace(bullet, ""), `${index}-${i}`)}</li>);
          return ordered ? <ol key={index}>{items}</ol> : <ul key={index}>{items}</ul>;
        }
        return (
          <p key={index}>
            {lines.map((line, i) => (
              <span key={i}>
                {i > 0 && <br />}
                {inline(line, `${index}-${i}`)}
              </span>
            ))}
          </p>
        );
      })}
    </>
  );
}

function progressText(t: TFunction, progress: ChatProgress | undefined): string {
  switch (progress?.step) {
    case "searching":
      return progress.detail
        ? t("agent.stepSearchingFor", "Searching for “{{q}}”…", { q: progress.detail })
        : t("agent.stepSearching", "Searching your mail…");
    case "reading":
      return t("agent.stepReading", "Reading the conversation…");
    case "checking":
      return t("agent.stepChecking", "Looking through recent mail…");
    case "drafting":
      return t("agent.stepDrafting", "Writing a draft…");
    case "following":
      return t("agent.stepFollowing", "Setting up a follow-up…");
    default:
      return t("agent.stepThinking", "Thinking…");
  }
}

function Typing({ progress, onStop }: { progress?: ChatProgress; onStop: () => void }) {
  const { t } = useTranslation();
  return (
    <AgentMessage>
      <Bubble>
        <p className="agent-typing">
          <span className="agent-typing__dots" aria-hidden="true"><i /><i /><i /></span>
          {progressText(t, progress)}
        </p>
      </Bubble>
      <Chips label={t("agent.quickReplies", "Quick replies")}>
        <Chip onClick={onStop}>{t("agent.stop", "Stop")}</Chip>
      </Chips>
    </AgentMessage>
  );
}

function DraftCard({ draft }: { draft: ChatDraft }) {
  const { t } = useTranslation();
  const setActiveAccountId = useMailStore((s) => s.setActiveAccountId);
  async function openInComposer() {
    setActiveAccountId(draft.accountId);
    const original = draft.replyToMessageId ? await getMessage(draft.replyToMessageId).catch(() => null) : null;
    if (original) {
      useComposeStore.getState().openCompose("reply", original, { to: draft.to, cc: draft.cc, body: draft.body });
    } else {
      useComposeStore.getState().openCompose("new", null, { to: draft.to, cc: draft.cc, subject: draft.subject, body: draft.body });
    }
  }
  return (
    <article className="agent-draft">
      <div className="agent-draft__head">
        <FileText size={14} aria-hidden="true" />
        <span>{t("agent.draftTo", "Draft to {{who}}", { who: draft.to.join(", ") })}</span>
      </div>
      {draft.unverified && draft.unverified.length > 0 && (
        <p className="agent-draft__warn" role="note">
          {t("agent.draftUnverified", "Check the recipients: you haven't mentioned or written to {{who}}.", { who: draft.unverified.join(", ") })}
        </p>
      )}
      {draft.subject && <div className="agent-draft__subject">{draft.subject}</div>}
      <div className={`agent-draft__body${draft.body.split("\n").length > 7 || draft.body.length > 360 ? " is-long" : ""}`}>{draft.body}</div>
      <div className="agent-draft__foot">
        <Chip primary onClick={() => void openInComposer()}>{t("agent.openDraft", "Open in composer")}</Chip>
        <span className="agent-draft__note">{t("agent.draftNotSent", "Not sent. You review and send it.")}</span>
      </div>
    </article>
  );
}

function Sources({ entry }: { entry: ChatEntry }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  if (entry.used.length === 0) return null;
  return (
    <div className="agent-sources">
      <button type="button" className="agent-sources__toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
        {t("agent.sourcesCount", { count: entry.used.length, defaultValue: "Used {{count}} messages" })}
        {entry.fallback && ` · ${t("agent.fallbackNote", "searched for you (this model can't use tools)")}`}
      </button>
      {open && (
        <ul className="agent-brief__list agent-sources__list">
          {entry.used.map((mail) => (
            <MailRow
              key={mail.messageId}
              item={{
                messageId: mail.messageId,
                accountId: mail.accountId,
                from: mail.from,
                fromAddress: mail.fromAddress,
                subject: mail.subject,
              }}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function ChatAnswer({ entry, question, onRetry }: { entry: ChatEntry; question?: ChatEntry; onRetry: (text: string) => void }) {
  const { t, i18n } = useTranslation();
  const open = useOpenMessage();
  if (entry.status === "cancelled") {
    return <p className="agent-msg__note agent-msg__note--indent">{t("agent.stopped", "Stopped.")}</p>;
  }
  if (entry.status === "error") {
    return (
      <AgentMessage>
        <Bubble>
          <p className="agent-error">{t("agent.answerFailed", "I couldn't answer that: {{error}}", { error: entry.error ?? "" })}</p>
        </Bubble>
        {question && (
          <Chips label={t("agent.quickReplies", "Quick replies")}>
            <Chip primary onClick={() => onRetry(question.text)}>{t("agent.retry", "Try again")}</Chip>
          </Chips>
        )}
      </AgentMessage>
    );
  }
  return (
    <AgentMessage wide={entry.drafts.length > 0 || entry.followUps.length > 0}>
      <Bubble>
        <RichText text={entry.text || t("agent.emptyAnswer", "(no answer)")} />
      </Bubble>
      {entry.drafts.map((draft, index) => <DraftCard key={index} draft={draft} />)}
      {entry.followUps.map((follow) => (
        <button key={follow.watchId} type="button" className="agent-follow" onClick={() => open(follow.accountId, follow.messageId)}>
          <BellRing size={14} aria-hidden="true" />
          <span>
            {follow.deadline
              ? t("agent.followStartedUntil", "Following “{{title}}”, reminder on {{date}}", {
                title: follow.title,
                date: formatDate(follow.deadline, { month: "short", day: "numeric" }, i18n.language),
              })
              : t("agent.followStartedPlain", "Following “{{title}}”", { title: follow.title })}
          </span>
        </button>
      ))}
      <Sources entry={entry} />
    </AgentMessage>
  );
}

const SUGGESTIONS: Array<{ key: string; fallback: string }> = [
  { key: "agent.suggestToday", fallback: "Anything that needs me today?" },
  { key: "agent.suggestWaiting", fallback: "What's waiting for my reply?" },
  { key: "agent.suggestFind", fallback: "Find last week's mail about the contract" },
];

/** Bottom of the conversation: the composer once a model is set up,
 *  otherwise a prompt to set one up. */
function Composer({ running, onStop, showSuggestions }: { running: string[]; onStop: () => void; showSuggestions: boolean }) {
  const { t } = useTranslation();
  const { data: model, isLoading } = useAgentModel();
  const { data: accounts = [] } = useAccountsQuery();
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const setActiveView = useUIStore((s) => s.setActiveView);
  const setSettingsTab = useUIStore((s) => s.setSettingsTab);
  const { send } = useChatMutations();
  const [text, setText] = useState("");
  const [scope, setScope] = useState<string>(() => activeAccountId ?? "all");
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useLayoutEffect(() => {
    const node = inputRef.current;
    if (!node) return;
    node.style.height = "auto";
    node.style.height = `${Math.min(node.scrollHeight, 160)}px`;
  }, [text]);

  if (isLoading) return <div className="agent-composer" />;
  if (!model?.configured) {
    return (
      <div className="agent-composer">
        <div className="agent-setup">
          <div className="agent-setup__text">
            <span className="agent-setup__title">{t("agent.setupTitle", "Ask Pebble about your mail")}</span>
            <span className="agent-setup__desc">
              {t("agent.setupDesc", "Set up an AI model to search, summarise and draft replies here. Local models keep your mail on this computer.")}
            </span>
          </div>
          <button
            type="button"
            className="agent-chip is-primary"
            onClick={() => {
              setSettingsTab("assistant");
              setActiveView("settings");
            }}
          >
            {t("agent.setupAction", "Set up a model")}
          </button>
        </div>
      </div>
    );
  }

  const busy = running.length > 0;
  const submit = (value: string) => {
    const question = value.trim();
    if (!question || busy || send.isPending) return;
    send.mutate({ text: question, accountIds: scope === "all" ? null : [scope] });
    setText("");
  };

  return (
    <div className="agent-composer">
      {showSuggestions && !busy && (
        <div className="agent-suggestions" role="group" aria-label={t("agent.suggestions", "Suggestions")}>
          {SUGGESTIONS.map((item) => (
            <Chip key={item.key} onClick={() => submit(t(item.key, item.fallback))}>{t(item.key, item.fallback)}</Chip>
          ))}
        </div>
      )}
      <form
        className="agent-composer__box"
        onSubmit={(event) => {
          event.preventDefault();
          submit(text);
        }}
      >
        <textarea
          ref={inputRef}
          className="agent-composer__input"
          rows={1}
          value={text}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              submit(text);
            }
          }}
          placeholder={t("agent.composerPlaceholder", "Ask Pebble about your mail…")}
          aria-label={t("agent.composerPlaceholder", "Ask Pebble about your mail…")}
        />
        {accounts.length > 1 && (
          <select
            className="agent-composer__scope"
            value={scope}
            onChange={(event) => setScope(event.target.value)}
            aria-label={t("agent.scope", "Which accounts Pebble may read")}
          >
            <option value="all">{t("agent.scopeAll", "All accounts")}</option>
            {accounts.map((account) => (
              <option key={account.id} value={account.id}>{account.account_label || account.email}</option>
            ))}
          </select>
        )}
        {busy ? (
          <IconButton label={t("agent.stop", "Stop")} icon={<Square size={14} />} variant="secondary" onClick={onStop} />
        ) : (
          <IconButton
            type="submit"
            label={t("agent.send", "Send")}
            icon={<ArrowUp size={16} />}
            variant="secondary"
            disabled={!text.trim() || send.isPending}
          />
        )}
      </form>
    </div>
  );
}

// ------------------------------------------------------------------ thread

/** The conversation: Pebble's messages, oldest first, then the composer. */
export default function AgentThread() {
  const { t, i18n } = useTranslation();
  const { data: events = [], isLoading } = useAgentEvents();
  const { markRead } = useAgentMutations();
  useChatInvalidation();
  const { data: chat = [] } = useChat();
  const { data: running = [] } = useRunningChats();
  const progress = useChatProgress();
  const { send, cancel } = useChatMutations();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [atBottom, setAtBottom] = useState(true);
  const blocks = useMemo(() => buildBlocks(events), [events]);
  type Item =
    | { kind: "block"; at: number; key: string; block: Block }
    | { kind: "chat"; at: number; key: string; entry: ChatEntry; order: number }
    | { kind: "typing"; at: number; key: string; runId: string };
  const items = useMemo<Item[]>(() => {
    const list: Item[] = blocks.map((block) => ({
      kind: "block",
      at: block.at,
      key: block.kind === "awaiting" ? block.events[0].id : block.event.id,
      block,
    }));
    chat.forEach((entry, order) => list.push({ kind: "chat", at: entry.createdAt, key: entry.id, entry, order }));
    const answered = new Set(chat.filter((entry) => entry.role === "assistant").map((entry) => entry.runId));
    for (const runId of running) {
      if (!answered.has(runId)) list.push({ kind: "typing", at: Number.MAX_SAFE_INTEGER, key: `typing-${runId}`, runId });
    }
    return list.sort((a, b) => a.at - b.at || (a.kind === "chat" && b.kind === "chat" ? a.order - b.order : 0));
  }, [blocks, chat, running]);
  const questionFor = (runId: string | null) => chat.find((entry) => entry.role === "user" && entry.runId === runId);
  const stopAll = () => running.forEach((runId) => cancel.mutate(runId));
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
  }, [blocks.length, events, chat.length, running.length, progress]);

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
          {items.map((item) => {
            const at = item.kind === "typing" ? (previous ?? item.at) : item.at;
            const separator = item.kind !== "typing"
              && (previous === null || dayKey(previous) !== dayKey(at) || at - previous > 2 * 3600);
            const compact = item.kind === "block" && !separator && previous !== null && at - previous < 5 * 60;
            const divider = item.kind === "block" && !dividerShown && dividerAt.current !== null && at >= dividerAt.current;
            if (divider) dividerShown = true;
            if (item.kind === "block") {
              previous = item.block.kind === "awaiting" ? item.block.events[item.block.events.length - 1].createdAt : item.at;
            } else if (item.kind === "chat") {
              previous = item.at;
            }
            return (
              <div key={item.key} className="agent-thread__item" title={item.kind === "typing" ? undefined : formatDateTime(at, undefined, i18n.language)}>
                {divider && <div className="agent-thread__divider" role="separator">{t("agent.newMessages", "New")}</div>}
                {separator && !divider && <div className="agent-thread__separator">{dayLabel(t, at, i18n.language)}</div>}
                {item.kind === "block" && <BlockView block={item.block} compact={compact && !divider} />}
                {item.kind === "chat" && item.entry.role === "user" && <UserReply><RichText text={item.entry.text} /></UserReply>}
                {item.kind === "chat" && item.entry.role === "assistant" && (
                  <ChatAnswer
                    entry={item.entry}
                    question={questionFor(item.entry.runId)}
                    onRetry={(text) => send.mutate({ text, accountIds: questionFor(item.entry.runId)?.scope ?? null })}
                  />
                )}
                {item.kind === "typing" && <Typing progress={progress[item.runId]} onStop={() => cancel.mutate(item.runId)} />}
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
      <Composer running={running} onStop={stopAll} showSuggestions={chat.length === 0} />
    </section>
  );
}
