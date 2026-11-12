import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AccountsTab from "../../../src/features/settings/AccountsTab";
import {
  getOAuthAccountProxySetting,
  updateAccount,
  updateOAuthAccountProxySetting,
} from "../../../src/lib/api";

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
    invalidateQueries: vi.fn(),
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
        provider: "gmail",
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
  getOAuthAccountProxy: vi.fn(() => Promise.resolve(null)),
  getOAuthAccountProxySetting: vi.fn(),
  testAccountConnection: vi.fn(),
  updateAccount: vi.fn(),
  updateOAuthAccountProxy: vi.fn(() => Promise.resolve(undefined)),
  updateOAuthAccountProxySetting: vi.fn(),
}));

vi.mock("../../../src/lib/signatures", () => ({
  getSignature: vi.fn(() => Promise.resolve("")),
  setSignature: vi.fn(() => Promise.resolve()),
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

describe("AccountsTab OAuth proxy", () => {
  it("rejects invalid proxy ports before changing OAuth account metadata", async () => {
    render(<AccountsTab />);
    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));
    const port = await screen.findByLabelText("Port");
    await waitFor(() => expect((port as HTMLInputElement).value).toBe("7890"));
    fireEvent.change(port, { target: { value: "70000" } });
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    await screen.findByText("Port must be an integer between 1 and 65535.");
    expect(updateAccount).not.toHaveBeenCalled();
    expect(updateOAuthAccountProxySetting).not.toHaveBeenCalled();
  });
  it("edits a local label independently and keeps the OAuth address read-only", async () => {
    render(<AccountsTab />);
    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));
    const label = await screen.findByLabelText("Account label") as HTMLInputElement;
    fireEvent.change(label, { target: { value: "Work only" } });
    expect((screen.getByLabelText("accountSetup.emailAddress") as HTMLInputElement).readOnly).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    await waitFor(() => expect(updateAccount).toHaveBeenCalled());
    const args = vi.mocked(updateAccount).mock.calls[0];
    expect(args[1]).toBe("user@example.com");
    expect(args[2]).toBe("User");
    expect(args[14]).toBe("Work only");
  });
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getOAuthAccountProxySetting).mockResolvedValue({
      mode: "custom",
      proxy: { host: "127.0.0.1", port: 7890 },
    });
    vi.mocked(updateAccount).mockResolvedValue(undefined);
    vi.mocked(updateOAuthAccountProxySetting).mockResolvedValue(undefined);
  });

  it("loads and saves OAuth proxy settings without IMAP credentials", async () => {
    render(<AccountsTab />);

    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));

    await waitFor(() => {
      expect(getOAuthAccountProxySetting).toHaveBeenCalledWith("account-1");
    });
    await waitFor(() => {
      expect((screen.getByLabelText("SOCKS5 Proxy") as HTMLInputElement).value).toBe(
        "127.0.0.1",
      );
    });
    expect(screen.queryByLabelText("Password / App password")).toBeNull();

    fireEvent.change(screen.getByLabelText("SOCKS5 Proxy"), {
      target: { value: "10.0.0.2" },
    });
    fireEvent.change(screen.getByLabelText("Port"), {
      target: { value: "1080" },
    });
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));

    await waitFor(() => {
      expect(updateAccount).toHaveBeenCalledWith(
        "account-1",
        "user@example.com",
        "User",
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        "#22c55e",
        "",
      );
    });
    expect(updateOAuthAccountProxySetting).toHaveBeenCalledWith(
      "account-1",
      "custom",
      "10.0.0.2",
      1080,
    );
  });

  it("preserves disabled OAuth proxy mode when editing account metadata", async () => {
    vi.mocked(getOAuthAccountProxySetting).mockResolvedValueOnce({
      mode: "disabled",
      proxy: null,
    });

    render(<AccountsTab />);

    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));
    await screen.findByLabelText("Account color");
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));

    await waitFor(() => {
      expect(updateOAuthAccountProxySetting).toHaveBeenCalledWith(
        "account-1",
        "disabled",
        undefined,
        undefined,
      );
    });
  });

  it("saves a custom account color from the edit dialog", async () => {
    render(<AccountsTab />);

    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));

    const colorInput = await screen.findByLabelText("Account color");
    expect((colorInput as HTMLInputElement).value).toBe("#22c55e");

    fireEvent.change(colorInput, {
      target: { value: "#f97316" },
    });
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));

    await waitFor(() => {
      expect(updateAccount).toHaveBeenCalledWith(
        "account-1",
        "user@example.com",
        "User",
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        "#f97316",
        "",
      );
    });
  });

  it("offers built-in color presets in the edit dialog", async () => {
    render(<AccountsTab />);

    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));

    const preset = await screen.findByRole("button", { name: "Use color #0ea5e9" });
    fireEvent.click(preset);
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));

    await waitFor(() => {
      expect(updateAccount).toHaveBeenCalledWith(
        "account-1",
        "user@example.com",
        "User",
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        "#0ea5e9",
        "",
      );
    });
  });

  it("keeps the edit-account dialog open when clicking the backdrop", async () => {
    render(<AccountsTab />);

    fireEvent.click(screen.getByRole("button", { name: "Edit account" }));

    const dialog = await screen.findByRole("dialog", { name: "Edit Account" });
    fireEvent.mouseDown(dialog);
    fireEvent.click(dialog);

    expect(screen.queryByRole("dialog", { name: "Edit Account" })).not.toBeNull();
  });
});
