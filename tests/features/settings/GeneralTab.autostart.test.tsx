import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import GeneralTab from "../../../src/features/settings/GeneralTab";
import { getAutostartEnabled, setAutostartEnabled } from "../../../src/lib/api";
import { useUIStore } from "../../../src/stores/ui.store";

vi.mock("../../../src/features/compose/ScheduledSendsView", () => ({ default: () => null }));

vi.mock("../../../src/lib/api", () => ({
  showTestNotification: vi.fn().mockResolvedValue(undefined),
  openDefaultMailSettings: vi.fn().mockResolvedValue(undefined),
  getAutostartEnabled: vi.fn().mockResolvedValue(false),
  setAutostartEnabled: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "settings.startupBehavior": "Startup Behavior",
        "settings.startHiddenToTray": "Start hidden to tray",
        "settings.launchAtStartup": "Launch Pebble at system startup",
        "settings.autostartFailed": "Failed to update launch at startup",
      };
      return labels[key] ?? fallback ?? key;
    },
  }),
}));

describe("GeneralTab launch-at-startup", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    useUIStore.setState({
      pollInterval: 15,
      realtimeMode: "realtime",
      showFolderUnreadCount: false,
      notificationsEnabled: true,
      keepRunningInBackground: true,
      startHiddenToTray: false,
    });
  });

  it("reflects the current OS autostart state on mount", async () => {
    vi.mocked(getAutostartEnabled).mockResolvedValue(true);
    render(<GeneralTab />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Launch Pebble at system startup",
    });
    await waitFor(() => {
      expect((checkbox as HTMLInputElement).checked).toBe(true);
    });
    expect(getAutostartEnabled).toHaveBeenCalledTimes(1);
  });

  it("enables autostart through the backend when toggled on", async () => {
    vi.mocked(getAutostartEnabled).mockResolvedValue(false);
    render(<GeneralTab />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Launch Pebble at system startup",
    });
    await waitFor(() => {
      expect((checkbox as HTMLInputElement).checked).toBe(false);
    });

    fireEvent.click(checkbox);

    await waitFor(() => {
      expect(setAutostartEnabled).toHaveBeenCalledWith(true);
    });
    await waitFor(() => {
      expect((checkbox as HTMLInputElement).checked).toBe(true);
    });
  });

  it("reverts the toggle when the backend call fails", async () => {
    vi.mocked(getAutostartEnabled).mockResolvedValue(false);
    vi.mocked(setAutostartEnabled).mockRejectedValueOnce(new Error("denied"));
    render(<GeneralTab />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Launch Pebble at system startup",
    });
    await waitFor(() => {
      expect((checkbox as HTMLInputElement).checked).toBe(false);
    });

    fireEvent.click(checkbox);

    await waitFor(() => {
      expect(setAutostartEnabled).toHaveBeenCalledWith(true);
    });
    // After the failure the checkbox returns to its previous (off) state.
    await waitFor(() => {
      expect((checkbox as HTMLInputElement).checked).toBe(false);
    });
  });
});
