import { describe, expect, it } from "vitest";
import { appLocale, formatDate, formatDateTime, formatListDate } from "../../src/lib/formatDate";

describe("formatDate helpers", () => {
  const ts = Date.UTC(2026, 2, 5, 12, 0) / 1000;

  it("maps the app language to a locale", () => {
    expect(appLocale("zh")).toBe("zh-CN");
    expect(appLocale("en")).toBe("en");
  });

  it("formats in the app language rather than the system locale", () => {
    expect(formatDate(ts, { year: "numeric", month: "long", timeZone: "UTC" }, "zh")).toBe("2026年3月");
    expect(formatDate(ts, { year: "numeric", month: "long", timeZone: "UTC" }, "en")).toBe("March 2026");
    expect(formatDateTime(new Date(ts * 1000), { hour: "2-digit", minute: "2-digit", hour12: false, timeZone: "UTC" }, "en")).toBe("12:00");
  });

  it("shows only the time for today in list dates", () => {
    const now = new Date(2026, 2, 5, 18, 0);
    const today = new Date(2026, 2, 5, 9, 30);
    expect(formatListDate(today, "en", now)).toMatch(/09:30|9:30/);
    expect(formatListDate(new Date(2026, 1, 1, 9, 30), "zh", now)).toBe("2月1日");
  });
});
