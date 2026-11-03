import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import AccountSetup from "../../src/components/AccountSetup";
import { accountsQueryKey } from "../../src/hooks/queries";
import {
  addAccount,
  completeOAuthFlow,
  startSync,
  testPop3Connection,
} from "../../src/lib/api";

vi.mock("../../src/lib/i18n", () => ({
  default: {
    t: (_key: string, fallback?: string) => fallback ?? _key,
  },
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

vi.mock("../../src/lib/api", () => ({
  addAccount: vi.fn(),
  completeOAuthFlow: vi.fn(),
  startSync: vi.fn(),
  testImapConnection: vi.fn(),
  testPop3Connection: vi.fn(),
}));

describe("AccountSetup OAuth", () => {
  it("passes the selected range into OAuth before starting the first sync", async () => {
    render(<QueryClientProvider client={new QueryClient()}><AccountSetup onClose={vi.fn()} /></QueryClientProvider>);
    fireEvent.change(screen.getByLabelText("Sync range"), { target: { value: "90" } });
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Microsoft" }));
    await waitFor(() => expect(completeOAuthFlow).toHaveBeenCalledWith("outlook", "", "", undefined, undefined, undefined, 90));
    await waitFor(() => expect(startSync).toHaveBeenCalled());
    expect(vi.mocked(completeOAuthFlow).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(startSync).mock.invocationCallOrder[0]);
  });

  it("blocks OAuth when custom days are invalid", () => {
    render(<QueryClientProvider client={new QueryClient()}><AccountSetup onClose={vi.fn()} /></QueryClientProvider>);
    fireEvent.change(screen.getByLabelText("Sync range"), { target: { value: "custom" } });
    fireEvent.change(screen.getByLabelText("Number of days"), { target: { value: "0" } });
    expect((screen.getByRole("button", { name: "Sign in with Google" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Google" }));
    expect(completeOAuthFlow).not.toHaveBeenCalled();
  });
  beforeEach(() => {
    vi.mocked(completeOAuthFlow).mockResolvedValue({
      id: "account-1",
      email: "user@example.com",
      display_name: "User",
      provider: "gmail",
      created_at: 1,
      updated_at: 1,
    });
    vi.mocked(startSync).mockResolvedValue("started");
    vi.mocked(addAccount).mockResolvedValue({
      id: "pop3-account-1",
      email: "legacy@example.com",
      display_name: "Legacy",
      provider: "pop3",
      created_at: 1,
      updated_at: 1,
    });
    vi.mocked(testPop3Connection).mockResolvedValue("POP3 connection successful (0 messages)");
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("refreshes account folders after OAuth sign-in starts sync", async () => {
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const onClose = vi.fn();

    render(
      <QueryClientProvider client={queryClient}>
        <AccountSetup onClose={onClose} />
      </QueryClientProvider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Sign in with Google" }));

    await waitFor(() => {
      expect(completeOAuthFlow).toHaveBeenCalledWith("gmail", "", "", undefined, undefined, undefined, null);
    });
    await waitFor(() => {
      expect(startSync).toHaveBeenCalledWith("account-1", 3);
    });
    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: accountsQueryKey });
    });

    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: ["folders", "account-1"],
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["folders"] });
  });

  it("passes proxy settings to OAuth sign-in", async () => {
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });

    render(
      <QueryClientProvider client={queryClient}>
        <AccountSetup onClose={vi.fn()} />
      </QueryClientProvider>,
    );

    fireEvent.change(screen.getByLabelText("SOCKS5 Proxy"), {
      target: { value: "127.0.0.1" },
    });
    fireEvent.change(screen.getByLabelText("Port"), {
      target: { value: "7890" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Google" }));

    await waitFor(() => {
      expect(completeOAuthFlow).toHaveBeenCalledWith("gmail", "", "", "127.0.0.1", 7890, undefined, null);
    });
  });

  it("keeps the add-account dialog open when clicking the backdrop", () => {
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
    const onClose = vi.fn();

    render(
      <QueryClientProvider client={queryClient}>
        <AccountSetup onClose={onClose} />
      </QueryClientProvider>,
    );

    const dialog = screen.getByRole("dialog", { name: "Add Email Account" });
    fireEvent.mouseDown(dialog);
    fireEvent.click(dialog);

    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Add Email Account" })).toBeTruthy();
  });

  it("tests and submits manual POP3 account settings", async () => {
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });

    render(
      <QueryClientProvider client={queryClient}>
        <AccountSetup onClose={vi.fn()} />
      </QueryClientProvider>,
    );

    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "legacy@example.com" },
    });
    fireEvent.change(screen.getByLabelText("Sender name"), {
      target: { value: "Legacy" },
    });
    fireEvent.change(screen.getByLabelText("Incoming protocol"), {
      target: { value: "pop3" },
    });
    fireEvent.change(screen.getByLabelText("POP3 host"), {
      target: { value: "pop.example.com" },
    });
    fireEvent.change(screen.getByLabelText("SMTP host"), {
      target: { value: "smtp.example.com" },
    });
    fireEvent.change(screen.getByLabelText("Username"), {
      target: { value: "legacy-user" },
    });
    fireEvent.change(screen.getByLabelText("Password / App password"), {
      target: { value: "app-password" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));

    await waitFor(() => {
      expect(testPop3Connection).toHaveBeenCalledWith(
        "pop.example.com",
        995,
        "tls",
        false,
        undefined,
        undefined,
        "legacy-user",
        "app-password",
        false,
      );
    });

    fireEvent.click(screen.getByRole("button", { name: "Add Account & Sync" }));

    await waitFor(() => {
      expect(addAccount).toHaveBeenCalledWith(
        expect.objectContaining({
          email: "legacy@example.com",
          display_name: "Legacy",
          provider: "pop3",
          sync_range_days: null,
          imap_host: "pop.example.com",
          imap_port: 995,
          smtp_host: "smtp.example.com",
          username: "legacy-user",
          password: "app-password",
        }),
      );
    });
  });

  it("allows manual IMAP account submission with an empty username", async () => {
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });

    render(
      <QueryClientProvider client={queryClient}>
        <AccountSetup onClose={vi.fn()} />
      </QueryClientProvider>,
    );

    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "user@hotmail.com" },
    });
    fireEvent.change(screen.getByLabelText("Sync range"), { target: { value: "30" } });
    fireEvent.change(screen.getByLabelText("Sender name"), {
      target: { value: "Hotmail" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Outlook" }));
    fireEvent.change(screen.getByLabelText("Username"), {
      target: { value: "" },
    });
    fireEvent.change(screen.getByLabelText("Password / App password"), {
      target: { value: "app-password" },
    });

    expect((screen.getByLabelText("Username") as HTMLInputElement).required).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "Add Account & Sync" }));

    await waitFor(() => {
      expect(addAccount).toHaveBeenCalledWith(
        expect.objectContaining({
          email: "user@hotmail.com",
          display_name: "Hotmail",
          provider: "imap",
          sync_range_days: 30,
          imap_host: "outlook.office365.com",
          smtp_host: "smtp.office365.com",
          username: "",
          password: "app-password",
        }),
      );
    });
  });

  it("keeps a manually edited Outlook username independent from the mailbox address", () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });

    render(
      <QueryClientProvider client={queryClient}>
        <AccountSetup onClose={vi.fn()} />
      </QueryClientProvider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Outlook" }));
    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "mailbox@outlook.com" },
    });
    fireEvent.change(screen.getByLabelText("Username"), {
      target: { value: "external-login@qq.com" },
    });
    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "renamed-mailbox@outlook.com" },
    });

    expect((screen.getByLabelText("Username") as HTMLInputElement).value).toBe(
      "external-login@qq.com",
    );
  });

  it("clears the plaintext opt-in when security switches back to encrypted", async () => {
    // Issue #70 review: the opt-in checkbox only shows while a connection is
    // unencrypted. Checking it and then switching both sides back to an
    // encrypted mode must NOT persist allow_plaintext=true on the account.
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });

    render(
      <QueryClientProvider client={queryClient}>
        <AccountSetup onClose={vi.fn()} />
      </QueryClientProvider>,
    );

    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "legacy@88.com" },
    });
    fireEvent.change(screen.getByLabelText("Sender name"), {
      target: { value: "Legacy" },
    });
    fireEvent.change(screen.getByLabelText("IMAP host"), {
      target: { value: "mail.88.com" },
    });
    fireEvent.change(screen.getByLabelText("SMTP host"), {
      target: { value: "mail.88.com" },
    });
    fireEvent.change(screen.getByLabelText("Password / App password"), {
      target: { value: "app-password" },
    });

    const imapSecurity = document.getElementById(
      "setup-imap-security",
    ) as HTMLSelectElement;
    const smtpSecurity = document.getElementById(
      "setup-smtp-security",
    ) as HTMLSelectElement;

    // Go plaintext on both sides, opt in.
    fireEvent.change(imapSecurity, { target: { value: "plain" } });
    fireEvent.change(smtpSecurity, { target: { value: "plain" } });
    const optIn = screen.getByRole("checkbox", {
      name: /Allow unencrypted connection/,
    });
    fireEvent.click(optIn);
    expect((optIn as HTMLInputElement).checked).toBe(true);

    // Change our mind: switch both back to encrypted. The checkbox disappears
    // and the flag must reset.
    fireEvent.change(imapSecurity, { target: { value: "tls" } });
    fireEvent.change(smtpSecurity, { target: { value: "starttls" } });
    expect(
      screen.queryByRole("checkbox", { name: /Allow unencrypted connection/ }),
    ).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Add Account & Sync" }));

    await waitFor(() => {
      expect(addAccount).toHaveBeenCalledWith(
        expect.objectContaining({ allow_plaintext: false }),
      );
    });
  });
});
