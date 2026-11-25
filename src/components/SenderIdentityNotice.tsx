import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useAccountsQuery } from "@/hooks/queries";
import { profileLocalStorage } from "@/lib/profileStorage";
import { senderIdentityLabel } from "@/lib/accountIdentity";
import { useUIStore } from "@/stores/ui.store";
import FolderModal from "@/components/FolderModal";

const NOTICE_KEY = "pebble-sender-identity-notice-v1";

export default function SenderIdentityNotice() {
  const { data: accounts = [] } = useAccountsQuery();
  const { t } = useTranslation();
  const [dismissed, setDismissed] = useState(() => profileLocalStorage.getItem(NOTICE_KEY) === "seen");
  const senders = accounts.filter((account) => account.provider !== "outlook" && account.display_name.trim());
  if (dismissed || senders.length === 0) return null;

  function dismiss() {
    profileLocalStorage.setItem(NOTICE_KEY, "seen");
    setDismissed(true);
  }

  return <FolderModal
    title={t("accountSetup.senderNoticeTitle", "Review your sender names")}
    busy={false}
    onClose={dismiss}
  >
    <p>{t("accountSetup.senderNotice", "Sender names now appear in outgoing mail. Review these names, and use Account label for private account descriptions.")}</p>
    <details>
      <summary>{t("accountSetup.senderPreview", "Preview sender identities")}</summary>
      <ul style={{ maxHeight: "120px", overflowY: "auto", overflowWrap: "anywhere", paddingLeft: "18px" }}>
        {senders.map((account) => <li key={account.id}>{senderIdentityLabel(account)}</li>)}
      </ul>
    </details>
    <div className="folder-modal-actions">
      <button type="button" onClick={() => {
        dismiss();
        useUIStore.getState().setSettingsTab("accounts");
        useUIStore.getState().setActiveView("settings");
      }}>{t("accountSetup.reviewNames", "Review in account settings")}</button>
      <button type="button" onClick={dismiss}>{t("accountSetup.senderNoticeDismiss", "Got it")}</button>
    </div>
  </FolderModal>;
}
