import { useRef, useState, type FormEvent } from "react";
import { Plus, Trash2, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type {
  Contact,
  ContactEmailInput,
  ContactEmailLabel,
  ContactInput,
} from "@/lib/api";
import { formatError } from "@/lib/extractErrorMessage";
import Dialog from "@/components/ui/Dialog";
import Button from "@/components/ui/Button";
import IconButton from "@/components/ui/IconButton";
import { isValidEmailAddress } from "@/features/compose/recipient-utils";
import { fieldGroupStyle, inputStyle, labelStyle } from "@/styles/form";

interface ContactEditorDialogProps {
  contact: Contact | null;
  initialValue?: { displayName?: string | null; address: string };
  onClose: () => void;
  onSave: (input: ContactInput) => Promise<void>;
}

interface DraftEmail extends ContactEmailInput {
  key: string;
}

type ErrorField = "name" | "emails" | "notes" | "form";

interface FormError {
  field: ErrorField;
  message: string;
}

let draftEmailId = 0;
const CONTACT_EDITOR_ERROR_ID = "contact-editor-error";

function makeDraftEmail(email?: Contact["emails"][number], initialAddress = ""): DraftEmail {
  return {
    key: email?.id ?? `new-email-${++draftEmailId}`,
    id: email?.id,
    address: email?.address ?? initialAddress,
    label: email?.label ?? "work",
    is_primary: email?.is_primary ?? true,
  };
}

export default function ContactEditorDialog({
  contact,
  initialValue,
  onClose,
  onSave,
}: ContactEditorDialogProps) {
  const { t } = useTranslation();
  const nameRef = useRef<HTMLInputElement>(null);
  const [displayName, setDisplayName] = useState(
    contact?.display_name ?? initialValue?.displayName ?? "",
  );
  const [notes, setNotes] = useState(contact?.notes ?? "");
  const [isFavorite, setIsFavorite] = useState(contact?.is_favorite ?? false);
  const [emails, setEmails] = useState<DraftEmail[]>(
    contact?.emails.length
      ? contact.emails.map((email) => makeDraftEmail(email))
      : [makeDraftEmail(undefined, initialValue?.address ?? "")],
  );
  const [error, setError] = useState<FormError | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  const title = contact
    ? t("contacts.edit", "Edit contact")
    : t("contacts.new", "New contact");



  const updateEmail = (key: string, patch: Partial<DraftEmail>) => {
    setEmails((current) => current.map((email) => (
      email.key === key ? { ...email, ...patch } : email
    )));
  };

  const makePrimary = (key: string) => {
    setEmails((current) => current.map((email) => ({
      ...email,
      is_primary: email.key === key,
    })));
  };

  const addEmail = () => {
    setEmails((current) => [
      ...current,
      { ...makeDraftEmail(), is_primary: current.length === 0 },
    ]);
  };

  const removeEmail = (key: string) => {
    setEmails((current) => {
      const next = current.filter((email) => email.key !== key);
      if (next.length > 0 && !next.some((email) => email.is_primary)) {
        next[0] = { ...next[0], is_primary: true };
      }
      return next;
    });
  };

  const validate = (): FormError | null => {
    if (displayName.trim().length > 512) {
      return {
        field: "name",
        message: t("contacts.nameTooLong", "Name must be 512 characters or fewer"),
      };
    }
    if (emails.length === 0) {
      return {
        field: "emails",
        message: t("contacts.emailRequired", "Add at least one email address"),
      };
    }
    if (emails.some((email) => !isValidEmailAddress(email.address))) {
      return {
        field: "emails",
        message: t("contacts.invalidEmail", "Enter a valid email address"),
      };
    }
    if (emails.filter((email) => email.is_primary).length !== 1) {
      return {
        field: "emails",
        message: t("contacts.primaryRequired", "Choose exactly one primary email"),
      };
    }
    const normalized = emails.map((email) => email.address.trim().toLowerCase());
    if (new Set(normalized).size !== normalized.length) {
      return {
        field: "emails",
        message: t("contacts.duplicateEmail", "Each email address can only be added once"),
      };
    }
    if (notes.trim().length > 2000) {
      return {
        field: "notes",
        message: t("contacts.notesTooLong", "Notes must be 2000 characters or fewer"),
      };
    }
    return null;
  };

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    const validationError = validate();
    if (validationError) {
      setError(validationError);
      return;
    }

    setError(null);
    setIsSaving(true);
    try {
      await onSave({
        ...(contact ? { id: contact.id } : {}),
        display_name: displayName.trim(),
        notes: notes.trim(),
        is_favorite: isFavorite,
        emails: emails.map(({ id, address, label, is_primary }) => ({
          id,
          address: address.trim(),
          label,
          is_primary,
        })),
      });
    } catch (saveError) {
      setError({ field: "form", message: formatError(saveError, { fallbackKey: "contacts.saveFailed", fallback: "Couldn't save the contact. Try again." }) });
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog
      onClose={onClose}
      dismissible={!isSaving}
      busy={isSaving}
      labelledBy="contact-editor-title"
      initialFocus={nameRef}
      panelClassName="contact-dialog"
    >
        <header className="contact-dialog-header">
          <div>
            <span className="contact-dialog-kicker">{t("contacts.title", "Contacts")}</span>
            <h2 id="contact-editor-title">{title}</h2>
          </div>
          <IconButton
            size="md"
            label={t("common.close", "Close")}
            icon={<X size={16} />}
            onClick={onClose}
            disabled={isSaving}
          />
        </header>

        <form onSubmit={handleSubmit} noValidate>
          <div className="contact-dialog-body">
            <div style={fieldGroupStyle}>
              <label style={labelStyle} htmlFor="contact-name">
                {t("contacts.name", "Name")}
              </label>
              <input
                ref={nameRef}
                id="contact-name"
                style={inputStyle}
                value={displayName}
                maxLength={512}
                aria-invalid={error?.field === "name" || undefined}
                aria-describedby={error?.field === "name" ? CONTACT_EDITOR_ERROR_ID : undefined}
                onChange={(event) => setDisplayName(event.target.value)}
                autoComplete="name"
              />
            </div>

            <fieldset
              className="contact-email-fieldset"
              aria-invalid={error?.field === "emails" || undefined}
              aria-describedby={error?.field === "emails" ? CONTACT_EDITOR_ERROR_ID : undefined}
            >
              <legend>{t("contacts.emailAddress", "Email address")}</legend>
              <div className="contact-email-stack">
                {emails.map((email, index) => {
                  const addressId = `contact-email-${email.key}`;
                  const labelId = `contact-email-label-${email.key}`;
                  return (
                    <div className="contact-email-row" key={email.key}>
                      <div className="contact-email-fields">
                        <div>
                          <label style={labelStyle} htmlFor={addressId}>
                            {t("contacts.emailAddress", "Email address")}
                          </label>
                          <input
                            id={addressId}
                            style={inputStyle}
                            type="email"
                            value={email.address}
                            aria-invalid={error?.field === "emails" || undefined}
                            aria-describedby={error?.field === "emails"
                              ? CONTACT_EDITOR_ERROR_ID
                              : undefined}
                            onChange={(event) => updateEmail(email.key, { address: event.target.value })}
                            autoComplete="email"
                          />
                        </div>
                        <div>
                          <label style={labelStyle} htmlFor={labelId}>
                            {t("contacts.emailLabel", "Email label")}
                          </label>
                          <select
                            id={labelId}
                            style={inputStyle}
                            value={email.label}
                            onChange={(event) => updateEmail(email.key, {
                              label: event.target.value as ContactEmailLabel,
                            })}
                          >
                            <option value="work">{t("contacts.work", "Work")}</option>
                            <option value="personal">{t("contacts.personal", "Personal")}</option>
                            <option value="other">{t("contacts.other", "Other")}</option>
                          </select>
                        </div>
                      </div>
                      <div className="contact-email-controls">
                        <label className="contact-primary-control">
                          <input
                            type="radio"
                            name="primary-contact-email"
                            checked={email.is_primary}
                            onChange={() => makePrimary(email.key)}
                          />
                          {email.is_primary
                            ? t("contacts.primary", "Primary")
                            : t("contacts.setPrimary", "Set as primary")}
                        </label>
                        <button
                          type="button"
                          className="contact-inline-danger"
                          aria-label={t("contacts.removeEmailNumbered", { index: index + 1, defaultValue: "Remove email {{index}}" })}
                          onClick={() => removeEmail(email.key)}
                        >
                          <Trash2 size={14} />
                          {t("contacts.removeEmail", "Remove email")}
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
              <button type="button" className="contact-text-button" onClick={addEmail}>
                <Plus size={14} />
                {t("contacts.addEmail", "Add email")}
              </button>
            </fieldset>

            <div style={fieldGroupStyle}>
              <label style={labelStyle} htmlFor="contact-notes">
                {t("contacts.notes", "Notes")}
              </label>
              <textarea
                id="contact-notes"
                style={{ ...inputStyle, minHeight: 86, resize: "vertical" }}
                value={notes}
                maxLength={2000}
                aria-invalid={error?.field === "notes" || undefined}
                aria-describedby={error?.field === "notes" ? CONTACT_EDITOR_ERROR_ID : undefined}
                onChange={(event) => setNotes(event.target.value)}
              />
              <span className="contact-character-count">{notes.length}/2000</span>
            </div>

            <label className="contact-favorite-control">
              <input
                type="checkbox"
                checked={isFavorite}
                onChange={(event) => setIsFavorite(event.target.checked)}
              />
              {t("contacts.favorite", "Favorite contact")}
            </label>

            {error && (
              <p id={CONTACT_EDITOR_ERROR_ID} className="contact-form-error" role="alert">
                {error.message}
              </p>
            )}
          </div>

          <footer className="contact-dialog-footer">
            <Button variant="secondary" onClick={onClose} disabled={isSaving}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="submit" variant="primary" loading={isSaving}>
              {isSaving ? t("common.saving", "Saving...") : t("contacts.save", "Save contact")}
            </Button>
          </footer>
        </form>
    </Dialog>
  );
}
