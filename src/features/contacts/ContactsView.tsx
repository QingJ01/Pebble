import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeft,
  Clipboard,
  Download,
  Mail,
  Pencil,
  Plus,
  Search,
  Star,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  contactSuggestionsQueryRoot,
  contactsQueryRoot,
  useContactsQuery,
} from "@/hooks/queries";
import { useContactMutations } from "@/hooks/mutations";
import {
  exportContactsVcard,
  importContactsVcard,
  type Contact,
  type ContactInput,
  type VcardImportResult,
} from "@/lib/api";
import { queryClient } from "@/lib/query-client";
import { useComposeStore } from "@/stores/compose.store";
import { useConfirmStore } from "@/stores/confirm.store";
import { toastError, useToastStore } from "@/stores/toast.store";
import { useUIStore } from "@/stores/ui.store";
import Button from "@/components/ui/Button";
import IconButton from "@/components/ui/IconButton";
import Checkbox from "@/components/ui/Checkbox";
import ContactEditorDialog from "./ContactEditorDialog";
import ContactListItem from "./ContactListItem";

const EMPTY_CONTACTS: Contact[] = [];
const MAX_VCARD_FILE_SIZE = 5 * 1024 * 1024;

function primaryEmailFor(contact: Contact) {
  return contact.emails.find((email) => email.is_primary) ?? contact.emails[0];
}

