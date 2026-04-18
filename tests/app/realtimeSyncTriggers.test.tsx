import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useRealtimeSyncTriggers } from "../../src/app/useRealtimeSyncTriggers";
import { triggerSync } from "../../src/lib/api";

vi.mock("../../src/lib/api", () => ({
  triggerSync: vi.fn(() => Promise.resolve()),
}));

vi.mock("../../src/stores/mail.store", () => ({
  useMailStore: (selector: (s: { activeAccountId: string }) => unknown) =>
    selector({ activeAccountId: "account-1" }),
}));

vi.mock("../../src/stores/ui.store", () => ({
  useUIStore: (selector: (s: { networkStatus: "online" }) => unknown) =>
    selector({ networkStatus: "online" }),
}));

describe("useRealtimeSyncTriggers", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("triggers sync when the window regains focus", () => {
    renderHook(() => useRealtimeSyncTriggers());

    act(() => {
      window.dispatchEvent(new Event("focus"));
    });

    expect(triggerSync).toHaveBeenCalledWith("account-1", "window_focus");
  });

  it("notifies the backend when the window loses focus", () => {
    renderHook(() => useRealtimeSyncTriggers());

    act(() => {
      window.dispatchEvent(new Event("blur"));
    });

    expect(triggerSync).toHaveBeenCalledWith("account-1", "window_blur");
  });

  it("triggers sync when the app is online with an active account", () => {
    renderHook(() => useRealtimeSyncTriggers());

    expect(triggerSync).toHaveBeenCalledWith("account-1", "network_online");
  });
});
