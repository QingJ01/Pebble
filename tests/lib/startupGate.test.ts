import { describe, expect, it, vi } from "vitest";
import { waitForStartup, type StartupStage } from "@/lib/startupGate";
import { splashHoldMs, SPLASH_MIN_VISIBLE_MS } from "@/App";

describe("waitForStartup", () => {
  it("reports each new stage and resolves when ready", async () => {
    const stages: StartupStage[] = [
      { stage: "migrating", done: 0, total: 2 },
      { stage: "migrating", done: 0, total: 2 },
      { stage: "migrating", done: 1, total: 2 },
      { stage: "ready" },
    ];
    const invoke = vi.fn(async () => stages.shift());
    const seen: StartupStage[] = [];
    await waitForStartup((s) => seen.push(s), invoke, 0);
    expect(seen).toEqual([
      { stage: "migrating", done: 0, total: 2 },
      { stage: "migrating", done: 1, total: 2 },
      { stage: "ready" },
    ]);
  });

  it("rejects with the backend message when startup failed", async () => {
    const invoke = vi.fn(async () => ({ stage: "failed", message: "disk full" }));
    await expect(waitForStartup(() => {}, invoke, 0)).rejects.toThrow("disk full");
  });

  it("treats a backend without the command as ready", async () => {
    const invoke = vi.fn(async () => { throw new Error("unknown command"); });
    await expect(waitForStartup(() => {}, invoke, 0)).resolves.toBeUndefined();
  });
});

describe("splashHoldMs", () => {
  it("holds the splash until its spring-in has finished", () => {
    expect(splashHoldMs(1_000, 1_000)).toBe(SPLASH_MIN_VISIBLE_MS);
    expect(splashHoldMs(1_300, 1_000)).toBe(SPLASH_MIN_VISIBLE_MS - 300);
    expect(splashHoldMs(5_000, 1_000)).toBe(0);
    expect(splashHoldMs(5_000, undefined)).toBe(0);
  });
});
