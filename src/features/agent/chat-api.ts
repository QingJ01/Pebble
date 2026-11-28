import { useMailStore } from "@/stores/mail.store";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

export interface ChatMailRef {
  messageId: string;
  accountId: string;
  from: string;
  fromAddress: string;
  subject: string;
  date: number;
}

export interface ChatDraft {
  accountId: string;
  replyToMessageId: string | null;
  to: string[];
  cc: string[];
  subject: string;
  body: string;
  /** Recipients nobody mentioned or wrote to (could come from mail content). */
  unverified?: string[];
}

export interface ChatFollowUp {
  watchId: string;
  title: string;
  deadline: number | null;
  messageId: string;
  accountId: string;
}

/** Mirrors `commands::agent_chat::ChatEntry`. */
export interface ChatEntry {
  id: string;
  role: "user" | "assistant";
  runId: string | null;
  status: "done" | "error" | "cancelled";
  createdAt: number;
  text: string;
  scope: string[];
  used: ChatMailRef[];
  drafts: ChatDraft[];
  followUps: ChatFollowUp[];
  error: string | null;
  fallback: boolean;
}

export interface ChatProgress {
  runId: string;
  step: "thinking" | "searching" | "reading" | "drafting" | "following" | "checking" | string;
  detail: string | null;
}

export const CHAT_CHANGED = "agent:chat-changed";
export const CHAT_PROGRESS = "agent:chat-progress";

export const chatKeys = {
  list: ["agent", "chat"] as const,
  running: ["agent", "chat", "running"] as const,
};

/** `fromAccountId`: the account the user is in; new mail is drafted from it. */
export const sendChat = (text: string, accountIds: string[] | null, fromAccountId: string | null) =>
  invoke<string>("agent_chat_send", { text, accountIds, fromAccountId });
export const listChat = () => invoke<ChatEntry[]>("agent_chat_list", { limit: 200 });
export const runningChats = () => invoke<string[]>("agent_chat_running");
export const cancelChat = (runId: string) => invoke<boolean>("agent_chat_cancel", { runId });
export const clearChat = () => invoke<void>("agent_chat_clear");

export function useChat() {
  return useQuery({ queryKey: chatKeys.list, queryFn: listChat });
}

export function useRunningChats() {
  return useQuery({ queryKey: chatKeys.running, queryFn: runningChats });
}

/** Refetch the conversation when the backend changes it. */
export function useChatInvalidation() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen(CHAT_CHANGED, () => {
      void queryClient.invalidateQueries({ queryKey: chatKeys.list });
      void queryClient.invalidateQueries({ queryKey: chatKeys.running });
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

/** Latest progress per running run. */
export function useChatProgress(): Record<string, ChatProgress> {
  const [progress, setProgress] = useState<Record<string, ChatProgress>>({});
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<ChatProgress>(CHAT_PROGRESS, (event) => {
      setProgress((current) => ({ ...current, [event.payload.runId]: event.payload }));
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  return progress;
}

export function useChatMutations() {
  const queryClient = useQueryClient();
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: chatKeys.list });
    void queryClient.invalidateQueries({ queryKey: chatKeys.running });
  };
  return {
    send: useMutation({
      mutationFn: ({ text, accountIds }: { text: string; accountIds: string[] | null }) =>
        sendChat(text, accountIds, useMailStore.getState().activeAccountId ?? null),
      onSettled: refresh,
    }),
    cancel: useMutation({ mutationFn: cancelChat, onSettled: refresh }),
    clear: useMutation({ mutationFn: clearChat, onSettled: refresh }),
  };
}
