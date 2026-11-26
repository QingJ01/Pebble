import { afterEach, describe, expect, it, vi } from "vitest";
import { useUIStore } from "../../src/stores/ui.store";

function mockNarrow(narrow: boolean) {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: narrow,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

describe("toggleSidebar", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    useUIStore.setState({ sidebarCollapsed: false, sidebarNarrowExpanded: false });
  });

  it("flips the saved preference in a wide window", () => {
    mockNarrow(false);
    useUIStore.setState({ sidebarCollapsed: false, sidebarNarrowExpanded: false });
    useUIStore.getState().toggleSidebar();
    expect(useUIStore.getState().sidebarCollapsed).toBe(true);
    expect(useUIStore.getState().sidebarNarrowExpanded).toBe(false);
  });

  it("expands the icon rail in a narrow window without touching the preference", () => {
    mockNarrow(true);
    useUIStore.setState({ sidebarCollapsed: false, sidebarNarrowExpanded: false });
    useUIStore.getState().toggleSidebar();
    expect(useUIStore.getState().sidebarNarrowExpanded).toBe(true);
    expect(useUIStore.getState().sidebarCollapsed).toBe(false);
  });
});
