/**
 * Toast store.
 *
 *   addToast({ message, type, duration?, action?, detail? }) → id
 *   toastError(err, fallbackKey?)   // localized title + raw detail via describeError
 *   removeToast(id)                 // animated dismiss (exit = 120ms)
 *   pauseToasts() / resumeToasts()  // ToastContainer calls these on hover/focus
 *
 * Rules:
 * - At most MAX_VISIBLE_TOASTS (3) are shown; the rest wait in the queue and
 *   their timers start only once they become visible.
 * - An identical toast (same type + message + detail, no action) that is
 *   still showing is merged: `count` increments and its timer restarts.
 *   Toasts with an action are never merged (each keeps its own callback).
 * - Default durations: success/info 4000ms, warning 6000ms, error 8000ms;
 *   toasts with an action stay at least 6000ms. `duration: Infinity` = sticky.
 */
import { create } from "zustand";
import { describeError } from "@/lib/extractErrorMessage";

export type ToastType = "success" | "error" | "info" | "warning";

export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface Toast {
  id: string;
  message: string;
  type: ToastType;
  /** ms; Infinity keeps the toast until dismissed. */
  duration?: number;
  action?: ToastAction;
  /** Raw/technical text shown on demand ("Show details"). */
  detail?: string;
  /** false: never merge with an identical toast (e.g. per-operation progress). */
  mergeable?: boolean;
  /** Number of merged identical toasts (≥ 1). */
  count: number;
  /** The action undoes a mail move (the `u` vim key runs the latest one). */
  undoable?: boolean;
  /** True while the exit animation runs. */
  exiting?: boolean;
}

export type ToastInput = Omit<Toast, "id" | "count" | "exiting">;

export const MAX_VISIBLE_TOASTS = 3;
export const TOAST_EXIT_MS = 120;
export const MIN_ACTION_TOAST_MS = 6000;
export const DEFAULT_TOAST_DURATIONS: Record<ToastType, number> = {
  success: 4000,
  info: 4000,
  warning: 6000,
  error: 8000,
};

interface ToastState {
  toasts: Toast[];
  paused: boolean;
  addToast: (toast: ToastInput) => string;
  removeToast: (id: string) => void;
  pauseToasts: () => void;
  resumeToasts: () => void;
  clearToasts: () => void;
}

interface TimerEntry {
  handle: ReturnType<typeof setTimeout> | null;
  remaining: number;
  startedAt: number;
}

const timers = new Map<string, TimerEntry>();
let nextId = 0;

export function resolveToastDuration(toast: Pick<Toast, "type" | "duration" | "action">): number {
  const base = toast.duration ?? DEFAULT_TOAST_DURATIONS[toast.type] ?? 4000;
  return toast.action ? Math.max(base, MIN_ACTION_TOAST_MS) : base;
}

/** Toasts currently on screen: the first MAX_VISIBLE non-exiting ones, plus exiting ones. */
export function selectVisibleToasts(toasts: Toast[]): Toast[] {
  let shown = 0;
  return toasts.filter((toast) => {
    if (toast.exiting) return true;
    if (shown >= MAX_VISIBLE_TOASTS) return false;
    shown += 1;
    return true;
  });
}

function sameToast(a: Toast, b: ToastInput) {
  // Toasts with an action (Undo, Retry…) are never merged: each carries its own
  // callback, and merging would leave earlier actions unreachable.
  if (a.action || b.action) return false;
  if (a.mergeable === false || b.mergeable === false) return false;
  return a.type === b.type
    && a.message === b.message
    && (a.detail ?? "") === (b.detail ?? "");
}

function clearTimer(id: string) {
  const entry = timers.get(id);
  if (entry?.handle) clearTimeout(entry.handle);
  timers.delete(id);
}

export const useToastStore = create<ToastState>((set, get) => {
  function startTimer(id: string, ms: number) {
    const entry: TimerEntry = { handle: null, remaining: ms, startedAt: Date.now() };
    timers.set(id, entry);
    if (!Number.isFinite(ms) || get().paused) return;
    entry.handle = setTimeout(() => get().removeToast(id), ms);
  }

  /** Start timers for toasts that just became visible. */
  function schedule() {
    for (const toast of selectVisibleToasts(get().toasts)) {
      if (toast.exiting || timers.has(toast.id)) continue;
      startTimer(toast.id, resolveToastDuration(toast));
    }
  }

  return {
    toasts: [],
    paused: false,

    addToast: (input) => {
      const existing = get().toasts.find((t) => !t.exiting && sameToast(t, input));
      if (existing) {
        set((s) => ({
          toasts: s.toasts.map((t) => (t.id === existing.id ? { ...t, count: t.count + 1 } : t)),
        }));
        if (timers.has(existing.id)) {
          clearTimer(existing.id);
          startTimer(existing.id, resolveToastDuration(input));
        }
        return existing.id;
      }
      const id = String(++nextId);
      set((s) => ({ toasts: [...s.toasts, { ...input, id, count: 1 }] }));
      schedule();
      return id;
    },

    removeToast: (id) => {
      const toast = get().toasts.find((t) => t.id === id);
      if (!toast || toast.exiting) return;
      clearTimer(id);
      set((s) => ({ toasts: s.toasts.map((t) => (t.id === id ? { ...t, exiting: true } : t)) }));
      setTimeout(() => {
        set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
      }, TOAST_EXIT_MS);
      schedule();
    },

    pauseToasts: () => {
      if (get().paused) return;
      const now = Date.now();
      for (const entry of timers.values()) {
        if (entry.handle) {
          clearTimeout(entry.handle);
          entry.handle = null;
          entry.remaining = Math.max(0, entry.remaining - (now - entry.startedAt));
        }
      }
      set({ paused: true });
    },

    resumeToasts: () => {
      if (!get().paused) return;
      set({ paused: false });
      const now = Date.now();
      for (const [id, entry] of timers) {
        if (entry.handle || !Number.isFinite(entry.remaining)) continue;
        entry.startedAt = now;
        entry.handle = setTimeout(() => get().removeToast(id), entry.remaining);
      }
    },

    clearToasts: () => {
      for (const id of [...timers.keys()]) clearTimer(id);
      set({ toasts: [], paused: false });
    },
  };
});

/**
 * Show an error toast for any thrown value: localized actionable title by
 * PebbleError kind, raw message available behind "Show details".
 * `fallbackKey` is an i18n key describing what failed (e.g. "compose.sendError"),
 * used when the error kind has no specific guidance.
 */
export function toastError(err: unknown, fallbackKey?: string, options: Partial<ToastInput> = {}): string {
  const described = describeError(err, { fallbackKey });
  return useToastStore.getState().addToast({
    type: "error",
    message: described.title,
    detail: described.detail && described.detail !== described.title ? described.detail : undefined,
    ...options,
  });
}
