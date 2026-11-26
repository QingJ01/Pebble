import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Contact } from "@/lib/api";
import { useComposeStore } from "@/stores/compose.store";
import { useConfirmStore } from "@/stores/confirm.store";
import { useUIStore } from "@/stores/ui.store";

const mocks = vi.hoisted(() => ({
  contacts: [] as Contact[],
  useContactsQuery: vi.fn(),
  save: vi.fn(),
  remove: vi.fn(),
  setFavorite: vi.fn(),
  confirm: vi.fn(),
  importVcard: vi.fn(),
  exportVcard: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string, fallbackOrOptions?: string | { count?: number; defaultValue?: string }) => {
      if (key === "contacts.count" && typeof fallbackOrOptions === "object") {
        return `translated count ${fallbackOrOptions.count}`;
      }
      if (typeof fallbackOrOptions === "string") return fallbackOrOptions;
      const opts = (fallbackOrOptions ?? {}) as Record<string, unknown>;
      const template = typeof opts.defaultValue === "string" ? opts.defaultValue : key;
      return template.replace(/\{\{(\w+)\}\}/g, (_m: string, n: string) => String(opts[n] ?? ""));
    },
  }),
}));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 60,
    getVirtualItems: () => Array.from(
      { length: Math.min(count, 10) },
      (_, index) => ({ index, key: `contact-row-${index}`, start: index * 60 }),
    ),
    measureElement: vi.fn(),
    scrollToIndex: vi.fn(),
  }),
}));

vi.mock("@/hooks/queries", () => ({
  useContactsQuery: (options: unknown) => mocks.useContactsQuery(options),
}));

vi.mock("@/hooks/mutations", () => ({
  useContactMutations: () => ({
    save: { mutateAsync: mocks.save, isPending: false },
    remove: { mutateAsync: mocks.remove, isPending: false },
    setFavorite: { mutateAsync: mocks.setFavorite, isPending: false },
  }),
}));

vi.mock("@/lib/api", () => ({
  importContactsVcard: (data: string) => mocks.importVcard(data),
  exportContactsVcard: () => mocks.exportVcard(),
}));

import ContactsView from "@/features/contacts/ContactsView";

const alice: Contact = {
  id: "contact-1",
  display_name: "Alice",
  notes: "Rust community",
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

function contact(index: number): Contact {
  return {
    ...alice,
    id: `contact-${index}`,
    display_name: `Contact ${index}`,
    emails: [{
      ...alice.emails[0],
      id: `email-${index}`,
      address: `contact-${index}@example.com`,
    }],
  };
}

describe("ContactsView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.contacts = [];
    mocks.useContactsQuery.mockImplementation(() => ({
      data: mocks.contacts,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    }));
    mocks.save.mockImplementation(async () => alice);
    mocks.remove.mockResolvedValue(undefined);
    mocks.setFavorite.mockResolvedValue(undefined);
    mocks.confirm.mockResolvedValue(true);
    mocks.importVcard.mockResolvedValue({
      created: 1,
      merged: 2,
      skipped: 3,
      invalid: 4,
      errors: ["Card 4: invalid email"],
    });
    mocks.exportVcard.mockResolvedValue("BEGIN:VCARD\r\nEND:VCARD\r\n");
    useConfirmStore.setState({ confirm: mocks.confirm });
    useUIStore.setState({ activeView: "contacts" as never });
    useComposeStore.setState({
      composeMode: null,
      composePrefill: null,
      composeReplyTo: null,
      composeDirty: false,
    });
  });

  it("shows an empty state and opens the new contact editor", () => {
    render(<ContactsView />);

    expect(screen.getByText("No contacts yet")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "New contact" })[0]);
    expect(screen.getByRole("dialog", { name: "New contact" })).toBeTruthy();
  });

  it("passes search and favorite filters to the contacts query", () => {
    render(<ContactsView />);

    fireEvent.change(screen.getByLabelText("Search contacts"), {
      target: { value: "alice" },
    });
    fireEvent.click(screen.getByLabelText("Favorites only"));

    expect(mocks.useContactsQuery).toHaveBeenLastCalledWith(expect.objectContaining({
      query: "alice",
      favoriteOnly: true,
      limit: Number.MAX_SAFE_INTEGER,
    }));
  });

  it("uses the translated count and only mounts virtualized contact rows", () => {
    mocks.contacts = Array.from({ length: 100 }, (_, index) => contact(index));

    render(<ContactsView />);

    expect(screen.getByLabelText("translated count 100")).toBeTruthy();
    expect(screen.getAllByRole("listitem")).toHaveLength(10);
  });

  it("opens contact details and composes to the primary email", () => {
    mocks.contacts = [alice];
    render(<ContactsView />);

    fireEvent.click(screen.getByRole("button", { name: /Alice.*alice@example.com/i }));
    fireEvent.click(screen.getByRole("button", { name: "Write email" }));

    expect(useUIStore.getState().activeView).toBe("compose");
    expect(useComposeStore.getState().composePrefill).toEqual({
      to: ["alice@example.com"],
    });
  });

  it("toggles favorite and deletes after confirmation", async () => {
    mocks.contacts = [alice];
    render(<ContactsView />);
    fireEvent.click(screen.getByRole("button", { name: /Alice.*alice@example.com/i }));

    fireEvent.click(screen.getByRole("button", { name: "Add to favorites" }));
    await waitFor(() => {
      expect(mocks.setFavorite).toHaveBeenCalledWith({
        contactId: "contact-1",
        isFavorite: true,
      });
    });

    fireEvent.click(screen.getByRole("button", { name: "Delete contact" }));
    await waitFor(() => expect(mocks.confirm).toHaveBeenCalled());
    await waitFor(() => {
      expect(mocks.remove).toHaveBeenCalledWith({
        contactId: "contact-1",
        suppressAddresses: true,
      });
    });
  });

  it("imports a vCard file and shows the result summary", async () => {
    render(<ContactsView />);
    const file = new File(["BEGIN:VCARD\r\nEND:VCARD\r\n"], "contacts.vcf", {
      type: "text/vcard",
    });
    Object.defineProperty(file, "text", {
      value: vi.fn().mockResolvedValue("BEGIN:VCARD\r\nEND:VCARD\r\n"),
    });

    fireEvent.change(screen.getByLabelText("Choose vCard file"), {
      target: { files: [file] },
    });

    await waitFor(() => {
      expect(mocks.importVcard).toHaveBeenCalledWith("BEGIN:VCARD\r\nEND:VCARD\r\n");
    });
    expect(await screen.findByRole("status")).toBeTruthy();
    expect(document.body.textContent).toContain("1 created");
    expect(document.body.textContent).toContain("2 merged");
    expect(document.body.textContent).toContain("4 invalid");
    expect(document.body.textContent).toContain("Card 4: invalid email");
  });

  it("exports saved contacts as a vCard download", async () => {
    const createObjectURL = vi.fn().mockReturnValue("blob:contacts");
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, "createObjectURL", { configurable: true, value: createObjectURL });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: revokeObjectURL });
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    render(<ContactsView />);

    fireEvent.click(screen.getByRole("button", { name: "Export vCard" }));

    await waitFor(() => expect(mocks.exportVcard).toHaveBeenCalled());
    expect(createObjectURL).toHaveBeenCalled();
    expect(click).toHaveBeenCalled();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:contacts");
    click.mockRestore();
  });
});
