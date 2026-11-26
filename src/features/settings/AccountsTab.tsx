import { accountLabel } from "@/lib/accountIdentity";
import { parseProxyPort, PROXY_PORT_ERROR_KEY } from "@/lib/proxy-port";
import OAuthIdentityPanel from "./OAuthIdentityPanel";
import { useEffect, useMemo, useRef, useState } from "react";
import { Plus, Trash2, Mail, Pencil, Plug, RefreshCw, X } from "lucide-react";
import Dialog from "@/components/ui/Dialog";
import Button from "@/components/ui/Button";
import IconButton from "@/components/ui/IconButton";
import ConfirmDialog from "@/components/ConfirmDialog";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useQueryClient } from "@tanstack/react-query";
import {
  deleteAccount,
  getAccountSyncRange,
  getImapSyncFolders,
  getOAuthAccountProxySetting,
  testAccountConnection,
  triggerSync,
  updateAccount,
  updateAccountSyncRange,
  updateImapSyncFolders,
  updateOAuthAccountProxySetting,
} from "@/lib/api";
import type {
  Account,
  AccountProxyMode,
  ConnectionSecurity,
  ImapSyncFolderSettings,
} from "@/lib/api";
import { useAccountsQuery, accountsQueryKey } from "@/hooks/queries";
import { useMailStore } from "@/stores/mail.store";
import { realtimePreferenceToPollInterval, useUIStore, type RealtimeStatus } from "@/stores/ui.store";
import { toastError, useToastStore } from "@/stores/toast.store";
import AccountSetup from "@/components/AccountSetup";
import SyncRangeFields, { syncRangeDays, syncRangeDraft } from "@/components/SyncRangeFields";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import { getSignature, setSignature } from "@/lib/signatures";
import { ACCOUNT_COLOR_PRESETS, assignAccountColors, getAccountColor } from "@/lib/accountColors";
import { SettingsList, SettingsListRow, SettingsNotice, SettingsSection, StatusChip } from "./SettingsLayout";
import { providerLabel } from "./providerLabel";

