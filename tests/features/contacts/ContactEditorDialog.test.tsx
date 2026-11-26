import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Contact } from "@/lib/api";
import ContactEditorDialog from "@/features/contacts/ContactEditorDialog";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: unknown) => {
      if (typeof options === "string") return options;
      const opts = (options ?? {}) as Record<string, unknown>;
      const template = typeof opts.defaultValue === "string" ? opts.defaultValue : key;
      return template.replace(/\{\{(\w+)\}\}/g, (_m: string, n: string) => String(opts[n] ?? ""));
    },
  }),
}));

const contact: Contact = {
  id: "contact-1",
  display_name: "Alice",
  notes: "Old note",
  is_favorite: true,
  emails: [{
    id: "email-1",
    address: "alice@example.com",
    label: "work",
    is_primary: true,
  }],
  created_at: 1,
  updated_at: 1,
};

describe("ContactEditorDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("creates a contact with a labeled primary email", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<ContactEditorDialog contact={null} onClose={vi.fn()} onSave={onSave} />);

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "  Bob  " } });
    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: " bob@example.com " },
    });
    fireEvent.change(screen.getByLabelText("Email label"), { target: { value: "personal" } });
    fireEvent.click(screen.getByRole("button", { name: "Save contact" }));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith({
        display_name: "Bob",
        notes: "",
        is_favorite: false,
        emails: [{
          id: undefined,
          address: "bob@example.com",
          label: "personal",
          is_primary: true,
        }],
      });
    });
  });

  it("blocks an invalid email before saving", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<ContactEditorDialog contact={null} onClose={vi.fn()} onSave={onSave} />);

    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "invalid" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save contact" }));

    const alert = await screen.findByRole("alert");
    const emailInput = screen.getByLabelText("Email address");
    expect(alert.textContent).toContain("Enter a valid email address");
    expect(emailInput.getAttribute("aria-invalid")).toBe("true");
    expect(emailInput.getAttribute("aria-describedby")).toBe(alert.id);
    expect(onSave).not.toHaveBeenCalled();
  });

  it("associates duplicate-email validation with every email input", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<ContactEditorDialog contact={null} onClose={vi.fn()} onSave={onSave} />);

    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "duplicate@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add email" }));
    const emailInputs = screen.getAllByLabelText("Email address");
    fireEvent.change(emailInputs[1], { target: { value: " DUPLICATE@example.com " } });
    fireEvent.click(screen.getByRole("button", { name: "Save contact" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Each email address can only be added once");
    for (const input of emailInputs) {
      expect(input.getAttribute("aria-invalid")).toBe("true");
      expect(input.getAttribute("aria-describedby")).toBe(alert.id);
    }
    expect(onSave).not.toHaveBeenCalled();
  });

  it("associates one-primary validation with the email group", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const invalidPrimaryContact: Contact = {
      ...contact,
      emails: [
        contact.emails[0],
        {
          id: "email-2",
          address: "alice.personal@example.com",
          label: "personal",
          is_primary: true,
        },
      ],
    };
    render(
      <ContactEditorDialog
        contact={invalidPrimaryContact}
        onClose={vi.fn()}
        onSave={onSave}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Save contact" }));

    const alert = await screen.findByRole("alert");
    const emailGroup = screen.getByRole("group", { name: "Email address" });
    expect(alert.textContent).toContain("Choose exactly one primary email");
    expect(emailGroup.getAttribute("aria-invalid")).toBe("true");
    expect(emailGroup.getAttribute("aria-describedby")).toBe(alert.id);
    expect(onSave).not.toHaveBeenCalled();
  });

  it("enforces name and notes limits and associates each error with its field", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<ContactEditorDialog contact={null} onClose={vi.fn()} onSave={onSave} />);

    const nameInput = screen.getByLabelText("Name");
    const emailInput = screen.getByLabelText("Email address");
    const notesInput = screen.getByLabelText("Notes");
    expect(nameInput.getAttribute("maxlength")).toBe("512");
    expect(notesInput.getAttribute("maxlength")).toBe("2000");

    fireEvent.change(emailInput, { target: { value: "valid@example.com" } });
    fireEvent.change(nameInput, { target: { value: "n".repeat(513) } });
    fireEvent.click(screen.getByRole("button", { name: "Save contact" }));

    let alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Name must be 512 characters or fewer");
    expect(nameInput.getAttribute("aria-invalid")).toBe("true");
    expect(nameInput.getAttribute("aria-describedby")).toBe(alert.id);

    fireEvent.change(nameInput, { target: { value: "Valid name" } });
    fireEvent.change(notesInput, { target: { value: "n".repeat(2001) } });
    fireEvent.click(screen.getByRole("button", { name: "Save contact" }));

    alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Notes must be 2000 characters or fewer");
    expect(notesInput.getAttribute("aria-invalid")).toBe("true");
    expect(notesInput.getAttribute("aria-describedby")).toBe(alert.id);
    expect(onSave).not.toHaveBeenCalled();
  });

  it("keeps a primary email after add/remove and saves favorite state", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<ContactEditorDialog contact={null} onClose={vi.fn()} onSave={onSave} />);

    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "remove@example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add email" }));
    const emailInputs = screen.getAllByLabelText("Email address");
    fireEvent.change(emailInputs[1], { target: { value: "keep@example.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Remove email 1" }));
    fireEvent.click(screen.getByLabelText("Favorite contact"));
    fireEvent.click(screen.getByRole("button", { name: "Save contact" }));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
        is_favorite: true,
        emails: [expect.objectContaining({
          address: "keep@example.com",
          is_primary: true,
        })],
      }));
    });
  });

  it("traps focus inside the dialog and restores the invoking control", () => {
    const trigger = document.createElement("button");
    trigger.textContent = "Open editor";
    document.body.appendChild(trigger);
    trigger.focus();

    const { unmount } = render(
      <ContactEditorDialog contact={null} onClose={vi.fn()} onSave={vi.fn()} />,
    );

    expect(document.activeElement).toBe(screen.getByLabelText("Name"));
    const closeButton = screen.getByRole("button", { name: "Close" });
    const saveButton = screen.getByRole("button", { name: "Save contact" });

    closeButton.focus();
    fireEvent.keyDown(document, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(saveButton);

    saveButton.focus();
    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(closeButton);

    unmount();
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });

  it("preserves ids while editing and closes on Escape", async () => {
    const onClose = vi.fn();
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<ContactEditorDialog contact={contact} onClose={onClose} onSave={onSave} />);

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Alice Updated" } });
    fireEvent.click(screen.getByRole("button", { name: "Save contact" }));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith(expect.objectContaining({
        id: "contact-1",
        display_name: "Alice Updated",
        emails: [expect.objectContaining({ id: "email-1" })],
      }));
    });
    await waitFor(() => {
      expect((screen.getByRole("button", { name: "Save contact" }) as HTMLButtonElement).disabled)
        .toBe(false);
    });

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});
