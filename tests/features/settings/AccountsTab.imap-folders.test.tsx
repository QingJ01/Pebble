import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AccountsTab from "../../../src/features/settings/AccountsTab";
import {
  getImapSyncFolders,
  getAccountSyncRange,
  updateAccountSyncRange,
  triggerSync,
  updateAccount,
  updateImapSyncFolders,
} from "../../../src/lib/api";

const mocks = vi.hoisted(() => ({
  invalidateQueries: vi.fn(() => Promise.resolve()),
}));

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({
    invalidateQueries: mocks.invalidateQueries,
  }),
}));

vi.mock("../../../src/hooks/queries", () => ({
  accountsQueryKey: ["accounts"],
  useAccountsQuery: () => ({
    data: [
      {
        id: "account-1",
        email: "user@example.com",
        display_name: "User",
        provider: "imap",
        color: "#22c55e",
        created_at: 1,
        updated_at: 1,
      },
    ],
  }),
}));

vi.mock("../../../src/lib/api", () => ({
  getAccountSyncRange: vi.fn(() => Promise.resolve(null)),
  updateAccountSyncRange: vi.fn(() => Promise.resolve()),
  deleteAccount: vi.fn(),
  getImapSyncFolders: vi.fn(),
  getOAuthAccountProxySetting: vi.fn(),
  testAccountConnection: vi.fn(),
  triggerSync: vi.fn(),
  updateAccount: vi.fn(),
  updateImapSyncFolders: vi.fn(),
  updateOAuthAccountProxySetting: vi.fn(),
}));

vi.mock("../../../src/lib/signatures", () => ({
  getSignature: vi.fn(() => Promise.resolve("")),
  setSignature: vi.fn(() => Promise.resolve()),
}));

vi.mock("../../../src/components/AccountSetup", () => ({
  default: () => null,
}));

vi.mock("../../../src/stores/mail.store", () => ({
  useMailStore: {
    getState: () => ({
      activeAccountId: null,
      setActiveAccountId: vi.fn(),
    }),
  },
}));

vi.mock("../../../src/stores/ui.store", () => ({
  realtimePreferenceToPollInterval: () => 3,
  useUIStore: (selector: (state: { realtimeStatusByAccount: Record<string, unknown> }) => unknown) =>
    selector({ realtimeStatusByAccount: {} }),
}));

vi.mock("../../../src/stores/toast.store", () => ({
  useToastStore: {
    getState: () => ({
      addToast: vi.fn(),
    }),
  },
}));

const folderSettings = {
  folders: [
    {
      id: "inbox-id",
      account_id: "account-1",
      remote_id: "INBOX",
      name: "Inbox",
      folder_type: "folder" as const,
      role: "inbox" as const,
      parent_id: null,
      color: null,
      is_system: true,
      sort_order: 0,
    },
    {
      id: "sent-id",
      account_id: "account-1",
      remote_id: "Sent",
      name: "Sent",
      folder_type: "folder" as const,
      role: "sent" as const,
      parent_id: null,
      color: null,
      is_system: true,
      sort_order: 1,
    },
    {
      id: "projects-id",
      account_id: "account-1",
      remote_id: "Projects",
      name: "Projects",
      folder_type: "folder" as const,
      role: null,
      parent_id: null,
      color: null,
      is_system: true,
      sort_order: 10,
    },
  ],
  selected_remote_ids: ["INBOX", "Sent"],
};

describe("AccountsTab IMAP folder selection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getAccountSyncRange).mockResolvedValue(null);
    vi.mocked(getImapSyncFolders).mockResolvedValue(folderSettings);
    vi.mocked(updateImapSyncFolders).mockImplementation(
      async (_accountId, selectedRemoteIds) => ({
        ...folderSettings,
        selected_remote_ids: selectedRemoteIds,
      }),
    );
    vi.mocked(updateAccount).mockResolvedValue(undefined);
    vi.mocked(triggerSync).mockResolvedValue(undefined);
  });

  it("loads a custom range and saves a wider range to backfill earlier mail", async () => {
    vi.mocked(getAccountSyncRange).mockResolvedValue(14);
    render(<AccountsTab />);
    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));
    const days = await screen.findByLabelText("Number of days") as HTMLInputElement;
    expect(days.value).toBe("14");
    fireEvent.change(screen.getByLabelText("Sync range"), { target: { value: "all" } });
    await waitFor(() => expect(screen.getByRole("button", { name: "common.save" })).not.toBeDisabled());
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    await waitFor(() => expect(updateAccountSyncRange).toHaveBeenCalledWith("account-1", null, 3));
  });

  it("does not overwrite the stored range when loading fails", async () => {
    vi.mocked(getAccountSyncRange).mockRejectedValue(new Error("Storage unavailable"));
    render(<AccountsTab />);
    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Storage unavailable");
    expect(screen.getByRole("button", { name: "common.save" })).toBeDisabled();
    expect(updateAccountSyncRange).not.toHaveBeenCalled();
  });

  it("shows a save failure without closing the editor", async () => {
    vi.mocked(updateAccountSyncRange).mockRejectedValueOnce(new Error("Sync restart failed"));
    render(<AccountsTab />);
    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "common.save" })).not.toBeDisabled());
    fireEvent.change(screen.getByLabelText("Sync range"), { target: { value: "30" } });
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Sync restart failed");
    expect(screen.getByLabelText("Sync range")).toHaveValue("30");
  });

  it("lists folders, locks Inbox, and saves additional selections", async () => {
    render(<AccountsTab />);

    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));
    fireEvent.click(await screen.findByRole("button", { name: "Choose folders" }));

    await waitFor(() => {
      expect(getImapSyncFolders).toHaveBeenCalledWith("account-1");
    });

    const inbox = await screen.findByRole("checkbox", { name: "Inbox" });
    const sent = screen.getByRole("checkbox", { name: "Sent" });
    const projects = screen.getByRole("checkbox", { name: "Projects" });
    expect((inbox as HTMLInputElement).checked).toBe(true);
    expect((inbox as HTMLInputElement).disabled).toBe(true);

    fireEvent.click(sent);
    fireEvent.click(projects);
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));

    await waitFor(() => {
      expect(updateImapSyncFolders).toHaveBeenCalledWith("account-1", [
        "INBOX",
        "Projects",
      ]);
    });
    expect(triggerSync).toHaveBeenCalledWith("account-1", "manual");
  });
});
