import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

/** Mirrors `pebble_notify::Mode` (snake_case). */
export type NotifyMode = "all" | "smart" | "important_only";

export interface QuietHours {
  startMinute: number;
  endMinute: number;
  allowWatch: boolean;
  allowVip: boolean;
}

/** Mirrors `smart_notify::NotifyPrefsRecord`. */
export interface NotifyPrefs {
  mode: NotifyMode;
  quietHours: QuietHours | null;
  modelAssist: boolean;
  modelReviewsBulk: boolean;
  modelSendBody: boolean;
}

export interface NotifySettings {
  defaults: NotifyPrefs;
  accounts: Record<string, NotifyPrefs>;
}

export interface SenderPref {
  account_id: string;
  pattern: string;
  kind: "vip" | "mute";
  created_at: number;
}

export type NotifyTier = "interrupt" | "digest" | "silent";

export interface NotificationEntry {
  accountId: string;
  messageId: string;
  tier: NotifyTier;
  reason: string;
  why: string | null;
  delivered: boolean;
  createdAt: number;
  seen: boolean;
  subject: string;
  fromName: string;
  fromAddress: string;
  threadId: string | null;
  date: number;
}

export const NOTIFY_CHANGED_EVENT = "notify:changed";

export const notifyKeys = {
  settings: ["notify", "settings"] as const,
  senders: ["notify", "senders"] as const,
  log: ["notify", "log"] as const,
  digest: ["notify", "digest"] as const,
  all: ["notify"] as const,
};

export const getNotifySettings = () => invoke<NotifySettings>("get_notify_settings");
export const setNotifyPrefs = (accountId: string | null, prefs: NotifyPrefs) =>
  invoke<void>("set_notify_prefs", { accountId, prefs });
export const resetAccountNotifyPrefs = (accountId: string) =>
  invoke<void>("reset_account_notify_prefs", { accountId });
export const listSenderPrefs = () => invoke<SenderPref[]>("list_sender_prefs");
export const setSenderPref = (accountId: string | null, pattern: string, kind: "vip" | "mute") =>
  invoke<string>("set_sender_pref", { accountId, pattern, kind });
export const removeSenderPref = (accountId: string | null, pattern: string) =>
  invoke<void>("remove_sender_pref", { accountId, pattern });
export const listNotificationLog = (limit = 100) =>
  invoke<NotificationEntry[]>("list_notification_log", { limit });
export const listDigest = (limit = 200) => invoke<NotificationEntry[]>("list_digest", { limit });
export const markDigestSeen = () => invoke<number>("mark_digest_seen");

/** Refetch notify queries whenever the backend reports a change. */
export function useNotifyChangedInvalidation() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen(NOTIFY_CHANGED_EVENT, () => {
      void queryClient.invalidateQueries({ queryKey: notifyKeys.all });
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

export function useNotifySettings() {
  return useQuery({ queryKey: notifyKeys.settings, queryFn: getNotifySettings });
}

export function useSenderPrefs() {
  return useQuery({ queryKey: notifyKeys.senders, queryFn: listSenderPrefs });
}

export function useNotificationLog(limit = 50) {
  return useQuery({ queryKey: [...notifyKeys.log, limit], queryFn: () => listNotificationLog(limit) });
}

export function useDigest() {
  return useQuery({ queryKey: notifyKeys.digest, queryFn: () => listDigest() });
}

/** Save prefs and refresh. */
export function useSaveNotifyPrefs() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ accountId, prefs }: { accountId: string | null; prefs: NotifyPrefs }) => setNotifyPrefs(accountId, prefs),
    onSettled: () => queryClient.invalidateQueries({ queryKey: notifyKeys.settings }),
  });
}

export function useSenderPrefMutations() {
  const queryClient = useQueryClient();
  const refresh = () => queryClient.invalidateQueries({ queryKey: notifyKeys.senders });
  return {
    set: useMutation({
      mutationFn: ({ accountId, pattern, kind }: { accountId: string | null; pattern: string; kind: "vip" | "mute" }) =>
        setSenderPref(accountId, pattern, kind),
      onSettled: refresh,
    }),
    remove: useMutation({
      mutationFn: ({ accountId, pattern }: { accountId: string | null; pattern: string }) => removeSenderPref(accountId, pattern),
      onSettled: refresh,
    }),
  };
}

/** "HH:MM" <-> minutes since midnight. */
export function minutesToTime(minutes: number): string {
  const clamped = Math.max(0, Math.min(24 * 60 - 1, Math.round(minutes)));
  return `${String(Math.floor(clamped / 60)).padStart(2, "0")}:${String(clamped % 60).padStart(2, "0")}`;
}

export function timeToMinutes(value: string): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  return hours * 60 + minutes;
}

export const DEFAULT_QUIET_HOURS: QuietHours = { startMinute: 22 * 60, endMinute: 7 * 60, allowWatch: true, allowVip: true };
