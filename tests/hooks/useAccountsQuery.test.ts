import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const mockInvoke = vi.mocked(invoke);

// Import after mocking
import { accountsQueryKey } from "../../src/hooks/queries/useAccountsQuery";
import { listAccounts } from "../../src/lib/api";

describe("useAccountsQuery", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should have correct query key", () => {
    expect(accountsQueryKey).toEqual(["accounts"]);
  });

  it("listAccounts should call the correct Tauri command", async () => {
    const mockAccounts = [
      {
        id: "a1",
        email: "test@example.com",
        display_name: "Test User",
        provider: "imap" as const,
        created_at: 1000,
        updated_at: 1000,
      },
    ];
    mockInvoke.mockResolvedValueOnce(mockAccounts);

    const result = await listAccounts();

    expect(result).toEqual(mockAccounts);
    expect(mockInvoke).toHaveBeenCalledWith("list_accounts");
  });
});
