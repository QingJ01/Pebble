import { Star } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Contact } from "@/lib/api";

interface ContactListItemProps {
  contact: Contact;
  selected: boolean;
  onSelect: () => void;
}

function initialsFor(contact: Contact) {
  const source = contact.display_name.trim() || contact.emails[0]?.address || "?";
  const words = source.split(/[\s@._-]+/).filter(Boolean);
  return words.slice(0, 2).map((word) => word[0]?.toUpperCase()).join("") || "?";
}

export default function ContactListItem({ contact, selected, onSelect }: ContactListItemProps) {
  const { t } = useTranslation();
  const primaryEmail = contact.emails.find((email) => email.is_primary) ?? contact.emails[0];
  const title = contact.display_name.trim() || primaryEmail?.address || t("contacts.unknown", "Unknown contact");
  const address = primaryEmail?.address ?? "";

  return (
    <button
      type="button"
      className={`list-row contact-list-item${selected ? " is-selected" : ""}`}
      aria-label={`${title} ${address}`.trim()}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span className="contact-avatar" aria-hidden="true">{initialsFor(contact)}</span>
      <span className="contact-list-copy">
        <span className="contact-list-name">{title}</span>
        <span className="contact-list-email">{address}</span>
      </span>
      {contact.is_favorite && (
        <Star className="contact-list-star" size={14} fill="currentColor" aria-hidden="true" />
      )}
    </button>
  );
}
