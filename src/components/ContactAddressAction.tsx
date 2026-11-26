import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ContactRound, UserPlus } from "lucide-react";
import { useTranslation } from "react-i18next";
import { type ContactInput } from "@/lib/api";
import { useAccountsQuery } from "@/hooks/queries";
import {
  contactByAddressQueryKey,
  useContactByAddressQuery,
} from "@/hooks/queries/useContactsQuery";
import { useContactMutations } from "@/hooks/mutations";
import { useToastStore } from "@/stores/toast.store";
import { useUIStore } from "@/stores/ui.store";
import ContactEditorDialog from "@/features/contacts/ContactEditorDialog";

interface ContactAddressActionProps {
  accountId: string;
  name?: string | null;
  address: string;
}

export default function ContactAddressAction({
  accountId,
  name,
  address,
}: ContactAddressActionProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const normalizedAddress = address.trim().toLowerCase();
  const { data: accounts = [] } = useAccountsQuery();
  const { save } = useContactMutations();
  const [editorOpen, setEditorOpen] = useState(false);
  const addToast = useToastStore((state) => state.addToast);

  const isSelf = useMemo(() => accounts.some((account) => (
    account.id === accountId
    && account.email.trim().toLowerCase() === normalizedAddress
  )), [accountId, accounts, normalizedAddress]);
  const { data: contact = null, isLoading: loading } = useContactByAddressQuery(
    normalizedAddress,
    !isSelf,
  );

  if (isSelf || !normalizedAddress) return null;

  const contactName = contact?.display_name.trim()
    || contact?.emails[0]?.address
    || name?.trim()
    || address;
  const addLabel = t("contacts.addAddress", { address, defaultValue: "Add {{address}} to contacts" });
  const viewLabel = t("contacts.viewInContacts", { name: contactName, defaultValue: "View {{name}} in contacts" });

  const handleSave = async (input: ContactInput) => {
    const saved = await save.mutateAsync(input);
    const retainsOriginalAddress = saved.emails.some((email) => (
      email.address.trim().toLowerCase() === normalizedAddress
    ));
    queryClient.setQueryData(contactByAddressQueryKey(normalizedAddress), retainsOriginalAddress ? saved : null);
    for (const email of saved.emails) {
      queryClient.setQueryData(contactByAddressQueryKey(email.address), saved);
    }
    setEditorOpen(false);
    addToast({ message: t("contacts.saveSuccess", "Contact saved"), type: "success" });
  };

  return (
    <>
      <button
        type="button"
        className="contact-address-action"
        aria-label={contact ? viewLabel : addLabel}
        title={contact ? viewLabel : addLabel}
        disabled={loading}
        onClick={() => {
          if (contact) {
            useUIStore.getState().openContactInContacts(contact.id);
          } else {
            setEditorOpen(true);
          }
        }}
      >
        {contact ? <ContactRound size={13} /> : <UserPlus size={13} />}
      </button>

      {editorOpen && (
        <ContactEditorDialog
          contact={null}
          initialValue={{ displayName: name, address }}
          onClose={() => setEditorOpen(false)}
          onSave={handleSave}
        />
      )}
    </>
  );
}
