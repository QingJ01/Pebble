import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  MAX_VISIBLE_TOASTS,
  TOAST_EXIT_MS,
  resolveToastDuration,
  selectVisibleToasts,
  toastError,
  useToastStore,
} from "../../src/stores/toast.store";

const store = () => useToastStore.getState();

describe("toast store", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    store().clearToasts();
  });
  afterEach(() => {
    store().clearToasts();
    vi.useRealTimers();
  });

  it("uses per-type default durations, a 6s floor for actions and allows sticky toasts", () => {
    expect(resolveToastDuration({ type: "success" })).toBe(4000);
    expect(resolveToastDuration({ type: "info" })).toBe(4000);
    expect(resolveToastDuration({ type: "error" })).toBe(8000);
    expect(resolveToastDuration({ type: "info", duration: 2000, action: { label: "Undo", onClick: () => {} } })).toBe(6000);
    expect(resolveToastDuration({ type: "info", duration: Infinity })).toBe(Infinity);

    const id = store().addToast({ type: "success", message: "Saved" });
    vi.advanceTimersByTime(3999);
    expect(store().toasts.find((t) => t.id === id)?.exiting).toBeFalsy();
    vi.advanceTimersByTime(1);
    expect(store().toasts.find((t) => t.id === id)?.exiting).toBe(true);
    vi.advanceTimersByTime(TOAST_EXIT_MS);
    expect(store().toasts).toHaveLength(0);

    store().addToast({ type: "error", message: "Sticky", duration: Infinity });
    vi.advanceTimersByTime(60_000);
    expect(store().toasts).toHaveLength(1);
  });

  it("shows at most three toasts and starts queued timers only when they become visible", () => {
    for (let i = 0; i < 5; i += 1) store().addToast({ type: "info", message: `m${i}` });
    expect(selectVisibleToasts(store().toasts).map((t) => t.message)).toEqual(["m0", "m1", "m2"]);
    expect(MAX_VISIBLE_TOASTS).toBe(3);

    vi.advanceTimersByTime(4000 + TOAST_EXIT_MS);
    // First three expired together; the queued two have only just started.
    expect(store().toasts.map((t) => t.message)).toEqual(["m3", "m4"]);
    // They started at t=4000, so they expire at t=8000 (now t=4120).
    vi.advanceTimersByTime(3879);
    expect(store().toasts.every((t) => !t.exiting)).toBe(true);
    vi.advanceTimersByTime(1);
    expect(store().toasts.every((t) => t.exiting)).toBe(true);
  });

  it("never merges toasts marked not mergeable", () => {
    const a = store().addToast({ type: "info", message: "Undoing…", mergeable: false });
    const b = store().addToast({ type: "info", message: "Undoing…", mergeable: false });
    expect(a).not.toBe(b);
    expect(store().toasts).toHaveLength(2);
  });

  it("never merges toasts that carry an action", () => {
    const undoA = vi.fn();
    const undoB = vi.fn();
    store().addToast({ type: "success", message: "Archived", action: { label: "Undo", onClick: undoA } });
    store().addToast({ type: "success", message: "Archived", action: { label: "Undo", onClick: undoB } });
    expect(store().toasts).toHaveLength(2);
    expect(store().toasts.map((t) => t.action?.onClick)).toEqual([undoA, undoB]);
  });

  it("merges identical toasts into one with a count and restarts its timer", () => {
    const first = store().addToast({ type: "error", message: "Offline" });
    vi.advanceTimersByTime(7000);
    const second = store().addToast({ type: "error", message: "Offline" });
    expect(second).toBe(first);
    expect(store().toasts).toHaveLength(1);
    expect(store().toasts[0].count).toBe(2);
    vi.advanceTimersByTime(7000);
    expect(store().toasts[0].exiting).toBeFalsy();
    store().addToast({ type: "info", message: "Offline" });
    expect(store().toasts).toHaveLength(2);
  });

  it("pauses timers while hovered/focused and resumes with the remaining time", () => {
    store().addToast({ type: "success", message: "Saved" });
    vi.advanceTimersByTime(3000);
    store().pauseToasts();
    vi.advanceTimersByTime(10_000);
    expect(store().toasts[0].exiting).toBeFalsy();
    store().resumeToasts();
    vi.advanceTimersByTime(999);
    expect(store().toasts[0].exiting).toBeFalsy();
    vi.advanceTimersByTime(1);
    expect(store().toasts[0].exiting).toBe(true);
  });

  it("animates removal before dropping the toast", () => {
    const id = store().addToast({ type: "info", message: "Bye" });
    store().removeToast(id);
    expect(store().toasts[0].exiting).toBe(true);
    vi.advanceTimersByTime(TOAST_EXIT_MS);
    expect(store().toasts).toHaveLength(0);
  });

  it("toastError shows a localized title with the raw message as detail", () => {
    toastError({ kind: "Network", message: "dns failure" });
    const toast = store().toasts[0];
    expect(toast.type).toBe("error");
    expect(toast.message).not.toContain("[object Object]");
    expect(toast.detail).toBe("dns failure");
  });
});
