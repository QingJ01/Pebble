import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
  uiState: {
    syncStatus: "idle" as "idle" | "syncing" | "error",
    setSyncStatus: vi.fn(),
    networkStatus: "online" as "online" | "offline",
    lastMailError: null as string | null,
    setLastMailError: vi.fn(),
    realtimeStatusByAccount: {} as Record<
      string,
      {
        account_id: string;
        mode: "realtime" | "polling" | "backoff" | "offline" | "auth_required" | "error";
        provider: string;
        last_success_at?: number | null;
        next_retry_at?: number | null;
        message?: string | null;
      }
    >,
    setRealtimeStatus: vi.fn(),
  },
  mailState: {
    activeAccountId: "account-1" as string | null,
  },
  pendingOpsSummary: {
    total_active_count: 0,
    failed_count: 0,
    in_progress_count: 0,
    last_error: null as string | null,
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "status.ready": "Ready",
        "status.syncing": "Syncing...",
        "status.syncError": "Sync error",
        "status.offline": "Offline",
        "status.syncNow": "Sync now",
        "status.stopSync": "Stop sync",
        "status.remoteWritesQueued": `${mocks.pendingOpsSummary.total_active_count} remote writes queued`,
        "status.remoteWritesPending": `${mocks.pendingOpsSummary.total_active_count} remote writes pending`,
        "status.remoteWritesRetrying": `${mocks.pendingOpsSummary.in_progress_count} remote writes retrying`,
        "status.realtimeConnected": "Realtime connected",
      };
      return labels[key] ?? fallback ?? key;
    },
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({
    invalidateQueries: vi.fn(),
  }),
}));

vi.mock("../../src/stores/ui.store", () => ({
  useUIStore: (selector: (state: typeof mocks.uiState) => unknown) => selector(mocks.uiState),
}));

vi.mock("../../src/stores/mail.store", () => ({
  useMailStore: (selector: (state: typeof mocks.mailState) => unknown) => selector(mocks.mailState),
}));

vi.mock("../../src/hooks/mutations/useSyncMutation", () => ({
  useSyncMutation: () => ({
    mutateAsync: vi.fn(),
  }),
}));

vi.mock("../../src/hooks/queries", () => ({
  pendingMailOpsSummaryQueryKey: (accountId: string | null) => ["pending-mail-ops-summary", accountId],
  usePendingMailOpsSummary: () => ({
    data: mocks.pendingOpsSummary,
  }),
}));

vi.mock("../../src/lib/api", () => ({
  stopSync: vi.fn(),
}));

import StatusBar from "../../src/components/StatusBar";

describe("StatusBar accessibility", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.uiState.syncStatus = "idle";
    mocks.uiState.networkStatus = "online";
    mocks.uiState.lastMailError = null;
    mocks.uiState.realtimeStatusByAccount = {};
    mocks.mailState.activeAccountId = "account-1";
    mocks.pendingOpsSummary.total_active_count = 0;
    mocks.pendingOpsSummary.failed_count = 0;
    mocks.pendingOpsSummary.in_progress_count = 0;
    mocks.pendingOpsSummary.last_error = null;
  });

  it("announces normal status changes politely", () => {
    render(<StatusBar />);

    const status = screen.getByText("Ready").closest("[role='status']");

    expect(status?.getAttribute("aria-live")).toBe("polite");
    expect(status?.getAttribute("aria-atomic")).toBe("true");
  });

  it("announces mail errors assertively", () => {
    mocks.uiState.lastMailError = "IMAP connection failed";

    render(<StatusBar />);

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("IMAP connection failed");
    expect(alert.getAttribute("aria-live")).toBe("assertive");
    expect(alert.getAttribute("aria-atomic")).toBe("true");
  });

  it("announces realtime connection health politely", () => {
    mocks.uiState.realtimeStatusByAccount = {
      "account-1": {
        account_id: "account-1",
        mode: "realtime",
        provider: "imap",
        last_success_at: 1_700_000_000,
        next_retry_at: null,
        message: null,
      },
    };

    render(<StatusBar />);

    const status = screen.getByRole("status", { name: /realtime connected/i });
    expect(status.getAttribute("aria-live")).toBe("polite");
    expect(status.getAttribute("aria-atomic")).toBe("true");
  });
});
