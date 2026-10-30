import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AppearanceTab from "../../../src/features/settings/AppearanceTab";
import { useUIStore } from "../../../src/stores/ui.store";

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "settings.sidebarStyle": "Sidebar style",
        "settings.sidebarStyleGrouped": "Grouped by account",
        "settings.sidebarStyleClassic": "Classic",
      };
      return labels[key] ?? fallback ?? key;
    },
  }),
}));

vi.mock("../../../src/lib/backgroundImage", () => ({
  backgroundImageUrl: vi.fn((path: string) => `asset://${path}`),
  deleteBackgroundImage: vi.fn().mockResolvedValue(undefined),
  importBackgroundImage: vi.fn(),
}));

describe("AppearanceTab sidebar style", () => {
  beforeEach(() => {
    localStorage.clear();
    useUIStore.setState({
      sidebarStyle: "grouped",
      backgroundImage: null,
    });
  });

  it("switches the sidebar to the classic style and persists it", () => {
    render(<AppearanceTab />);

    const group = screen.getByRole("group", { name: "Sidebar style" });
    fireEvent.click(within(group).getByRole("button", { name: /^Classic/ }));

    expect(useUIStore.getState().sidebarStyle).toBe("classic");
    expect(localStorage.getItem("pebble-sidebar-style")).toBe("classic");
  });

  it("switches back to the grouped style and persists it", () => {
    useUIStore.setState({ sidebarStyle: "classic" });

    render(<AppearanceTab />);

    const group = screen.getByRole("group", { name: "Sidebar style" });
    fireEvent.click(within(group).getByRole("button", { name: /^Grouped by account/ }));

    expect(useUIStore.getState().sidebarStyle).toBe("grouped");
    expect(localStorage.getItem("pebble-sidebar-style")).toBe("grouped");
  });
});