export default function AccountsTab() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data: accounts = [] } = useAccountsQuery();
  const accountColorsById = useMemo(() => assignAccountColors(accounts), [accounts]);
  const realtimeStatusByAccount = useUIStore((state) => state.realtimeStatusByAccount);
  const [showSetup, setShowSetup] = useState(false);
  const [editingAccount, setEditingAccount] = useState<Account | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<{ id: string; email: string } | null>(null);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<{ id: string; ok: boolean; message: string } | null>(null);

  async function doTestConnection(accountId: string) {
    setTestingId(accountId);
    setTestResult(null);
    try {
      const report = await testAccountConnection(accountId);
      setTestResult({ id: accountId, ok: true, message: report });
    } catch (err) {
      const msg = extractErrorMessage(err);
      setTestResult({ id: accountId, ok: false, message: msg });
    } finally {
      setTestingId(null);
    }
  }

  async function doDelete(accountId: string) {
    try {
      await deleteAccount(accountId);
      if (useMailStore.getState().activeAccountId === accountId) {
        useMailStore.getState().setActiveAccountId(null);
      }
      await queryClient.invalidateQueries({ queryKey: accountsQueryKey });
      useToastStore.getState().addToast({
        message: t("settings.deleteAccountSuccess", "Account removed"),
        type: "success",
      });
    } catch (err) {
      toastError(err, "settings.deleteAccountFailed");
    }
  }

  return (
    <div>
      <SettingsSection
        title={t("settings.emailAccounts")}
        actions={
          <Button size="sm" variant="primary" icon={<Plus size={14} />} onClick={() => setShowSetup(true)}>
            {t("settings.addAccount")}
          </Button>
        }
      >
        {accounts.length === 0 ? (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: "var(--space-3)",
              padding: "var(--space-8) 0",
              color: "var(--color-text-tertiary)",
            }}
          >
            <Mail size={40} strokeWidth={1.5} aria-hidden="true" />
            <p className="settings-empty">{t("settings.noAccounts")}</p>
            <Button onClick={() => setShowSetup(true)}>{t("settings.addFirstAccount")}</Button>
          </div>
        ) : (
          <SettingsList>
            {accounts.map((account) => {
              const realtimeStatus = realtimeStatusByAccount[account.id];
              const realtimeLabel = getAccountRealtimeStatusText(realtimeStatus, t);
              const accountColor = accountColorsById.get(account.id) ?? getAccountColor(account);

              return (
                <SettingsListRow
                  key={account.id}
                  style={{ alignItems: "flex-start" }}
                  leading={
                    <span
                      aria-hidden="true"
                      style={{
                        width: "8px",
                        height: "8px",
                        marginTop: "var(--space-1)",
                        borderRadius: "var(--radius-full)",
                        backgroundColor: accountColor,
                        flexShrink: 0,
                      }}
                    />
                  }
                  title={accountLabel(account)}
                  meta={account.email}
                  actions={
                    <>
                      <IconButton
                        label={t("accountSetup.testConnection", "Test Connection")}
                        icon={<Plug size={14} />}
                        loading={testingId === account.id}
                        onClick={() => doTestConnection(account.id)}
                      />
                      <IconButton
                        label={t("settings.editAccount", "Edit account")}
                        icon={<Pencil size={14} />}
                        onClick={() => setEditingAccount(account)}
                      />
                      <IconButton
                        variant="danger"
                        label={t("settings.removeAccount")}
                        icon={<Trash2 size={14} />}
                        onClick={() => setDeleteTarget({ id: account.id, email: account.email })}
                      />
                    </>
                  }
                >
                  <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: "var(--space-2)", marginTop: "var(--space-1)" }}>
                    <StatusChip>{providerLabel(account.provider)}</StatusChip>
                    {realtimeLabel && (
                      <span
                        aria-label={realtimeLabel}
                        title={realtimeStatus?.message ?? realtimeLabel}
                        style={{ fontSize: "var(--font-xs)", color: "var(--color-text-tertiary)" }}
                      >
                        {realtimeLabel}
                      </span>
                    )}
                  </div>
                </SettingsListRow>
              );
            })}
          </SettingsList>
        )}

        {testResult && (
          <SettingsNotice
            tone={testResult.ok ? "success" : "danger"}
            style={{ whiteSpace: "pre-wrap", fontFamily: "var(--font-mono)" }}
          >
            {testResult.message}
          </SettingsNotice>
        )}
      </SettingsSection>

      {/* Delete confirmation */}
      {deleteTarget && (
        <ConfirmDialog
          title={t("settings.removeAccount", "Remove Account")}
          message={t("settings.confirmRemoveAccountDetail", {
            email: deleteTarget.email,
            defaultValue: "Remove {{email}} from Pebble? Local mail, attachments and changes that haven't synced yet are deleted from this device. Mail on the server is not affected.",
          })}
          confirmLabel={t("settings.removeAccount", "Remove account")}
          destructive
          onCancel={() => setDeleteTarget(null)}
          onConfirm={() => {
            doDelete(deleteTarget.id);
            setDeleteTarget(null);
          }}
        />
      )}

      {/* AccountSetup modal */}
      {showSetup && <AccountSetup onClose={() => setShowSetup(false)} />}

      {/* Edit account modal */}
      {editingAccount && (
        <EditAccountModal
          account={editingAccount}
          initialColor={accountColorsById.get(editingAccount.id) ?? getAccountColor(editingAccount)}
          onClose={() => setEditingAccount(null)}
          onSaved={async () => {
            setEditingAccount(null);
            await queryClient.invalidateQueries({ queryKey: accountsQueryKey });
          }}
        />
      )}
    </div>
  );
}

