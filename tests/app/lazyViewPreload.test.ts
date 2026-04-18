import { describe, expect, it, vi } from "vitest";
import { createLazyViewPreloader, scheduleLazyViewPreload } from "../../src/app/lazyViewPreload";

describe("lazy view preloading", () => {
  it("preloads every lazy view once and reuses the in-flight preload", async () => {
    const importers = [
      vi.fn().mockResolvedValue({ default: "SettingsView" }),
      vi.fn().mockResolvedValue({ default: "ComposeView" }),
    ];
    const preload = createLazyViewPreloader(importers);

    const first = preload();
    const second = preload();

    expect(second).toBe(first);
    expect(importers[0]).toHaveBeenCalledOnce();
    expect(importers[1]).toHaveBeenCalledOnce();
    await expect(first).resolves.toEqual([
      { status: "fulfilled", value: { default: "SettingsView" } },
      { status: "fulfilled", value: { default: "ComposeView" } },
    ]);
  });

  it("schedules preload during idle time when available", () => {
    const preload = vi.fn().mockResolvedValue([]);
    const requestIdleCallback = vi.fn((callback: IdleRequestCallback) => {
      callback({ didTimeout: false, timeRemaining: () => 12 });
      return 7;
    });
    const cancelIdleCallback = vi.fn();
    const win = {
      requestIdleCallback,
      cancelIdleCallback,
      setTimeout: vi.fn(),
      clearTimeout: vi.fn(),
    } as unknown as Window;

    const cleanup = scheduleLazyViewPreload(preload, win);
    cleanup();

    expect(requestIdleCallback).toHaveBeenCalledWith(expect.any(Function), { timeout: 1200 });
    expect(preload).toHaveBeenCalledOnce();
    expect(cancelIdleCallback).toHaveBeenCalledWith(7);
  });
});
