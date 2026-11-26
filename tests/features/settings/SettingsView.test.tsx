import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import SettingsView from "../../../src/features/settings/SettingsView";
import { useUIStore } from "../../../src/stores/ui.store";

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "settings.accounts": "Accounts",
        "settings.general": "General",
        "settings.proxy": "Proxy",
        "settings.appearance": "Appearance",
        "settings.privacy": "Privacy",
        "settings.rules": "Rules",
        "settings.remoteWrites": "Remote Writes",
        "settings.translation": "Translation",
        "settings.shortcuts": "Shortcuts",
        "settings.cloudSync": "Settings Backup",
        "settings.about": "About",
        "settings.tabs": "Settings tabs",
      };
      return labels[key] ?? fallback ?? key;
    },
  }),
}));

vi.mock("../../../src/features/settings/AccountsTab", () => ({
  default: () => <div>Accounts panel</div>,
}));

vi.mock("../../../src/features/settings/GeneralTab", () => ({
  default: () => <div>General panel</div>,
}));

vi.mock("../../../src/features/settings/ProxyTab", () => ({
  default: () => <div>Proxy panel</div>,
}));

vi.mock("../../../src/features/settings/AppearanceTab", () => ({
  default: () => <div>Appearance panel</div>,
}));

vi.mock("../../../src/features/settings/CloudSyncTab", () => ({
  default: () => <div>Cloud sync panel</div>,
}));

vi.mock("../../../src/features/settings/RulesTab", () => ({
  default: () => <div>Rules panel</div>,
}));

vi.mock("../../../src/features/settings/ShortcutsTab", () => ({
  default: () => <div>Shortcuts panel</div>,
}));

vi.mock("../../../src/features/settings/TranslateTab", () => ({
  default: () => <div>Translation panel</div>,
}));

vi.mock("../../../src/features/settings/PrivacyTab", () => ({
  default: () => <div>Privacy panel</div>,
}));

vi.mock("../../../src/features/settings/AboutTab", () => ({
  default: () => <div>About panel</div>,
}));

vi.mock("../../../src/features/settings/PendingOpsTab", () => ({
  default: () => <div>Remote queue panel</div>,
}));

describe("SettingsView", () => {
  beforeEach(() => {
    useUIStore.setState({ settingsTab: "accounts" });
  });

  it("exposes the pending remote writes queue as a settings tab", () => {
    render(<SettingsView />);

    const tab = screen.getByRole("tab", { name: "Remote Writes" });
    fireEvent.click(tab);

    expect(tab.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("Remote queue panel")).toBeTruthy();
  });

  it("exposes global proxy settings as a dedicated settings tab", () => {
    render(<SettingsView />);

    const tab = screen.getByRole("tab", { name: "Proxy" });
    fireEvent.click(tab);

    expect(tab.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("Proxy panel")).toBeTruthy();
  });

  it("styles settings tabs like the main sidebar items (neutral selected fill)", () => {
    render(<SettingsView />);

    const tab = screen.getByRole("tab", { name: "Accounts" });

    expect(tab.className).toContain("sidebar-btn");
    expect(tab.style.borderRight).toBe("");
  });

  it("keeps the settings panel vertically scrollable without horizontal overflow", () => {
    render(<SettingsView />);

    const panel = screen.getByRole("tabpanel");

    expect(panel.className).toContain("settings-panel-scroll");
    expect(panel.style.overflowY).toBe("auto");
    expect(panel.style.overflowX).toBe("hidden");
    expect(panel.style.boxSizing).toBe("border-box");
  });
});
