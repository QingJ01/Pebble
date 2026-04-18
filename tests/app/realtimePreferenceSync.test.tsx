import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setRealtimePreference } from "../../src/lib/api";
import { useRealtimePreferenceSync } from "../../src/app/useRealtimePreferenceSync";

const mocks = vi.hoisted(() => ({
  realtimeMode: "battery" as "realtime" | "balanced" | "battery" | "manual",
}));

vi.mock("../../src/lib/api", () => ({
  setRealtimePreference: vi.fn(() => Promise.resolve()),
}));

vi.mock("../../src/stores/ui.store", () => ({
  useUIStore: (selector: (state: { realtimeMode: typeof mocks.realtimeMode }) => unknown) =>
    selector({ realtimeMode: mocks.realtimeMode }),
}));

describe("useRealtimePreferenceSync", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.realtimeMode = "battery";
  });

  it("sends the current realtime preference to the backend", () => {
    renderHook(() => useRealtimePreferenceSync());

    expect(setRealtimePreference).toHaveBeenCalledWith("battery");
  });
});
