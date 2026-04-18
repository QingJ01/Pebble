import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(),
}));

import { getCurrentWindow } from "@tauri-apps/api/window";
import { showMainWindow } from "../../src/lib/showMainWindow";

const mockGetCurrentWindow = vi.mocked(getCurrentWindow);

describe("showMainWindow", () => {
  afterEach(() => {
    delete (window as unknown as { __splashStart?: number }).__splashStart;
    vi.restoreAllMocks();
  });

  it("shows the current Tauri window when the frontend entrypoint loads", async () => {
    const show = vi.fn().mockResolvedValue(undefined);
    const info = vi.spyOn(console, "info").mockImplementation(() => {});
    mockGetCurrentWindow.mockReturnValue({ show } as never);
    (window as unknown as { __splashStart: number }).__splashStart = 1_000;
    vi.spyOn(Date, "now").mockReturnValue(1_180);

    await showMainWindow();

    expect(mockGetCurrentWindow).toHaveBeenCalledOnce();
    expect(show).toHaveBeenCalledOnce();
    expect(info).toHaveBeenCalledWith("[startup] main window shown: 180ms since splash start");
  });
});
