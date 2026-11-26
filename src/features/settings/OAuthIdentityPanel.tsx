import { useState } from "react";
import { useTranslation } from "react-i18next";
import { applyOAuthIdentity, previewOAuthIdentity } from "@/lib/api";
import type { OAuthIdentityPreview } from "@/lib/api";
import type { Account } from "@/lib/ipc-types";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import Button from "@/components/ui/Button";
import { SettingsNotice } from "./SettingsLayout";

export default function OAuthIdentityPanel({ account, onApplied }: {
  account: Account;
  onApplied: (account: Account) => void;
}) {
  const { t } = useTranslation();
  const [preview, setPreview] = useState<OAuthIdentityPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [applied, setApplied] = useState(false);

  async function verify() {
    setBusy(true); setError(null); setPreview(null); setApplied(false);
    try { setPreview(await previewOAuthIdentity(account.id)); }
    catch (err) { setError(extractErrorMessage(err)); }
    finally { setBusy(false); }
  }
  async function apply() {
    if (!preview) return;
    setBusy(true); setError(null);
    try {
      onApplied(await applyOAuthIdentity(preview));
      setPreview(null); setApplied(true);
    } catch (err) { setError(extractErrorMessage(err)); }
    finally { setBusy(false); }
  }

  return <div className="field" style={{ gridColumn: "1 / -1" }}>
    <p className="field__hint">{t("accountSetup.oauthAddressHelp", "The mailbox address comes from your provider. Verify it here if it shows a different login address.")}</p>
    <div>
      <Button size="sm" disabled={busy} onClick={() => void verify()}>
        {t("accountSetup.verifyIdentity", "Verify mailbox")}
      </Button>
    </div>
    {preview && <SettingsNotice tone="info" role="status" style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", gap: "var(--space-1)" }}>
      <div>{preview.previous_email} → {preview.identity.email}</div>
      {preview.identity.display_name && <div>{preview.identity.display_name}</div>}
      <p style={{ margin: 0, color: "var(--color-text-secondary)" }}>{t("accountSetup.identityRepairHelp", "This updates the mailbox details while keeping your existing mail and account label.")}</p>
      <Button size="sm" disabled={busy} onClick={() => void apply()} style={{ marginTop: "var(--space-1)" }}>{t("accountSetup.applyIdentity", "Use verified mailbox details")}</Button>
    </SettingsNotice>}
    {applied && <p role="status" className="field__hint" style={{ color: "var(--color-success)" }}>{t("accountSetup.identityApplied", "Mailbox details updated.")}</p>}
    {error && <p role="alert" className="field__error">{error}</p>}
  </div>;
}
