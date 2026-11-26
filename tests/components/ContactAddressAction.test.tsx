import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Contact } from "@/lib/api";
import { getContactByEmail } from "@/lib/api";
import { useUIStore } from "@/stores/ui.store";
import ContactAddressAction from "@/components/ContactAddressAction";

const mocks = vi.hoisted(() => ({
  accounts: [{ id: "account-1", email: "me@example.com" }],
  save: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    t: (key: string, options?: unknown) => {
      if (typeof options === "string") return options;
      const opts = (options ?? {}) as Record<string, unknown>;
      const template = typeof opts.defaultValue === "string" ? opts.defaultValue : key;
      return template.replace(/\{\{(\w+)\}\}/g, (_m: string, n: string) => String(opts[n] ?? ""));
    },
  }),
}));

vi.mock("@/lib/api", () => ({
  getContactByEmail: vi.fn(),
}));

vi.mock("@/hooks/queries", () => ({
  useAccountsQuery: () => ({ data: mocks.accounts }),
}));

vi.mock("@/hooks/mutations", () => ({
  useContactMutations: () => ({
    save: { mutateAsync: mocks.save, isPending: false },
  }),
}));

const alice: Contact = {
  id: "contact-1",
  display_name: "Alice",
  notes: "",
  is_favorite: false,
  emails: [{
    id: "email-1",
    address: "alice@example.com",
    label: "work",
    is_primary: true,
  }],
  created_at: 1,
  updated_at: 1,
};

function renderAction(element: React.ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{element}</QueryClientProvider>,
  );
}

describe("ContactAddressAction", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.accounts = [{ id: "account-1", email: "me@example.com" }];
    vi.mocked(getContactByEmail).mockResolvedValue(null);
    mocks.save.mockResolvedValue(alice);
    useUIStore.setState({ activeView: "inbox", pendingContactId: null });
  });

  it("does not map the original participant to a contact saved with another address", async () => {
    mocks.save.mockResolvedValue({ ...alice, display_name: "Bob", emails: [{ ...alice.emails[0], address: "bob@example.com" }] });
    renderAction(<ContactAddressAction accountId="account-1" name="Alice" address="alice@example.com" />);
    const addButton = await screen.findByRole("button", { name: "Add alice@example.com to contacts" });
    await waitFor(() => expect((addButton as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(addButton);
    fireEvent.change(screen.getByLabelText("Email address"), { target: { value: "bob@example.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Save contact" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByRole("button", { name: "View Bob in contacts" })).toBeNull();
    expect(screen.getByRole("button", { name: "Add alice@example.com to contacts" })).toBeTruthy();
  });

  it("opens a prefilled editor for an unsaved participant", async () => {
    renderAction(
      <ContactAddressAction
        accountId="account-1"
        name="Sender"
        address="sender@example.com"
      />,
    );

    const addButton = await screen.findByRole("button", {
      name: "Add sender@example.com to contacts",
    });
    await waitFor(() => expect((addButton as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(addButton);

    expect(screen.getByRole("dialog", { name: "New contact" })).toBeTruthy();
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Sender");
    expect((screen.getByLabelText("Email address") as HTMLInputElement).value).toBe("sender@example.com");
  });

  it("opens an existing contact in the contacts view", async () => {
    vi.mocked(getContactByEmail).mockResolvedValue(alice);
    renderAction(
      <ContactAddressAction
        accountId="account-1"
        name="Alice"
        address="alice@example.com"
      />,
    );

    fireEvent.click(await screen.findByRole("button", {
      name: "View Alice in contacts",
    }));

    await waitFor(() => {
      expect(useUIStore.getState().activeView).toBe("contacts");
      expect(useUIStore.getState().pendingContactId).toBe("contact-1");
    });
  });

  it("does not offer contact actions for the current account address", async () => {
    renderAction(
      <ContactAddressAction
        accountId="account-1"
        name="Me"
        address="ME@example.com"
      />,
    );

    await waitFor(() => expect(getContactByEmail).not.toHaveBeenCalled());
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("deduplicates identical participant lookups through the query cache", async () => {
    renderAction(
      <>
        <ContactAddressAction
          accountId="account-1"
          name="Alice"
          address="Alice@example.com"
        />
        <ContactAddressAction
          accountId="account-1"
          name="Alice duplicate"
          address=" alice@EXAMPLE.com "
        />
      </>,
    );

    await waitFor(() => {
      expect(getContactByEmail).toHaveBeenCalledTimes(1);
      expect(getContactByEmail).toHaveBeenCalledWith("alice@example.com");
    });
  });
});
