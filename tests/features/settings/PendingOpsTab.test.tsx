import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import PendingOpsTab from "../../../src/features/settings/PendingOpsTab";

const mocks = vi.hoisted(() => ({ confirm: vi.fn(), retry: vi.fn(), refetch: vi.fn(), ops: [] as Record<string, unknown>[] }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock("@tanstack/react-query", () => ({ useQueryClient: () => ({ invalidateQueries: vi.fn() }) }));
vi.mock("../../../src/stores/confirm.store", () => ({ useConfirmStore: { getState: () => ({ confirm: mocks.confirm }) } }));
vi.mock("../../../src/stores/toast.store", () => ({ useToastStore: { getState: () => ({ addToast: vi.fn() }) } }));
vi.mock("../../../src/stores/mail.store", () => ({ useMailStore: (selector: (s: object) => unknown) => selector({ activeAccountId: null }) }));
vi.mock("../../../src/lib/api", () => ({ retryFailedSend: mocks.retry, dismissFailedPendingMailOps: vi.fn() }));
vi.mock("../../../src/hooks/queries", () => ({
  useAccountsQuery: () => ({ data: [] }), usePendingMailOpsSummary: () => ({ data: undefined, refetch: mocks.refetch }),
  usePendingMailOpsQuery: () => ({ data: mocks.ops, refetch: mocks.refetch }),
  pendingMailOpsQueryKey: () => ["ops"], pendingMailOpsSummaryQueryKey: () => ["summary"],
}));
describe("PendingOpsTab send recovery", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.ops = [{ id: "op", account_id: "account", message_id: "message", op_type: "send", status: "failed", attempts: 8, updated_at: 1, last_error: "SMTP unavailable" }];
  });
  it("warns about duplicate delivery and does not send when canceled", async () => {
    mocks.confirm.mockResolvedValue(false);
    render(<PendingOpsTab />);
    fireEvent.click(screen.getByRole("button", { name: "Retry send" }));
    await waitFor(() => expect(mocks.confirm).toHaveBeenCalledWith(expect.objectContaining({ message: expect.stringContaining("duplicate") })));
    expect(mocks.retry).not.toHaveBeenCalled();
  });
  it("retries exactly the selected failed send after confirmation", async () => {
    mocks.confirm.mockResolvedValue(true);
    mocks.retry.mockResolvedValue(undefined);
    render(<PendingOpsTab />);
    fireEvent.click(screen.getByRole("button", { name: "Retry send" }));
    await waitFor(() => expect(mocks.retry).toHaveBeenCalledWith("op"));
    await waitFor(() => expect(mocks.refetch).toHaveBeenCalledTimes(2));
  });
  it.each([["send", "pending"], ["send", "in_progress"], ["archive", "failed"]])("does not retry %s in %s state", (op_type, status) => {
    mocks.ops[0] = { ...mocks.ops[0], op_type, status };
    render(<PendingOpsTab />);
    expect(screen.queryByRole("button", { name: "Retry send" })).toBeNull();
  });
});