export default function ContactsView() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [favoriteOnly, setFavoriteOnly] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editorContact, setEditorContact] = useState<Contact | null | undefined>(undefined);
  const [importResult, setImportResult] = useState<VcardImportResult | null>(null);
  const [isImporting, setIsImporting] = useState(false);
  const [isExporting, setIsExporting] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const listPaneRef = useRef<HTMLDivElement>(null);
  const { data = EMPTY_CONTACTS, isLoading, error, refetch } = useContactsQuery({
    query,
    favoriteOnly,
    limit: Number.MAX_SAFE_INTEGER,
    offset: 0,
  });
  const contacts = data;
  const contactListVirtualizer = useVirtualizer({
    count: contacts.length,
    getScrollElement: () => listPaneRef.current,
    estimateSize: () => 49,
    measureElement: (element) => element.getBoundingClientRect().height,
    getItemKey: (index) => contacts[index]?.id ?? index,
    overscan: 8,
  });
  const { save, remove, setFavorite } = useContactMutations();
  const addToast = useToastStore((state) => state.addToast);
  const confirm = useConfirmStore((state) => state.confirm);
  const pendingContactId = useUIStore((state) => state.pendingContactId);
  const clearPendingContact = useUIStore((state) => state.clearPendingContact);

  const selectedContact = useMemo(
    () => contacts.find((contact) => contact.id === selectedId) ?? null,
    [contacts, selectedId],
  );

  useEffect(() => {
    if (selectedId && !contacts.some((contact) => contact.id === selectedId)) {
      setSelectedId(null);
    }
  }, [contacts, selectedId]);

  useEffect(() => {
    if (!selectedId) return;
    const index = contacts.findIndex((contact) => contact.id === selectedId);
    if (index >= 0) contactListVirtualizer.scrollToIndex(index, { align: "auto" });
  }, [contactListVirtualizer, contacts, selectedId]);

  useEffect(() => {
    if (!pendingContactId) return;
    if (contacts.some((contact) => contact.id === pendingContactId)) {
      setSelectedId(pendingContactId);
      clearPendingContact();
    }
  }, [clearPendingContact, contacts, pendingContactId]);

  const handleSave = async (input: ContactInput) => {
    const saved = await save.mutateAsync(input);
    setSelectedId(saved.id);
    setEditorContact(undefined);
    addToast({
      message: t("contacts.saveSuccess", "Contact saved"),
      type: "success",
    });
  };

  const handleFavorite = async (contact: Contact) => {
    try {
      await setFavorite.mutateAsync({
        contactId: contact.id,
        isFavorite: !contact.is_favorite,
      });
    } catch (favoriteError) {
      toastError(favoriteError, "contacts.favoriteFailed");
    }
  };

  const handleDelete = async (contact: Contact) => {
    const accepted = await confirm({
      title: t("contacts.delete", "Delete contact"),
      message: t(
        "contacts.deleteConfirm",
        "Delete this contact and hide its addresses from recent suggestions?",
      ),
      destructive: true,
      confirmLabel: t("contacts.delete", "Delete contact"),
    });
    if (!accepted) return;

    try {
      await remove.mutateAsync({ contactId: contact.id, suppressAddresses: true });
      setSelectedId(null);
      addToast({
        message: t("contacts.deleteSuccess", "Contact deleted"),
        type: "success",
      });
    } catch (deleteError) {
      toastError(deleteError, "contacts.deleteFailed");
    }
  };

  const writeToContact = (contact: Contact) => {
    const address = primaryEmailFor(contact)?.address;
    if (!address) return;
    useComposeStore.getState().openCompose("new", null, { to: [address] });
  };

  const copyAddress = async (address: string) => {
    try {
      await navigator.clipboard.writeText(address);
      addToast({
        message: t("contacts.copySuccess", "Email address copied"),
        type: "success",
      });
    } catch (copyError) {
      toastError(copyError, "contacts.copyFailed");
    }
  };

  const handleImport = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;

    if (file.size > MAX_VCARD_FILE_SIZE) {
      addToast({
        message: t("contacts.vcardFileTooLarge", "vCard files must be 5 MB or smaller"),
        type: "error",
      });
      return;
    }

    setIsImporting(true);
    try {
      const result = await importContactsVcard(await file.text());
      setImportResult(result);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: contactsQueryRoot }),
        queryClient.invalidateQueries({ queryKey: contactSuggestionsQueryRoot }),
      ]);
      addToast({
        message: t("contacts.importSuccess", "vCard import complete"),
        type: "success",
      });
    } catch (importError) {
      toastError(importError, "contacts.importFailed");
    } finally {
      setIsImporting(false);
    }
  };

  const handleExport = async () => {
    setIsExporting(true);
    let downloadUrl: string | null = null;
    try {
      const data = await exportContactsVcard();
      downloadUrl = URL.createObjectURL(new Blob([data], { type: "text/vcard;charset=utf-8" }));
      const anchor = document.createElement("a");
      anchor.href = downloadUrl;
      anchor.download = "pebble-contacts.vcf";
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      addToast({
        message: t("contacts.exportSuccess", "Contacts exported"),
        type: "success",
      });
    } catch (exportError) {
      toastError(exportError, "contacts.exportFailed");
    } finally {
      if (downloadUrl) URL.revokeObjectURL(downloadUrl);
      setIsExporting(false);
    }
  };

  return (
    <section className="contacts-view" aria-labelledby="contacts-view-title">
      <header className="page-header">
        <div className="page-header__text">
          <div className="contacts-title-row">
            <h1 id="contacts-view-title" className="page-header__title">{t("contacts.title", "Contacts")}</h1>
            <span
              className="contacts-count"
              aria-label={t("contacts.count", { count: contacts.length })}
            >
              {contacts.length}
            </span>
          </div>
        </div>
        <div className="page-header__actions">
          <input
            ref={fileInputRef}
            className="sr-only"
            type="file"
            accept=".vcf,text/vcard,text/x-vcard"
            aria-label={t("contacts.chooseVcard", "Choose vCard file")}
            onChange={handleImport}
          />
          <Button
            disabled={isImporting}
            icon={<Upload size={14} />}
            onClick={() => fileInputRef.current?.click()}
          >
            {isImporting
              ? t("contacts.importingVcard", "Importing…")
              : t("contacts.importVcard", "Import vCard")}
          </Button>
          <Button
            disabled={isExporting}
            icon={<Download size={14} />}
            onClick={handleExport}
          >
            {isExporting
              ? t("contacts.exportingVcard", "Exporting…")
              : t("contacts.exportVcard", "Export vCard")}
          </Button>
          <Button variant="primary" icon={<Plus size={14} />} onClick={() => setEditorContact(null)}>
            {t("contacts.new", "New contact")}
          </Button>
        </div>
      </header>

      <div className="contacts-toolbar">
        <label className="contacts-search">
          <Search className="contacts-search__icon" size={14} aria-hidden="true" />
          <span className="sr-only">{t("contacts.search", "Search contacts")}</span>
          <input
            className="input contacts-search__input"
            aria-label={t("contacts.search", "Search contacts")}
            placeholder={t("contacts.search", "Search contacts")}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <Checkbox
          checked={favoriteOnly}
          onChange={(event) => setFavoriteOnly(event.target.checked)}
          label={t("contacts.favoritesOnly", "Favorites only")}
        />
      </div>

      {importResult && (
        <div className="contacts-import-summary" role="status">
          <div>
            <strong>{t("contacts.importSummary", "Import summary")}</strong>
            <p>
              {t("contacts.importCreated", { count: importResult.created, defaultValue: "{{count}} created" })}
              {" · "}
              {t("contacts.importMerged", { count: importResult.merged, defaultValue: "{{count}} merged" })}
              {" · "}
              {t("contacts.importSkipped", { count: importResult.skipped, defaultValue: "{{count}} skipped" })}
              {" · "}
              {t("contacts.importInvalid", { count: importResult.invalid, defaultValue: "{{count}} invalid" })}
            </p>
            {importResult.errors.length > 0 && (
              <ul>
                {importResult.errors.map((message, index) => <li key={`${index}-${message}`}>{message}</li>)}
              </ul>
            )}
          </div>
          <IconButton
            label={t("contacts.closeImportSummary", "Close import summary")}
            icon={<X size={14} />}
            onClick={() => setImportResult(null)}
          />
        </div>
      )}

      <div className="contacts-shell" data-has-selection={Boolean(selectedContact)}>
        <div ref={listPaneRef} className="contacts-list-pane">
          {isLoading && (
            <div className="contacts-state">{t("common.loading", "Loading...")}</div>
          )}
          {!isLoading && error && (
            <div className="contacts-state contacts-state--error">
              <p>{t("contacts.loadError", "Failed to load contacts")}</p>
              <Button onClick={() => refetch()}>
                {t("contacts.retry", "Retry")}
              </Button>
            </div>
          )}
          {!isLoading && !error && contacts.length === 0 && (
            <div className="contacts-empty-state">
              <span className="contacts-empty-mark" aria-hidden="true"><Mail size={22} /></span>
              <h2>{t("contacts.empty", "No contacts yet")}</h2>
              <p>{t("contacts.emptyHint", "Save the people you email most for faster addressing.")}</p>
              <Button icon={<Plus size={14} />} onClick={() => setEditorContact(null)}>
                {t("contacts.new", "New contact")}
              </Button>
            </div>
          )}
          {!isLoading && !error && contacts.length > 0 && (
            <div
              className="contact-list"
              role="list"
              style={{ height: contactListVirtualizer.getTotalSize() }}
            >
              {contactListVirtualizer.getVirtualItems().map((virtualRow) => {
                const contact = contacts[virtualRow.index];
                return (
                  <div
                    ref={contactListVirtualizer.measureElement}
                    role="listitem"
                    key={virtualRow.key}
                    data-index={virtualRow.index}
                    className="contact-list__row"
                    style={{ transform: `translateY(${virtualRow.start}px)` }}
                  >
                    <ContactListItem
                      contact={contact}
                      selected={selectedId === contact.id}
                      onSelect={() => setSelectedId(contact.id)}
                    />
                  </div>
                );
              })}
            </div>
          )}
        </div>

        <div className="contacts-detail-pane">
          {selectedContact ? (
            <ContactDetail
              contact={selectedContact}
              onBack={() => setSelectedId(null)}
              onWrite={() => writeToContact(selectedContact)}
              onEdit={() => setEditorContact(selectedContact)}
              onFavorite={() => handleFavorite(selectedContact)}
              onDelete={() => handleDelete(selectedContact)}
              onCopy={copyAddress}
            />
          ) : (
            <div className="contacts-selection-state">
              <Mail size={24} aria-hidden="true" />
              <p>{t("contacts.selectPrompt", "Select a contact to see details")}</p>
            </div>
          )}
        </div>
      </div>

      {editorContact !== undefined && (
        <ContactEditorDialog
          contact={editorContact}
          onClose={() => setEditorContact(undefined)}
          onSave={handleSave}
        />
      )}
    </section>
  );
}

