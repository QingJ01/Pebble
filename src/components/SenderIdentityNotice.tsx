import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAccountsQuery } from "@/hooks/queries";
import { profileLocalStorage } from "@/lib/profileStorage";
import { senderIdentityLabel } from "@/lib/accountIdentity";
import { hasModalOverlay } from "@/lib/overlayStack";
import { useUIStore } from "@/stores/ui.store";
import FolderModal from "@/components/FolderModal";
import Button from "@/components/ui/Button";

const NOTICE_KEY = "pebble-sender-identity-notice-v1";

/** Quiet period (no pointer/key input) required before the notice may open. */
export const SENDER_NOTICE_QUIET_MS = 2500;
/** If the user never interacts, open after this long anyway. */
export const SENDER_NOTICE_MAX_WAIT_MS = 8000;
const POLL_MS = 500;

/**
 * Wait until startup has settled: the user has finished their first burst of
 * interaction (or has not touched the app for a while), no modal is open and
 * the window is visible. Returns true once, then stays true.
 */
function useStartupSettled(enabled: boolean) {
  const [settled, setSettled] = useState(false);
  const startRef = useRef(Date.now());

  useEffect(() => {
    if (!enabled || settled) return;
    let lastActivity = Date.now();
    let interacted = false;
    const onActivity = () => {
      interacted = true;
      lastActivity = Date.now();
    };
    const check = () => {
      const now = Date.now();
      const quiet = now - lastActivity >= SENDER_NOTICE_QUIET_MS;
      const waitedEnough = interacted || now - startRef.current >= SENDER_NOTICE_MAX_WAIT_MS;
      const visible = typeof document === "undefined" || document.visibilityState !== "hidden";
      if (quiet && waitedEnough && visible && !hasModalOverlay()) setSettled(true);
    };
    window.addEventListener("pointerdown", onActivity, true);
    window.addEventListener("keydown", onActivity, true);
    window.addEventListener("wheel", onActivity, { capture: true, passive: true });
    const timer = window.setInterval(check, POLL_MS);
    return () => {
      window.removeEventListener("pointerdown", onActivity, true);
      window.removeEventListener("keydown", onActivity, true);
      window.removeEventListener("wheel", onActivity, true);
      window.clearInterval(timer);
    };
  }, [enabled, settled]);

  return settled;
}

/**
 * One-time notice that sender names are now written into outgoing mail.
 * Shown once per profile, after startup has settled (never on top of the
 * user's first interaction). "Got it" is the primary action and has focus.
 */
export default function SenderIdentityNotice({ defer = true }: { defer?: boolean }) {
  const { data: accounts = [] } = useAccountsQuery();
  const { t } = useTranslation();
  const [dismissed, setDismissed] = useState(() => profileLocalStorage.getItem(NOTICE_KEY) === "seen");
  const [open, setOpen] = useState(true);
  const dismissRef = useRef<HTMLButtonElement>(null);
  const senders = accounts.filter((account) => account.provider !== "outlook" && account.display_name.trim());
  const eligible = !dismissed && senders.length > 0;
  const settled = useStartupSettled(eligible && defer);

  if (!eligible || (defer && !settled)) return null;

  function dismiss() {
    profileLocalStorage.setItem(NOTICE_KEY, "seen");
    setOpen(false);
    // Let the exit animation play before unmounting.
    window.setTimeout(() => setDismissed(true), 150);
  }

  return <FolderModal
    open={open}
    title={t("accountSetup.senderNoticeTitle", "Review your sender names")}
    busy={false}
    onClose={dismiss}
    initialFocus={dismissRef}
  >
    <p className="folder-modal-lead">{t("accountSetup.senderNotice", "Sender names now appear in outgoing mail. Review these names, and use Account label for private account descriptions.")}</p>
    <section aria-labelledby="sender-identity-preview-label" style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
      <h3 id="sender-identity-preview-label" className="dialog-section-label">
        {t("accountSetup.senderPreviewLabel", "Outgoing mail will show")}
      </h3>
      <ul
        style={{
          margin: 0,
          maxHeight: "120px",
          overflowY: "auto",
          overflowWrap: "anywhere",
          padding: "8px 10px 8px 26px",
          borderRadius: "var(--radius-md)",
          background: "var(--color-bg-hover)",
          fontSize: "var(--font-base)",
          lineHeight: 1.6,
        }}
      >
        {senders.map((account) => <li key={account.id}>{senderIdentityLabel(account)}</li>)}
      </ul>
    </section>
    <div className="folder-modal-actions">
      <Button variant="secondary" onClick={() => {
        dismiss();
        useUIStore.getState().setSettingsTab("accounts");
        useUIStore.getState().setActiveView("settings");
      }}>{t("accountSetup.reviewNames", "Review in account settings")}</Button>
      <Button ref={dismissRef} variant="primary" onClick={dismiss}>{t("accountSetup.senderNoticeDismiss", "Got it")}</Button>
    </div>
  </FolderModal>;
}