function getAccountRealtimeStatusText(
  status: RealtimeStatus | undefined,
  t: TFunction,
) {
  if (!status) return null;

  if (status.message) {
    return status.message;
  }

  switch (status.mode) {
    case "realtime":
      return t("status.realtimeConnected", "Realtime connected");
    case "polling":
      return t("status.realtimePolling", "Polling");
    case "manual":
      return t("status.realtimeManual", "Manual only");
    case "backoff":
      return t("status.realtimeBackoff", "Retrying");
    case "auth_required":
      return t("status.realtimeAuthRequired", "Reconnect required");
    case "offline":
      return t("status.offline", "Offline");
    case "error":
      return t("status.realtimeError", "Realtime error");
  }
}

function EditAccountModal({ account, initialColor, onClose, onSaved }: {
  account: Account;
  initialColor: string;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const emailInputRef = useRef<HTMLInputElement>(null);
  const [displayName, setDisplayName] = useState(account.display_name);
  const [accountLabelValue, setAccountLabelValue] = useState(account.account_label || "");
  const [providerName, setProviderName] = useState(account.provider_display_name || "");
  const [email, setEmail] = useState(account.email);
  const [accountColor, setAccountColor] = useState(initialColor);
  const [password, setPassword] = useState("");
  const [imapHost, setImapHost] = useState("");
  const [imapPort, setImapPort] = useState("");
  const [smtpHost, setSmtpHost] = useState("");
  const [smtpPort, setSmtpPort] = useState("");
  const [imapSecurity, setImapSecurity] = useState<ConnectionSecurity | "">("");
  const [smtpSecurity, setSmtpSecurity] = useState<ConnectionSecurity | "">("");
  const [acceptInvalidCerts, setAcceptInvalidCerts] = useState<"" | "false" | "true">("");
  const [oauthProxyMode, setOauthProxyMode] = useState<AccountProxyMode>("inherit");
  const [proxyHost, setProxyHost] = useState("");
  const [proxyPort, setProxyPort] = useState("");
  const [signature, setSignatureValue] = useState("");
  const [syncRange, setSyncRange] = useState(() => syncRangeDraft(null));
  const [savedSyncRange, setSavedSyncRange] = useState<number | null>(null);
  const [rangeLoading, setRangeLoading] = useState(account.provider !== "pop3");
  const [rangeError, setRangeError] = useState<string | null>(null);
  const realtimeMode = useUIStore((state) => state.realtimeMode);
  const [imapFolderSettings, setImapFolderSettings] = useState<ImapSyncFolderSettings | null>(null);
  const [selectedImapFolderIds, setSelectedImapFolderIds] = useState<Set<string>>(new Set());
  const [folderLoading, setFolderLoading] = useState(false);
  const [folderError, setFolderError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isOAuth = account.provider === "gmail" || account.provider === "outlook";
  const isImap = account.provider === "imap";
  const isPop3 = account.provider === "pop3";
  const incomingHostLabel = isPop3
    ? t("accountSetup.pop3Host", "POP3 host")
    : t("accountSetup.imapHost", "IMAP host");
  const incomingPortLabel = isPop3
    ? t("accountSetup.pop3Port", "POP3 port")
    : t("accountSetup.imapPort", "IMAP port");

  useEffect(() => {
    if (isPop3) return;
    let cancelled = false;
    getAccountSyncRange(account.id)
      .then((days) => {
        if (cancelled) return;
        setSavedSyncRange(days);
        setSyncRange(syncRangeDraft(days));
      })
      .catch((err) => {
        if (!cancelled) setRangeError(extractErrorMessage(err));
      })
      .finally(() => {
        if (!cancelled) setRangeLoading(false);
      });
    return () => { cancelled = true; };
  }, [account.id, isPop3]);

  useEffect(() => {
    let cancelled = false;
    getSignature(account.id)
      .then((loaded) => {
        if (!cancelled) setSignatureValue(loaded);
      })
      .catch((err) => {
        console.warn("Failed to load signature:", err);
      });
    return () => {
      cancelled = true;
    };
  }, [account.id]);

  useEffect(() => {
    if (!isOAuth) return;
    let cancelled = false;
    getOAuthAccountProxySetting(account.id)
      .then((setting) => {
        if (cancelled) return;
        setOauthProxyMode(setting.mode);
        setProxyHost(setting.proxy?.host ?? "");
        setProxyPort(setting.proxy?.port ? String(setting.proxy.port) : "");
      })
      .catch((err) => {
        console.warn("Failed to load OAuth proxy:", err);
      });
    return () => {
      cancelled = true;
    };
  }, [account.id, isOAuth]);


  async function loadImapFolders() {
    setFolderLoading(true);
    setFolderError(null);
    try {
      const settings = await getImapSyncFolders(account.id);
      setImapFolderSettings(settings);
      setSelectedImapFolderIds(new Set(settings.selected_remote_ids));
    } catch (err) {
      setFolderError(extractErrorMessage(err));
    } finally {
      setFolderLoading(false);
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const days = isPop3 ? null : syncRangeDays(syncRange);
    if (rangeLoading || rangeError || days === undefined) return;
    setLoading(true);
    setError(null);
    try {
      const parsedProxyPort = parseProxyPort(proxyPort, t(PROXY_PORT_ERROR_KEY, "Port must be an integer between 1 and 65535."));
      if (isOAuth) {
        await updateAccount(
          account.id,
          email,
          displayName,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          accountColor,
          accountLabelValue,
        );
        const trimmedProxyHost = proxyHost.trim();
        const trimmedProxyPort = proxyPort.trim();
        const hasCustomProxyDraft = !!trimmedProxyHost || !!trimmedProxyPort;
        const nextProxyMode: AccountProxyMode = hasCustomProxyDraft
          ? "custom"
          : oauthProxyMode === "disabled"
            ? "disabled"
            : "inherit";
        await updateOAuthAccountProxySetting(
          account.id,
          nextProxyMode,
          nextProxyMode === "custom" ? trimmedProxyHost || undefined : undefined,
          nextProxyMode === "custom" ? parsedProxyPort : undefined,
        );
      } else {
        await updateAccount(
          account.id,
          email,
          displayName,
          password || undefined,
          imapHost || undefined,
          imapPort ? parseInt(imapPort, 10) : undefined,
          smtpHost || undefined,
          smtpPort ? parseInt(smtpPort, 10) : undefined,
          imapSecurity || undefined,
          smtpSecurity || undefined,
          acceptInvalidCerts === "" ? undefined : acceptInvalidCerts === "true",
          proxyHost.trim() || undefined,
          parsedProxyPort,
          accountColor,
          accountLabelValue,
        );
      }

      if (isImap && imapFolderSettings) {
        const updatedSettings = await updateImapSyncFolders(
          account.id,
          Array.from(selectedImapFolderIds),
        );
        setImapFolderSettings(updatedSettings);
        setSelectedImapFolderIds(new Set(updatedSettings.selected_remote_ids));
        try {
          await triggerSync(account.id, "manual");
        } catch (err) {
          console.warn("Failed to trigger sync after updating IMAP folders:", err);
        }
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: ["folders", account.id] }),
          queryClient.invalidateQueries({ queryKey: ["messages"] }),
          queryClient.invalidateQueries({ queryKey: ["threads"] }),
          queryClient.invalidateQueries({ queryKey: ["folder-unread-counts", account.id] }),
          queryClient.invalidateQueries({ queryKey: ["starred-messages", account.id] }),
        ]);
      }

      if (!isPop3 && days !== savedSyncRange) {
        await updateAccountSyncRange(account.id, days, realtimePreferenceToPollInterval(realtimeMode));
        setSavedSyncRange(days);
      }
      await setSignature(account.id, signature);
      onSaved();
    } catch (err) {
      setError(extractErrorMessage(err));
    } finally {
      setLoading(false);
    }
  }

  const optionalStyle: React.CSSProperties = {
    color: "var(--color-text-tertiary)",
    fontWeight: "var(--weight-regular)" as React.CSSProperties["fontWeight"],
  };
  const colorInputValue = /^#[0-9a-fA-F]{6}$/.test(accountColor) ? accountColor : initialColor;
  const proxyFields = (
    <div className="settings-grid" style={{ gridTemplateColumns: "1fr auto" }}>
      <div className="field">
        <label className="field__label">{t("accountSetup.proxyHost", "SOCKS5 Proxy")} <span style={optionalStyle}>({t("settings.optional", "optional")})</span></label>
        <input aria-label={t("accountSetup.proxyHost", "SOCKS5 Proxy")} className="input" type="text" value={proxyHost} onChange={(e) => setProxyHost(e.target.value)} placeholder="127.0.0.1" />
      </div>
      <div className="field">
        <label className="field__label">{t("accountSetup.proxyPort", "Port")}</label>
        <input aria-label={t("accountSetup.proxyPort", "Port")} className="input" style={{ width: "80px" }} type="number" inputMode="numeric" value={proxyPort} onChange={(e) => setProxyPort(e.target.value)} placeholder="7890" />
      </div>
    </div>
  );

  return (
    <Dialog
      onClose={onClose}
      dismissible={!loading}
      closeOnBackdrop={false}
      labelledBy="edit-account-title"
      initialFocus={emailInputRef}
      busy={loading}
      panelClassName="dialog-panel dialog-panel--flush"
      style={{ width: "min(480px, 100%)", maxHeight: "90vh", overflow: "hidden" }}
    >
        <div className="dialog-panel__header">
          <h2 id="edit-account-title" className="dialog-title">
            {t("settings.editAccount", "Edit Account")}
          </h2>
          <IconButton label={t("common.close", "Close")} icon={<X size={16} />} onClick={onClose} />
        </div>

        <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", flex: "1 1 auto", minHeight: 0 }}>
          <div className="scroll-region edit-account-scroll dialog-panel__body settings-form">
            <div className="field">
              <label className="field__label">{t("accountSetup.accountLabel", "Account label")}</label>
              <input aria-label={t("accountSetup.accountLabel", "Account label")} className="input" maxLength={120}
                value={accountLabelValue} onChange={(e) => setAccountLabelValue(e.target.value)} />
              <p className="field__hint">{t("accountSetup.accountLabelHelp", "Only used to identify this account in Pebble; never sent as your sender name.")}</p>
            </div>
            <div className="field">
              <label className="field__label">{t("accountSetup.displayName")}</label>
              <input aria-label={t("accountSetup.displayName")} className="input" type="text" readOnly={account.provider === "outlook"} value={account.provider === "outlook" ? providerName : displayName} onChange={(e) => setDisplayName(e.target.value)} />
              <p className="field__hint">{account.provider === "outlook"
                ? t("accountSetup.providerManagedName", "The sender name is managed by your email provider.")
                : t("accountSetup.senderNameHelp", "Used in outgoing messages. Check this name before sending.")}</p>
            </div>
            <div className="field">
              <label className="field__label">{t("accountSetup.emailAddress")}</label>
              <input aria-label={t("accountSetup.emailAddress")} ref={emailInputRef} className="input" type="email" required readOnly={isOAuth} value={email} onChange={(e) => setEmail(e.target.value)} />
            </div>
            {isOAuth && <OAuthIdentityPanel account={account} onApplied={(updated) => {
              setEmail(updated.email);
              setProviderName(updated.provider_display_name || "");
              void queryClient.invalidateQueries({ queryKey: accountsQueryKey });
            }} />}
            <div className="field">
              <label htmlFor="account-color" className="field__label">{t("settings.accountColor", "Account color")}</label>
              <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
                <input
                  id="account-color"
                  aria-label={t("settings.accountColor", "Account color")}
                  type="color"
                  value={colorInputValue}
                  onChange={(e) => setAccountColor(e.target.value)}
                  style={{
                    width: "40px",
                    height: "var(--control-md)",
                    padding: "2px",
                    border: "1px solid var(--color-border)",
                    borderRadius: "var(--radius-md)",
                    backgroundColor: "var(--color-control-bg)",
                    cursor: "pointer",
                  }}
                />
                <input
                  aria-label={t("settings.accountColorHex", "Account color hex")}
                  className="input" style={{ width: "96px", fontFamily: "var(--font-mono)" }}
                  value={accountColor}
                  onChange={(e) => {
                    const value = e.target.value;
                    setAccountColor(value.startsWith("#") ? value : `#${value}`);
                  }}
                  pattern="^#[0-9a-fA-F]{6}$"
                  maxLength={7}
                />
              </div>
              <div
                aria-label={t("settings.accountColorPresets", "Color presets")}
                role="group"
                style={{
                  display: "flex",
                  flexWrap: "wrap",
                  gap: "var(--space-2)",
                  marginTop: "var(--space-1)",
                }}
              >
                {ACCOUNT_COLOR_PRESETS.map((preset) => {
                  const presetLabel = t("settings.useAccountColorPreset", {
                    color: t(`settings.accountColorName.${preset.name.toLowerCase()}`, preset.name),
                    defaultValue: "Use {{color}}",
                  });
                  const selected = accountColor.toLowerCase() === preset.color;
                  return (
                    <button
                      key={preset.color}
                      type="button"
                      aria-label={presetLabel}
                      aria-pressed={selected}
                      title={presetLabel}
                      onClick={() => setAccountColor(preset.color)}
                      style={{
                        width: "24px",
                        height: "24px",
                        borderRadius: "var(--radius-full)",
                        border: selected ? "2px solid var(--color-text-primary)" : "1px solid var(--color-border)",
                        backgroundColor: preset.color,
                        cursor: "pointer",
                        padding: 0,
                      }}
                    />
                  );
                })}
              </div>
            </div>
            {isOAuth ? (
              <SettingsNotice tone="info">
                {t(
                  "settings.oauthAccountNote",
                  "This account uses OAuth. Provider sign-in, password, and IMAP/SMTP settings are managed by the provider. Leave the proxy blank to inherit the global SOCKS5 proxy."
                )}
              </SettingsNotice>
            ) : (
              <>
                <div className="field">
                  <label className="field__label">{t("accountSetup.password")} <span style={optionalStyle}>({t("settings.leaveEmptyKeep", "leave empty to keep current")})</span></label>
                  <input aria-label={t("accountSetup.password")} className="input" type="password" value={password} onChange={(e) => setPassword(e.target.value)} />
                </div>

                <div className="settings-grid" style={{ gridTemplateColumns: "1fr auto auto" }}>
                  <div className="field">
                    <label className="field__label">{incomingHostLabel} <span style={optionalStyle}>({t("settings.optional", "optional")})</span></label>
                    <input aria-label={incomingHostLabel} className="input" type="text" value={imapHost} onChange={(e) => setImapHost(e.target.value)} placeholder={t("settings.leaveEmptyKeep")} />
                  </div>
                  <div className="field">
                    <label className="field__label">{incomingPortLabel}</label>
                    <input aria-label={incomingPortLabel} className="input" style={{ width: "80px" }} type="number" min={1} max={65535} step={1} inputMode="numeric" value={imapPort} onChange={(e) => setImapPort(e.target.value)} />
                  </div>
                  <div className="field">
                    <label htmlFor="accountsetup-imap-security" className="field__label">{t("accountSetup.security", "Security")}</label>
                    <select id="accountsetup-imap-security" value={imapSecurity} onChange={(e) => setImapSecurity(e.target.value as ConnectionSecurity | "")} className="select" style={{ width: "120px" }}>
                      <option value="">{t("settings.leaveEmptyKeep", "keep current")}</option>
                      <option value="tls">{t("accountSetup.securityTls", "SSL/TLS")}</option>
                      <option value="starttls">{t("accountSetup.securityStarttls", "STARTTLS")}</option>
                      <option value="plain">{t("accountSetup.securityPlain", "None")}</option>
                    </select>
                  </div>
                </div>

                <div className="settings-grid" style={{ gridTemplateColumns: "1fr auto auto" }}>
                  <div className="field">
                    <label className="field__label">{t("accountSetup.smtpHost")} <span style={optionalStyle}>({t("settings.optional", "optional")})</span></label>
                    <input aria-label={t("accountSetup.smtpHost")} className="input" type="text" value={smtpHost} onChange={(e) => setSmtpHost(e.target.value)} placeholder={t("settings.leaveEmptyKeep")} />
                  </div>
                  <div className="field">
                    <label className="field__label">{t("accountSetup.smtpPort")}</label>
                    <input aria-label={t("accountSetup.smtpPort")} className="input" style={{ width: "80px" }} type="number" min={1} max={65535} step={1} inputMode="numeric" value={smtpPort} onChange={(e) => setSmtpPort(e.target.value)} />
                  </div>
                  <div className="field">
                    <label htmlFor="accountsetup-smtp-security" className="field__label">{t("accountSetup.security", "Security")}</label>
                    <select id="accountsetup-smtp-security" value={smtpSecurity} onChange={(e) => setSmtpSecurity(e.target.value as ConnectionSecurity | "")} className="select" style={{ width: "120px" }}>
                      <option value="">{t("settings.leaveEmptyKeep", "keep current")}</option>
                      <option value="tls">{t("accountSetup.securityTls", "SSL/TLS")}</option>
                      <option value="starttls">{t("accountSetup.securityStarttls", "STARTTLS")}</option>
                      <option value="plain">{t("accountSetup.securityPlain", "None")}</option>
                    </select>
                  </div>
                </div>

                <div className="field">
                  <label htmlFor="accountsetup-cert-policy" className="field__label">
                    {t("accountSetup.tlsCertificateVerification", "TLS certificate verification")}
                  </label>
                  <select
                    id="accountsetup-cert-policy"
                    value={acceptInvalidCerts}
                    onChange={(e) => setAcceptInvalidCerts(e.target.value as "" | "false" | "true")}
                    className="select"
                  >
                    <option value="">{t("settings.leaveEmptyKeep", "keep current")}</option>
                    <option value="false">{t("accountSetup.verifyTlsCerts", "Verify certificates")}</option>
                    <option value="true">{t("accountSetup.acceptInvalidCerts", "Allow invalid TLS certificates")}</option>
                  </select>
                </div>

              </>
            )}

            {proxyFields}

            <SyncRangeFields value={syncRange} onChange={setSyncRange}
              disabled={loading || rangeLoading || !!rangeError} unsupported={isPop3} />
            {rangeLoading && <div role="status" className="settings-hint">{t("common.loading", "Loading...")}</div>}
            {rangeError && <div role="alert" className="settings-error">
              {t("syncRange.loadError", "Could not load the sync range. Close and reopen this dialog to retry.")} {rangeError}
            </div>}

            {isImap && (
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: "var(--space-3)",
                  padding: "var(--space-3)",
                  border: "1px solid var(--color-border)",
                  borderRadius: "var(--radius-lg)",
                }}
              >
                <div style={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: "var(--space-3)" }}>
                  <div className="field">
                    <div className="field__label">
                      {t("settings.syncFolders", "Folders to sync")}
                    </div>
                    <div className="field__hint">
                      {t(
                        "settings.syncFoldersDesc",
                        "Inbox is always synced. Choose any additional IMAP folders to keep locally.",
                      )}
                    </div>
                  </div>
                  <Button
                    size="sm"
                    onClick={loadImapFolders}
                    disabled={folderLoading || loading}
                    icon={<RefreshCw size={14} />}
                    style={{ flexShrink: 0 }}
                  >
                    {folderLoading
                      ? t("settings.loadingFolders", "Loading...")
                      : imapFolderSettings
                        ? t("settings.refreshFolders", "Refresh folders")
                        : t("settings.chooseFolders", "Choose folders")}
                  </Button>
                </div>

                {folderError && (
                  <div role="alert" className="settings-error">
                    {t("settings.folderListFailed", "Failed to list IMAP folders: {{error}}", {
                      error: folderError,
                    })}
                  </div>
                )}

                {imapFolderSettings && (
                  <>
                    <div
                      role="group"
                      aria-label={t("settings.syncFolders", "Folders to sync")}
                      style={{
                        display: "flex",
                        flexDirection: "column",
                        gap: "var(--space-2)",
                        maxHeight: "180px",
                        overflowY: "auto",
                        padding: "var(--space-2) var(--space-3)",
                        borderRadius: "var(--radius-md)",
                        backgroundColor: "var(--color-surface-sunken)",
                      }}
                    >
                      {imapFolderSettings.folders.map((folder) => {
                        const isInboxFolder =
                          folder.role === "inbox" || folder.remote_id.toUpperCase() === "INBOX";
                        return (
                          <label
                            key={folder.remote_id}
                            className="choice"
                            style={{
                              display: "flex",
                              fontSize: "var(--font-sm)",
                              cursor: isInboxFolder ? "default" : "pointer",
                            }}
                          >
                            <input
                              type="checkbox"
                              className="checkbox"
                              aria-label={folder.name}
                              checked={isInboxFolder || selectedImapFolderIds.has(folder.remote_id)}
                              disabled={isInboxFolder}
                              onChange={(event) => {
                                setSelectedImapFolderIds((current) => {
                                  const next = new Set(current);
                                  if (event.target.checked) {
                                    next.add(folder.remote_id);
                                  } else {
                                    next.delete(folder.remote_id);
                                  }
                                  return next;
                                });
                              }}
                            />
                            <span>{folder.name}</span>
                            {isInboxFolder && (
                              <span style={{ color: "var(--color-text-tertiary)", marginLeft: "auto" }}>
                                {t("settings.alwaysSynced", "Always synced")}
                              </span>
                            )}
                          </label>
                        );
                      })}
                    </div>
                    <div className="settings-hint">
                      {t(
                        "settings.syncFoldersCleanupWarning",
                        "Saving removes locally stored data for folders you uncheck. It does not delete mail from the server.",
                      )}
                    </div>
                  </>
                )}
              </div>
            )}

            {/* Signature */}
            <div className="field">
              <label className="field__label">{t("settings.signature", "Signature")} <span style={optionalStyle}>({t("settings.optional", "optional")})</span></label>
              <textarea
                className="textarea"
                rows={4}
                value={signature}
                onChange={(e) => setSignatureValue(e.target.value)}
                placeholder={t("settings.signaturePlaceholder", "Your email signature...")}
              />
            </div>

            {error && (
              <div role="alert" aria-live="assertive" className="dialog-error">
                {error}
              </div>
            )}

          </div>
          <div className="dialog-panel__footer">
            <Button variant="secondary" disabled={loading} onClick={onClose}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              type="submit"
              variant="primary"
              loading={loading}
              disabled={folderLoading || rangeLoading || !!rangeError || (!isPop3 && syncRangeDays(syncRange) === undefined)}
            >
              {loading ? t("common.saving") : t("common.save")}
            </Button>
          </div>
        </form>
    </Dialog>
  );
}
