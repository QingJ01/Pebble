import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import TitleBar from "../../src/components/TitleBar";

const windowMock = {
  close: vi.fn(),
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
};

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowMock,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

vi.mock("../../src/stores/compose.store", () => ({
  isComposeDirty: () => false,
}));

vi.mock("../../src/stores/confirm.store", () => ({
  useConfirmStore: {
    getState: () => ({
      confirm: vi.fn(),
    }),
  },
}));

vi.mock("../../src/lib/i18n", () => ({
  default: {
    t: (_key: string, fallback?: string) => fallback ?? _key,
  },
}));

describe("TitleBar", () => {
  it("renders the app logo as a transparent custom titlebar image", () => {
    const { container } = render(<TitleBar />);

    expect(screen.getByText("Pebble")).toBeTruthy();

    const logo = container.querySelector("img[aria-hidden='true']");
    expect(logo).not.toBeNull();
    expect(logo?.className).toContain("bg-transparent");
    expect(logo?.getAttribute("draggable")).toBe("false");
  });

  it("keeps the brand block flush at the top-left on macOS", async () => {
    const originalUserAgent = navigator.userAgent;
    Object.defineProperty(navigator, "userAgent", {
      value: `${originalUserAgent} Macintosh`,
      configurable: true,
    });
    vi.resetModules();

    try {
      const { default: TitleBarMac } = await import("../../src/components/TitleBar");
      const { container, unmount } = render(<TitleBarMac />);

      const logo = container.querySelector("img[aria-hidden='true']");
      expect(logo).not.toBeNull();
      expect((logo?.parentElement as HTMLElement | null)?.style.paddingLeft).toBe("");
      expect(screen.queryByLabelText("titleBar.minimize")).toBeNull();
      expect(screen.queryByLabelText("titleBar.maximize")).toBeNull();
      expect(screen.queryByLabelText("titleBar.close")).toBeNull();

      unmount();
    } finally {
      Object.defineProperty(navigator, "userAgent", {
        value: originalUserAgent,
        configurable: true,
      });
      vi.resetModules();
    }
  });
});
