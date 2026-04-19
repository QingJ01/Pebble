import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import GeneralTab from "../../../src/features/settings/GeneralTab";
import { useUIStore } from "../../../src/stores/ui.store";

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "settings.realtimeMode": "Realtime Mode",
        "settings.realtimeModeDesc": "Choose how aggressively Pebble checks for new mail.",
        "settings.realtimeModeRealtime": "Realtime (recommended)",
        "settings.realtimeModeBalanced": "Balanced",
        "settings.realtimeModeBattery": "Battery saver",
        "settings.realtimeModeManual": "Manual only",
        "settings.realtimeModeRealtimeDesc": "IMAP uses IDLE push when supported. Other providers check about every 3 seconds while you are active.",
        "settings.realtimeModeBalancedDesc": "Checks about every 15 seconds while you are active.",
        "settings.realtimeModeBatteryDesc": "Checks about every 60 seconds while you are active and slows down in the background.",
        "settings.realtimeModeManualDesc": "Stops background checks. Use Sync now to run a single pass.",
        "settings.syncInterval": "Sync Interval",
        "settings.syncIntervalDesc": "How often to check for new messages (seconds)",
        "settings.notifications": "Notifications",
        "settings.enableNotifications": "Enable desktop notifications",
        "settings.folderCounts": "Folder Counts",
        "settings.showUnreadCount": "Show unread count badges in sidebar",
      };
      return labels[key] ?? fallback ?? key;
    },
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("GeneralTab realtime mode", () => {
  beforeEach(() => {
    localStorage.clear();
    useUIStore.setState({
      pollInterval: 15,
      realtimeMode: "realtime",
      showFolderUnreadCount: false,
    });
  });

  it("defaults to realtime mode", () => {
    expect(useUIStore.getState().realtimeMode).toBe("realtime");
  });

  it("shows realtime strategy choices and persists selection", () => {
    render(<GeneralTab />);

    expect(screen.getByRole("button", { name: "Realtime (recommended)" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Balanced" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Battery saver" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Manual only" })).toBeTruthy();
    expect(screen.getByText("IMAP uses IDLE push when supported. Other providers check about every 3 seconds while you are active.")).toBeTruthy();
    expect(screen.getByText("Checks about every 15 seconds while you are active.")).toBeTruthy();
    expect(screen.getByText("Checks about every 60 seconds while you are active and slows down in the background.")).toBeTruthy();
    expect(screen.getByText("Stops background checks. Use Sync now to run a single pass.")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Battery saver" }));

    expect(useUIStore.getState().realtimeMode).toBe("battery");
    expect(localStorage.getItem("pebble-realtime-mode")).toBe("battery");
  });
});
