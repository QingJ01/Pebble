import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ProxyTab from "../../../src/features/settings/ProxyTab";
import {
  getAccountProxySetting,
  getGlobalProxy,
  getOAuthAccountProxySetting,
  updateAccountProxySetting,
  updateGlobalProxy,
  updateOAuthAccountProxySetting,
} from "../../../src/lib/api";

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "common.save": "Save",
        "common.saving": "Saving...",
        "settings.globalProxy": "Global Proxy",
        "settings.globalProxyDesc": "Used by inherited network requests.",
        "settings.globalProxyHost": "SOCKS5 Proxy",
        "settings.globalProxyPort": "Port",
        "settings.globalProxySaved": "Global proxy saved",
        "settings.accountProxies": "Account Proxies",
        "settings.accountProxiesDesc": "Override the global proxy for individual accounts.",
        "settings.accountProxySaved": "Account proxy saved",
        "settings.accountProxyModeInherit": "Inherit global proxy",
        "settings.accountProxyModeDisabled": "Do not use proxy",
        "settings.accountProxyModeCustom": "Use custom proxy",
        "settings.inheritsGlobalProxy": "Inherit global proxy unless a custom proxy or disabled mode is selected.",
        "settings.saveAccountProxy": "Save account proxy",
      };
      return labels[key] ?? fallback ?? key;
    },
  }),
}));

const mocks = vi.hoisted(() => ({
  accounts: [
    {
      id: "imap-1",
      email: "imap@example.com",
      display_name: "IMAP User",
      provider: "imap" as const,
      created_at: 1,
      updated_at: 1,
    },
    {
      id: "gmail-1",
      email: "gmail@example.com",
      display_name: "Gmail User",
      provider: "gmail" as const,
      created_at: 1,
      updated_at: 1,
    },
  ],
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({
    invalidateQueries: vi.fn(),
  }),
}));

vi.mock("../../../src/hooks/queries", () => ({
  accountsQueryKey: ["accounts"],
  useAccountsQuery: () => ({
    data: mocks.accounts,
  }),
}));

vi.mock("../../../src/lib/api", () => ({
  getAccountProxySetting: vi.fn(),
  getGlobalProxy: vi.fn(),
  getOAuthAccountProxySetting: vi.fn(),
  updateAccountProxySetting: vi.fn(),
  updateGlobalProxy: vi.fn(),
  updateOAuthAccountProxySetting: vi.fn(),
}));

