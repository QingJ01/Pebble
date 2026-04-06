import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const mockInvoke = vi.mocked(invoke);

import { foldersQueryKey } from "../../src/hooks/queries/useFoldersQuery";
import { listFolders } from "../../src/lib/api";

describe("useFoldersQuery", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should generate correct query key with accountId", () => {
    expect(foldersQueryKey("a1")).toEqual(["folders", "a1"]);
  });

  it("listFolders should call the correct Tauri command", async () => {
    const mockFolders = [
      {
        id: "f1",
        account_id: "a1",
        remote_id: "r1",
        name: "Inbox",
        folder_type: "folder" as const,
        role: "inbox" as const,
        parent_id: null,
        color: null,
        is_system: true,
        sort_order: 1,
      },
      {
        id: "f2",
        account_id: "a1",
        remote_id: "r2",
        name: "Sent",
        folder_type: "folder" as const,
        role: "sent" as const,
        parent_id: null,
        color: null,
        is_system: true,
        sort_order: 2,
      },
    ];
    mockInvoke.mockResolvedValueOnce(mockFolders);

    const result = await listFolders("a1");

    expect(result).toEqual(mockFolders);
    expect(mockInvoke).toHaveBeenCalledWith("list_folders", { accountId: "a1" });
  });
});
