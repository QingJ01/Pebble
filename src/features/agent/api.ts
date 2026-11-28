import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

/** Mirrors `pebble_store::agent::ThreadWatch`. */
export interface ThreadWatch {
  id: string;
  accountId: string;
  threadId: string | null;
  subjectKey: string;
  participants: string[];
  anchorMessageId: string | null;
  title: string;
  status: "active" | "paused" | "done" | "cancelled";
  source: string;
  conversationId: string | null;
  createdAt: number;
  deadline: number | null;
  nudgedAt: number | null;
  matchedMessageId: string | null;
  matchedAt: number | null;
  updatedAt: number;
}

export type AgentEventKind = "watch_reply" | "watch_overdue" | "awaiting_reply" | "briefing" | "deadline" | string;

/** Mirrors `pebble_store::agent::AgentEvent`. */
export interface AgentEvent {
  id: string;
  kind: AgentEventKind;
  dedupeKey: string;
  accountId: string | null;
  messageId: string | null;
  threadId: string | null;
  watchId: string | null;
  conversationId: string | null;
  payload: Record<string, unknown>;
  createdAt: number;
  readAt: number | null;
  dismissedAt: number | null;
  /** "pending" (waiting for a desktop notification), "pushed" or "in_app". */
  pushState: "pending" | "pushed" | "in_app";
  pushedAt: number | null;
  /** The user's quick reply (or Pebble's own settlement, action "answered"). */
  resolution: AgentResolution | null;
}

export type AgentAction = "wait" | "stop" | "later" | "no_reply";

export interface AgentResolution {
  action: AgentAction | "answered";
  days?: number;
  until?: number;
  at?: number;
}

export const AGENT_EVENTS_CHANGED = "agent:events-changed";

export const agentKeys = {
  all: ["agent"] as const,
  events: ["agent", "events"] as const,
  unread: ["agent", "unread"] as const,
  watches: ["agent", "watches"] as const,
  watchFor: (accountId: string, threadId: string) => ["agent", "watch-for", accountId, threadId] as const,
};

export interface CreateWatchInput {
  accountId: string;
  messageId?: string;
  subject?: string;
  participants?: string[];
  deadline?: number | null;
  anchor?: string;
}

export const createThreadWatch = (input: CreateWatchInput) =>
  invoke<ThreadWatch>("create_thread_watch", {
    accountId: input.accountId,
    messageId: input.messageId ?? null,
    subject: input.subject ?? null,
    participants: input.participants ?? null,
    deadline: input.deadline ?? null,
    anchor: input.anchor ?? null,
  });
export const listThreadWatches = () => invoke<ThreadWatch[]>("list_thread_watches");
export const getThreadWatchFor = (accountId: string, threadId: string) =>
  invoke<ThreadWatch | null>("get_thread_watch_for", { accountId, threadId });
export const updateThreadWatch = (
  id: string,
  change: { status?: "active" | "paused" | "cancelled"; deadline?: number; clearDeadline?: boolean },
) =>
  invoke<ThreadWatch | null>("update_thread_watch", {
    id,
    status: change.status ?? null,
    deadline: change.deadline ?? null,
    clearDeadline: change.clearDeadline ?? null,
  });
export const cancelSendFollowUp = (anchor: string) => invoke<number>("cancel_send_follow_up", { anchor });
export const listAgentEvents = (limit = 100) => invoke<AgentEvent[]>("list_agent_events", { limit, before: null });
export const countUnreadAgentEvents = () => invoke<number>("count_unread_agent_events");
export const markAgentEventsRead = () => invoke<number>("mark_agent_events_read");
export const dismissAgentEvent = (id: string) => invoke<void>("dismiss_agent_event", { id });
export const resolveAgentEvent = (id: string, action: AgentAction, days?: number) =>
  invoke<AgentEvent | null>("resolve_agent_event", { id, action, days: days ?? null });

/** Refetch agent queries when the backend reports a change. */
export function useAgentEventsInvalidation() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen(AGENT_EVENTS_CHANGED, () => {
      void queryClient.invalidateQueries({ queryKey: agentKeys.all });
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [queryClient]);
}

export function useAgentEvents() {
  return useQuery({ queryKey: agentKeys.events, queryFn: () => listAgentEvents() });
}

export function useUnreadAgentEvents() {
  return useQuery({ queryKey: agentKeys.unread, queryFn: countUnreadAgentEvents });
}

export function useThreadWatches() {
  return useQuery({ queryKey: agentKeys.watches, queryFn: listThreadWatches });
}

export function useThreadWatchFor(accountId: string | null | undefined, threadId: string | null | undefined) {
  return useQuery({
    queryKey: agentKeys.watchFor(accountId ?? "", threadId ?? ""),
    queryFn: () => getThreadWatchFor(accountId!, threadId!),
    enabled: !!accountId && !!threadId,
  });
}

export function useAgentMutations() {
  const queryClient = useQueryClient();
  const refresh = () => queryClient.invalidateQueries({ queryKey: agentKeys.all });
  return {
    createWatch: useMutation({ mutationFn: createThreadWatch, onSettled: refresh }),
    updateWatch: useMutation({
      mutationFn: ({ id, change }: { id: string; change: Parameters<typeof updateThreadWatch>[1] }) => updateThreadWatch(id, change),
      onSettled: refresh,
    }),
    dismiss: useMutation({ mutationFn: dismissAgentEvent, onSettled: refresh }),
    resolve: useMutation({
      mutationFn: ({ id, action, days }: { id: string; action: AgentAction; days?: number }) => resolveAgentEvent(id, action, days),
      onSettled: refresh,
    }),
    markRead: useMutation({ mutationFn: markAgentEventsRead, onSettled: refresh }),
  };
}

const DAY = 24 * 60 * 60;

/** Deadline `days` from now, in seconds. */
export function deadlineInDays(days: number, now = Date.now()): number {
  return Math.floor(now / 1000) + Math.round(days * DAY);
}