interface ContactDetailProps {
  contact: Contact;
  onBack: () => void;
  onWrite: () => void;
  onEdit: () => void;
  onFavorite: () => void;
  onDelete: () => void;
  onCopy: (address: string) => void;
}

function ContactDetail({
  contact,
  onBack,
  onWrite,
  onEdit,
  onFavorite,
  onDelete,
  onCopy,
}: ContactDetailProps) {
  const { t } = useTranslation();
  const primary = primaryEmailFor(contact);
  const name = contact.display_name.trim() || primary?.address || t("contacts.unknown", "Unknown contact");

  return (
    <article className="contact-detail">
      <Button
        variant="ghost"
        size="sm"
        className="contacts-mobile-back"
        onClick={onBack}
        icon={<ArrowLeft size={14} />}
      >
        {t("contacts.back", "Back to contacts")}
      </Button>

      <div className="contact-detail-heading">
        <span className="contact-detail-avatar" aria-hidden="true">
          {name.split(/[\s@._-]+/).filter(Boolean).slice(0, 2).map((part) => part[0]?.toUpperCase()).join("")}
        </span>
        <div>
          <div className="contact-detail-name-row">
            <h2>{name}</h2>
            {contact.is_favorite && <Star className="contact-favorite-star" size={16} fill="currentColor" aria-hidden="true" />}
          </div>
          {primary && <p>{primary.address}</p>}
        </div>
      </div>

      <div className="contact-detail-actions">
        <Button icon={<Mail size={14} />} onClick={onWrite}>
          {t("contacts.write", "Write email")}
        </Button>
        <Button
          icon={<Star className={contact.is_favorite ? "contact-favorite-star" : undefined} size={14} fill={contact.is_favorite ? "currentColor" : "none"} />}
          onClick={onFavorite}
        >
          {contact.is_favorite
            ? t("contacts.favoriteRemove", "Remove from favorites")
            : t("contacts.favoriteAdd", "Add to favorites")}
        </Button>
        <Button icon={<Pencil size={14} />} onClick={onEdit}>
          {t("contacts.editAction", "Edit contact")}
        </Button>
      </div>

      <section className="contact-detail-section">
        <h3>{t("contacts.emailAddress", "Email address")}</h3>
        <div className="contact-address-list">
          {contact.emails.map((email) => (
            <div className="contact-address-row" key={email.id}>
              <div>
                <span className="contact-address-value">{email.address}</span>
                <span className="contact-address-meta">
                  {t(`contacts.${email.label}`, email.label)}
                  {email.is_primary ? ` · ${t("contacts.primary", "Primary")}` : ""}
                </span>
              </div>
              <IconButton
                label={t("contacts.copyNamed", { address: email.address, defaultValue: "Copy email address {{address}}" })}
                icon={<Clipboard size={14} />}
                onClick={() => onCopy(email.address)}
              />
            </div>
          ))}
        </div>
      </section>

      {contact.notes && (
        <section className="contact-detail-section">
          <h3>{t("contacts.notes", "Notes")}</h3>
          <p className="contact-notes">{contact.notes}</p>
        </section>
      )}

      <div className="contact-detail-danger-zone">
        <Button variant="danger-ghost" icon={<Trash2 size={14} />} onClick={onDelete}>
          {t("contacts.delete", "Delete contact")}
        </Button>
      </div>
    </article>
  );
}
