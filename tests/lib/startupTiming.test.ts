import { afterEach, describe, expect, it, vi } from "vitest";
import {
  formatStartupTiming,
  getStartupElapsedMs,
  logStartupTiming,
} from "../../src/lib/startupTiming";

describe("startupTiming", () => {
  afterEach(() => {
    delete (window as unknown as { __splashStart?: number }).__splashStart;
    vi.restoreAllMocks();
  });

  it("calculates elapsed time from the splash start timestamp", () => {
    (window as unknown as { __splashStart: number }).__splashStart = 1_000;

    expect(getStartupElapsedMs(1_375)).toBe(375);
  });

  it("formats startup timing messages consistently", () => {
    expect(formatStartupTiming("frontend entry loaded", 42)).toBe(
      "[startup] frontend entry loaded: 42ms since splash start",
    );
  });

  it("logs startup timing messages", () => {
    const info = vi.spyOn(console, "info").mockImplementation(() => {});
    (window as unknown as { __splashStart: number }).__splashStart = 2_000;

    const elapsed = logStartupTiming("main window shown", 2_320);

    expect(elapsed).toBe(320);
    expect(info).toHaveBeenCalledWith("[startup] main window shown: 320ms since splash start");
  });
});