describe("ProxyTab global proxy", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.accounts = [
      {
        id: "imap-1",
        email: "imap@example.com",
        display_name: "IMAP User",
        provider: "imap" as const,
        created_at: 1,
        updated_at: 1,
      },
      {
        id: "gmail-1",
        email: "gmail@example.com",
        display_name: "Gmail User",
        provider: "gmail" as const,
        created_at: 1,
        updated_at: 1,
      },
    ];
    vi.mocked(getGlobalProxy).mockResolvedValue({ host: "127.0.0.1", port: 7890 });
    vi.mocked(getAccountProxySetting).mockResolvedValue({
      mode: "custom",
      proxy: { host: "192.168.0.2", port: 1080 },
    });
    vi.mocked(getOAuthAccountProxySetting).mockResolvedValue({
      mode: "inherit",
      proxy: null,
    });
    vi.mocked(updateAccountProxySetting).mockResolvedValue(undefined);
    vi.mocked(updateGlobalProxy).mockResolvedValue(undefined);
    vi.mocked(updateOAuthAccountProxySetting).mockResolvedValue(undefined);
  });

  it("loads and saves the encrypted global proxy setting", async () => {
    render(<ProxyTab />);

    await waitFor(() => {
      expect(getGlobalProxy).toHaveBeenCalled();
    });
    const globalGroup = screen.getByRole("group", { name: "Global Proxy" });
    expect((within(globalGroup).getByLabelText("SOCKS5 Proxy") as HTMLInputElement).value).toBe("127.0.0.1");
    expect((within(globalGroup).getByLabelText("Port") as HTMLInputElement).value).toBe("7890");

    fireEvent.change(within(globalGroup).getByLabelText("SOCKS5 Proxy"), {
      target: { value: "10.0.0.2" },
    });
    fireEvent.change(within(globalGroup).getByLabelText("Port"), {
      target: { value: "1080" },
    });
    fireEvent.click(within(globalGroup).getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(updateGlobalProxy).toHaveBeenCalledWith("10.0.0.2", 1080);
    });
  });

  it("clears the global proxy when both fields are blank", async () => {
    vi.mocked(getGlobalProxy).mockResolvedValue(null);

    render(<ProxyTab />);

    await waitFor(() => {
      expect(getGlobalProxy).toHaveBeenCalled();
    });
    const globalGroup = screen.getByRole("group", { name: "Global Proxy" });
    fireEvent.click(within(globalGroup).getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(updateGlobalProxy).toHaveBeenCalledWith(undefined, undefined);
    });
  });

  it("shows per-account proxy settings without a disclosure toggle", async () => {
    render(<ProxyTab />);

    expect(screen.queryByRole("button", { name: "Account Proxies" })).toBeNull();
    await waitFor(() => {
      expect(getAccountProxySetting).toHaveBeenCalledWith("imap-1");
      expect(getOAuthAccountProxySetting).toHaveBeenCalledWith("gmail-1");
    });
    expect(screen.getByRole("group", { name: "imap@example.com Account Proxy" })).not.toBeNull();
  });

  it("loads and saves per-account proxy modes from the proxy tab", async () => {
    render(<ProxyTab />);

    await waitFor(() => {
      expect(getAccountProxySetting).toHaveBeenCalledWith("imap-1");
    });
    const imapGroup = screen.getByRole("group", { name: "imap@example.com Account Proxy" });
    const customModeButton = within(imapGroup).getByRole("button", { name: "Use custom proxy" });
    expect(customModeButton.getAttribute("style")).toContain("border: 2px solid var(--color-accent)");
    expect(customModeButton.getAttribute("style")).toContain("background-color: var(--color-bg-hover)");
    expect(screen.queryAllByText("Inherit global proxy unless a custom proxy or disabled mode is selected.")).toHaveLength(0);
    const saveButton = within(imapGroup).getByRole("button", { name: "Save account proxy" });
    expect(saveButton.getAttribute("style")).toContain("background-color: var(--color-accent)");
    expect(saveButton.getAttribute("style")).toContain("color: rgb(255, 255, 255)");
    const accountProxyHostInput = within(imapGroup).getByLabelText("SOCKS5 Proxy") as HTMLInputElement;
    const accountProxyPortInput = within(imapGroup).getByLabelText("Port") as HTMLInputElement;
    expect(accountProxyHostInput.value).toBe("192.168.0.2");
    expect(accountProxyPortInput.value).toBe("1080");
    expect(accountProxyHostInput.getAttribute("style")).toContain("width: 100%");
    expect(accountProxyHostInput.getAttribute("style")).toContain("box-sizing: border-box");
    expect(accountProxyPortInput.getAttribute("style")).toContain("width: 100%");
    expect(accountProxyPortInput.getAttribute("style")).toContain("box-sizing: border-box");

    const gmailGroup = screen.getByRole("group", { name: "gmail@example.com Account Proxy" });
    expect(within(gmailGroup).queryByLabelText("SOCKS5 Proxy")).toBeNull();
    expect(within(gmailGroup).queryByLabelText("Port")).toBeNull();

    fireEvent.click(within(imapGroup).getByRole("button", { name: "Do not use proxy" }));
    expect(within(imapGroup).queryByLabelText("SOCKS5 Proxy")).toBeNull();
    expect(within(imapGroup).queryByLabelText("Port")).toBeNull();
    fireEvent.click(within(imapGroup).getByRole("button", { name: "Save account proxy" }));

    await waitFor(() => {
      expect(updateAccountProxySetting).toHaveBeenCalledWith("imap-1", "disabled", undefined, undefined);
    });

    fireEvent.click(within(imapGroup).getByRole("button", { name: "Use custom proxy" }));
    fireEvent.change(within(imapGroup).getByLabelText("SOCKS5 Proxy"), {
      target: { value: "10.0.0.2" },
    });
    fireEvent.change(within(imapGroup).getByLabelText("Port"), {
      target: { value: "1081" },
    });
    fireEvent.click(within(imapGroup).getByRole("button", { name: "Save account proxy" }));

    await waitFor(() => {
      expect(updateAccountProxySetting).toHaveBeenCalledWith("imap-1", "custom", "10.0.0.2", 1081);
    });

    fireEvent.click(within(gmailGroup).getByRole("button", { name: "Save account proxy" }));

    await waitFor(() => {
      expect(updateOAuthAccountProxySetting).toHaveBeenCalledWith("gmail-1", "inherit", undefined, undefined);
    });
  });

  it("refreshes account labels when account metadata changes without reloading proxy settings", async () => {
    const { rerender } = render(<ProxyTab />);

    await waitFor(() => {
      expect(screen.getByText("IMAP User")).toBeTruthy();
    });

    mocks.accounts = [
      {
        id: "imap-1",
        email: "renamed@example.com",
        display_name: "Renamed IMAP",
        provider: "imap" as const,
        created_at: 1,
        updated_at: 2,
      },
      {
        id: "gmail-1",
        email: "gmail@example.com",
        display_name: "Gmail User",
        provider: "gmail" as const,
        created_at: 1,
        updated_at: 1,
      },
    ];

    rerender(<ProxyTab />);

    expect(screen.getByText("Renamed IMAP")).toBeTruthy();
    expect(screen.getByText("renamed@example.com")).toBeTruthy();
    expect(getAccountProxySetting).toHaveBeenCalledTimes(1);
  });
});
