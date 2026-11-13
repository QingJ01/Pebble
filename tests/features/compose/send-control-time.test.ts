import { afterEach, describe, expect, it } from "vitest";
import { countdownSeconds, localScheduleValue, scheduledInstant } from "../../../src/features/compose/send-control-time";
import { loadUndoSeconds, saveUndoSeconds } from "../../../src/features/compose/send-controls-settings";

const originalTZ = process.env.TZ;
afterEach(() => { if (originalTZ === undefined) delete process.env.TZ; else process.env.TZ = originalTZ; localStorage.clear(); });

describe("local schedule instants", () => {
  it("validates future bounds and rejects normalized invalid dates", () => {
    const now = new Date(2026, 0, 1).getTime();
    expect(scheduledInstant("2026-02-30T12:00", now)).toBeNull();
    expect(scheduledInstant("2026-01-01T00:00", now)).toBeNull();
    expect(scheduledInstant("2028-01-01T00:00", now)).toBeNull();
    expect(scheduledInstant("not-a-date", now)).toBeNull();
    expect(scheduledInstant("2026-06-12T09:30", now)).not.toBeNull();
  });
  it("rejects the DST gap and explicitly uses the earlier repeated hour", () => {
    process.env.TZ = "America/New_York";
    const now = Date.parse("2026-01-01T00:00:00Z");
    expect(scheduledInstant("2026-03-08T02:30", now)).toBeNull();
    expect(scheduledInstant("2026-11-01T01:30", now)).toBe(Date.parse("2026-11-01T05:30:00Z") / 1000);
  });
  it("stores UTC unchanged while the current timezone only affects display", () => {
    const instant = Date.parse("2026-11-01T05:30:00Z") / 1000;
    process.env.TZ = "America/New_York";
    expect(localScheduleValue(instant)).toBe("2026-11-01T01:30");
    process.env.TZ = "Asia/Shanghai";
    expect(localScheduleValue(instant)).toBe("2026-11-01T13:30");
    expect(instant).toBe(Date.parse("2026-11-01T05:30:00Z") / 1000);
  });
  it("counts down against an instant, rounds up, and never shows negative seconds", () => {
    expect(countdownSeconds(100, 90001)).toBe(10);
    expect(countdownSeconds(100, 99999)).toBe(1);
    expect(countdownSeconds(100, 100000)).toBe(0);
    expect(countdownSeconds(100, 101000)).toBe(0);
  });
  it("defaults to ten seconds and persists disabling without persisting mail content", () => {
    expect(loadUndoSeconds()).toBe(10);
    saveUndoSeconds(0); expect(loadUndoSeconds()).toBe(0);
    saveUndoSeconds(30); expect(loadUndoSeconds()).toBe(30);
    saveUndoSeconds(-1); expect(loadUndoSeconds()).toBe(30);
    localStorage.setItem("pebble-undo-send-seconds", "garbage"); expect(loadUndoSeconds()).toBe(10);
    expect(localStorage.length).toBe(1);
  });
});
