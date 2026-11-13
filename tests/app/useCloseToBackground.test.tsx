import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setKeepRunningInBackground, startSync, stopSync } from "../../src/lib/api";

const mocks = vi.hoisted(() => ({
  accounts: [{ id: "account-1" }, { id: "account-2" }],
  focusHandler: undefined as undefined | ((event: { payload: boolean }) => void),
  hide: vi.fn().mockResolvedValue(undefined),
  onCloseRequested: vi.fn(),
  onFocusChanged: vi.fn((handler: (event: { payload: boolean }) => void) => {
    mocks.focusHandler = handler;
    return Promise.resolve(vi.fn());
  }),
  uiState: {
    keepRunningInBackground: false,
    pollInterval: 5,
    realtimeMode: "realtime" as "realtime" | "balanced" | "battery" | "manual",
  },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    hide: mocks.hide,
    onCloseRequested: mocks.onCloseRequested,
    onFocusChanged: mocks.onFocusChanged,
  }),
}));

vi.mock("../../src/lib/api", () => ({
  setKeepRunningInBackground: vi.fn().mockResolvedValue(undefined),
  startSync: vi.fn().mockResolvedValue(undefined),
  stopSync: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({ data: mocks.accounts }),
}));

vi.mock("../../src/stores/ui.store", () => {
  const useUIStore = (selector: (state: typeof mocks.uiState) => unknown) => selector(mocks.uiState);
  useUIStore.getState = () => mocks.uiState;
  return { useUIStore };
});

import { useCloseToBackground } from "../../src/app/useCloseToBackground";

function Harness() {
  useCloseToBackground();
  return null;
}

describe("useCloseToBackground", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.focusHandler = undefined;
    mocks.accounts = [{ id: "account-1" }, { id: "account-2" }];
    mocks.uiState.keepRunningInBackground = false;
    mocks.uiState.pollInterval = 5;
    mocks.uiState.realtimeMode = "realtime";
  });

  it("syncs the keep-running preference to the backend guard", async () => {
    mocks.uiState.keepRunningInBackground = true;
    render(<Harness />);

    await waitFor(() =>
      expect(setKeepRunningInBackground).toHaveBeenCalledWith(true),
    );
  });

  it.each([false, true])("leaves window lifetime with the native guard when background mode is %s", async (enabled) => {
    mocks.uiState.keepRunningInBackground = enabled;
    const view = render(<Harness />);

    await waitFor(() => expect(setKeepRunningInBackground).toHaveBeenCalledWith(enabled));
    view.unmount();
    expect(mocks.onCloseRequested).not.toHaveBeenCalled();
    expect(mocks.hide).not.toHaveBeenCalled();
    expect(stopSync).not.toHaveBeenCalled();
  });

  it("updates the native close guard when the preference changes", async () => {
    const view = render(<Harness />);
    await waitFor(() => expect(setKeepRunningInBackground).toHaveBeenLastCalledWith(false));
    mocks.uiState.keepRunningInBackground = true;
    view.rerender(<Harness />);
    await waitFor(() => expect(setKeepRunningInBackground).toHaveBeenLastCalledWith(true));
    expect(mocks.onCloseRequested).not.toHaveBeenCalled();
  });

  it("resumes sync when the hidden window regains focus", async () => {
    render(<Harness />);

    await waitFor(() => expect(mocks.onFocusChanged).toHaveBeenCalledOnce());
    mocks.focusHandler?.({ payload: true });

    expect(startSync).toHaveBeenCalledWith("account-1", 5);
    expect(startSync).toHaveBeenCalledWith("account-2", 5);
  });

  it("does not resume sync in manual realtime mode", async () => {
    mocks.uiState.realtimeMode = "manual";
    render(<Harness />);

    await waitFor(() => expect(mocks.onFocusChanged).toHaveBeenCalledOnce());
    mocks.focusHandler?.({ payload: true });

    expect(startSync).not.toHaveBeenCalled();
  });
});
