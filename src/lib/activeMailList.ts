/**
 * The mail list currently on screen, for global J/K navigation.
 *
 * The query cache can hold several message/thread lists at once (other folders,
 * the unified inbox with and without an account filter), so "the first cached
 * list" is not necessarily the visible one. The inbox view registers what it
 * renders here; keyboard navigation reads it first.
 */
import type { MessageSummary, ThreadSummary } from "@/lib/api";

let activeMessages: MessageSummary[] | null = null;
let activeThreads: ThreadSummary[] | null = null;

export function setActiveMailList(list: { messages: MessageSummary[] | null; threads: ThreadSummary[] | null }) {
  activeMessages = list.messages;
  activeThreads = list.threads;
}

export function clearActiveMailList() {
  activeMessages = null;
  activeThreads = null;
}

export function getActiveMessages(): MessageSummary[] | null {
  return activeMessages;
}

export function getActiveThreads(): ThreadSummary[] | null {
  return activeThreads;
}
