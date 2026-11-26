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
import { inputStyle, labelStyle } from "../../styles/form";

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
      {/* Section header */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: "20px",
        }}
      >
        <h2
          style={{
            margin: 0,
            fontSize: "16px",
            fontWeight: 600,
            color: "var(--color-text-primary)",
          }}
        >
          {t("settings.emailAccounts")}
        </h2>
        <button
          onClick={() => setShowSetup(true)}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "6px",
            padding: "7px 14px",
            borderRadius: "6px",
            border: "none",
            backgroundColor: "var(--color-accent)",
            color: "#fff",
            fontSize: "13px",
            fontWeight: 600,
            cursor: "pointer",
          }}
        >
          <Plus size={14} />
          {t("settings.addAccount")}
        </button>
      </div>

      {/* Empty state */}
      {accounts.length === 0 ? (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "12px",
            padding: "48px 0",
            color: "var(--color-text-secondary)",
          }}
        >
          <Mail size={40} strokeWidth={1.5} />
          <p style={{ margin: 0, fontSize: "14px" }}>{t("settings.noAccounts")}</p>
          <button
            onClick={() => setShowSetup(true)}
            style={{
              marginTop: "4px",
              padding: "8px 18px",
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              backgroundColor: "transparent",
              color: "var(--color-text-primary)",
              fontSize: "13px",
              cursor: "pointer",
            }}
          >
            {t("settings.addFirstAccount")}
          </button>
        </div>
      ) : (
        /* Account list */
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "1px",
            borderRadius: "8px",
            overflow: "hidden",
            border: "1px solid var(--color-border)",
          }}
        >
          {accounts.map((account, index) => {
            const realtimeStatus = realtimeStatusByAccount[account.id];
            const realtimeLabel = getAccountRealtimeStatusText(realtimeStatus, t);
            const accountColor = accountColorsById.get(account.id) ?? getAccountColor(account);

            return (
              <div
                key={account.id}
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  padding: "14px 16px",
                  backgroundColor: "var(--color-bg)",
                  borderTop: index > 0 ? "1px solid var(--color-border)" : "none",
                }}
              >
                <div style={{ display: "flex", flexDirection: "column", gap: "2px" }}>
                  <div style={{ display: "flex", alignItems: "center", gap: "8px", minWidth: 0 }}>
                    <span
                      aria-hidden="true"
                      style={{
                        width: "8px",
                        height: "8px",
                        borderRadius: "50%",
                        backgroundColor: accountColor,
                        flexShrink: 0,
                      }}
                    />
                    <span style={{ fontSize: "13px", fontWeight: 500 }}>
                      {accountLabel(account)}
                    </span>
                  </div>
                  <span
                    style={{
                      fontSize: "12px",
                      color: "var(--color-text-secondary)",
                    }}
                  >
                    {account.email}
                  </span>
                  <span
                    style={{
                      fontSize: "11px",
                      color: "var(--color-text-secondary)",
                      textTransform: "capitalize",
                    }}
                  >
                    {account.provider}
                  </span>
                  {realtimeLabel && (
                    <span
                      aria-label={realtimeLabel}
                      title={realtimeStatus?.message ?? realtimeLabel}
                      style={{
                        fontSize: "11px",
                        color: "var(--color-text-secondary)",
                      }}
                    >
                      {realtimeLabel}
                    </span>
                  )}
                </div>
                <div style={{ display: "flex", gap: "4px", alignItems: "center" }}>
                  <IconButton
                    label={t("accountSetup.testConnection", "Test Connection")}
                    icon={<Plug size={15} />}
                    loading={testingId === account.id}
                    onClick={() => doTestConnection(account.id)}
                  />
                  <IconButton
                    label={t("settings.editAccount", "Edit account")}
                    icon={<Pencil size={15} />}
                    onClick={() => setEditingAccount(account)}
                  />
                  <IconButton
                    label={t("settings.removeAccount")}
                    icon={<Trash2 size={15} />}
                    onClick={() => setDeleteTarget({ id: account.id, email: account.email })}
                    style={{ marginLeft: "4px" }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Test result */}
      {testResult && (
        <div
          style={{
            marginTop: "10px",
            padding: "10px 12px",
            borderRadius: "6px",
            backgroundColor: testResult.ok ? "var(--color-success-bg)" : "var(--color-danger-bg)",
            border: "1px solid var(--color-border)",
            color: testResult.ok ? "var(--color-success)" : "var(--color-danger)",
            fontSize: "12px",
            whiteSpace: "pre-wrap",
            fontFamily: "monospace",
            lineHeight: 1.5,
          }}
        >
          {testResult.message}
        </div>
      )}

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

  const fieldStyle: React.CSSProperties = {
    display: "flex",
    flexDirection: "column",
  };
  const colorInputValue = /^#[0-9a-fA-F]{6}$/.test(accountColor) ? accountColor : initialColor;
  const proxyFields = (
    <div style={{ display: "grid", gridTemplateColumns: "1fr auto", gap: "12px" }}>
      <div style={fieldStyle}>
        <label style={labelStyle}>{t("accountSetup.proxyHost", "SOCKS5 Proxy")} <span style={{ color: "var(--color-text-secondary)", fontWeight: 400 }}>({t("settings.optional", "optional")})</span></label>
        <input aria-label={t("accountSetup.proxyHost", "SOCKS5 Proxy")} style={inputStyle} type="text" value={proxyHost} onChange={(e) => setProxyHost(e.target.value)} placeholder="127.0.0.1" />
      </div>
      <div style={fieldStyle}>
        <label style={labelStyle}>{t("accountSetup.proxyPort", "Port")}</label>
        <input aria-label={t("accountSetup.proxyPort", "Port")} style={{ ...inputStyle, width: "80px" }} type="number" inputMode="numeric" value={proxyPort} onChange={(e) => setProxyPort(e.target.value)} placeholder="7890" />
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
        <div
          className="dialog-header"
          style={{
            padding: "16px 20px",
            borderBottom: "1px solid var(--color-border)",
          }}
        >
          <h2 id="edit-account-title" className="dialog-title">
            {t("settings.editAccount", "Edit Account")}
          </h2>
          <IconButton label={t("common.close", "Close")} icon={<X size={16} />} onClick={onClose} />
        </div>

        <div className="scroll-region edit-account-scroll" style={{ overflowY: "auto", padding: "20px" }}>
          <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
            <div style={fieldStyle}>
              <label style={labelStyle}>{t("accountSetup.accountLabel", "Account label")}</label>
              <input aria-label={t("accountSetup.accountLabel", "Account label")} style={inputStyle} maxLength={120}
                value={accountLabelValue} onChange={(e) => setAccountLabelValue(e.target.value)} />
              <small>{t("accountSetup.accountLabelHelp", "Only used to identify this account in Pebble; never sent as your sender name.")}</small>
            </div>
            <div>
              <label style={labelStyle}>{t("accountSetup.displayName")}</label>
              <input aria-label={t("accountSetup.displayName")} style={inputStyle} type="text" readOnly={account.provider === "outlook"} value={account.provider === "outlook" ? providerName : displayName} onChange={(e) => setDisplayName(e.target.value)} />
              <small>{account.provider === "outlook"
                ? t("accountSetup.providerManagedName", "The sender name is managed by your email provider.")
                : t("accountSetup.senderNameHelp", "Used in outgoing messages. Check this name before sending.")}</small>
            </div>
            <div style={fieldStyle}>
              <label style={labelStyle}>{t("accountSetup.emailAddress")}</label>
              <input aria-label={t("accountSetup.emailAddress")} ref={emailInputRef} style={inputStyle} type="email" required readOnly={isOAuth} value={email} onChange={(e) => setEmail(e.target.value)} />
            </div>
            {isOAuth && <OAuthIdentityPanel account={account} onApplied={(updated) => {
              setEmail(updated.email);
              setProviderName(updated.provider_display_name || "");
              void queryClient.invalidateQueries({ queryKey: accountsQueryKey });
            }} />}
            <div style={fieldStyle}>
              <label htmlFor="account-color" style={labelStyle}>{t("settings.accountColor", "Account color")}</label>
              <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
                <input
                  id="account-color"
                  aria-label={t("settings.accountColor", "Account color")}
                  type="color"
                  value={colorInputValue}
                  onChange={(e) => setAccountColor(e.target.value)}
                  style={{
                    width: "38px",
                    height: "32px",
                    padding: "2px",
                    border: "1px solid var(--color-border)",
                    borderRadius: "6px",
                    backgroundColor: "var(--color-bg)",
                    cursor: "pointer",
                  }}
                />
                <input
                  aria-label={t("settings.accountColorHex", "Account color hex")}
                  style={{ ...inputStyle, width: "96px", fontFamily: "monospace" }}
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
                  gap: "6px",
                  marginTop: "8px",
                }}
              >
                {ACCOUNT_COLOR_PRESETS.map((preset) => {
                  const presetLabel = `${t("settings.useAccountColorPreset", "Use color")} ${preset.color}`;
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
                        width: "22px",
                        height: "22px",
                        borderRadius: "50%",
                        border: selected ? "2px solid var(--color-text-primary)" : "1px solid var(--color-border)",
                        backgroundColor: preset.color,
                        cursor: "pointer",
                        padding: 0,
                        boxShadow: selected ? `0 0 0 2px ${preset.color}33` : "none",
                      }}
                    />
                  );
                })}
              </div>
            </div>
            {isOAuth ? (
              <div
                style={{
                  padding: "10px 12px",
                  borderRadius: "6px",
                  backgroundColor: "rgba(59,130,246,0.08)",
                  border: "1px solid rgba(59,130,246,0.25)",
                  color: "var(--color-text-secondary)",
                  fontSize: "12px",
                  lineHeight: 1.5,
                }}
              >
                {t(
                  "settings.oauthAccountNote",
                  "This account uses OAuth. Provider sign-in, password, and IMAP/SMTP settings are managed by the provider. Leave the proxy blank to inherit the global SOCKS5 proxy."
                )}
              </div>
            ) : (
              <>
                <div style={fieldStyle}>
                  <label style={labelStyle}>{t("accountSetup.password")} <span style={{ color: "var(--color-text-secondary)", fontWeight: 400 }}>({t("settings.leaveEmptyKeep", "leave empty to keep current")})</span></label>
                  <input aria-label={t("accountSetup.password")} style={inputStyle} type="password" value={password} onChange={(e) => setPassword(e.target.value)} />
                </div>

                <div style={{ display: "grid", gridTemplateColumns: "1fr auto auto", gap: "12px" }}>
                  <div style={fieldStyle}>
                    <label style={labelStyle}>{incomingHostLabel} <span style={{ color: "var(--color-text-secondary)", fontWeight: 400 }}>({t("settings.optional", "optional")})</span></label>
                    <input aria-label={incomingHostLabel} style={inputStyle} type="text" value={imapHost} onChange={(e) => setImapHost(e.target.value)} placeholder={t("settings.leaveEmptyKeep")} />
                  </div>
                  <div style={fieldStyle}>
                    <label style={labelStyle}>{incomingPortLabel}</label>
                    <input aria-label={incomingPortLabel} style={{ ...inputStyle, width: "70px" }} type="number" min={1} max={65535} step={1} inputMode="numeric" value={imapPort} onChange={(e) => setImapPort(e.target.value)} />
                  </div>
                  <div style={fieldStyle}>
                    <label htmlFor="accountsetup-imap-security" style={labelStyle}>{t("accountSetup.security", "Security")}</label>
                    <select id="accountsetup-imap-security" value={imapSecurity} onChange={(e) => setImapSecurity(e.target.value as ConnectionSecurity | "")} style={{ ...inputStyle, width: "110px" }}>
                      <option value="">{t("settings.leaveEmptyKeep", "keep current")}</option>
                      <option value="tls">{t("accountSetup.securityTls", "SSL/TLS")}</option>
                      <option value="starttls">{t("accountSetup.securityStarttls", "STARTTLS")}</option>
                      <option value="plain">{t("accountSetup.securityPlain", "None")}</option>
                    </select>
                  </div>
                </div>

                <div style={{ display: "grid", gridTemplateColumns: "1fr auto auto", gap: "12px" }}>
                  <div style={fieldStyle}>
                    <label style={labelStyle}>{t("accountSetup.smtpHost")} <span style={{ color: "var(--color-text-secondary)", fontWeight: 400 }}>({t("settings.optional", "optional")})</span></label>
                    <input aria-label={t("accountSetup.smtpHost")} style={inputStyle} type="text" value={smtpHost} onChange={(e) => setSmtpHost(e.target.value)} placeholder={t("settings.leaveEmptyKeep")} />
                  </div>
                  <div style={fieldStyle}>
                    <label style={labelStyle}>{t("accountSetup.smtpPort")}</label>
                    <input aria-label={t("accountSetup.smtpPort")} style={{ ...inputStyle, width: "70px" }} type="number" min={1} max={65535} step={1} inputMode="numeric" value={smtpPort} onChange={(e) => setSmtpPort(e.target.value)} />
                  </div>
                  <div style={fieldStyle}>
                    <label htmlFor="accountsetup-smtp-security" style={labelStyle}>{t("accountSetup.security", "Security")}</label>
                    <select id="accountsetup-smtp-security" value={smtpSecurity} onChange={(e) => setSmtpSecurity(e.target.value as ConnectionSecurity | "")} style={{ ...inputStyle, width: "110px" }}>
                      <option value="">{t("settings.leaveEmptyKeep", "keep current")}</option>
                      <option value="tls">{t("accountSetup.securityTls", "SSL/TLS")}</option>
                      <option value="starttls">{t("accountSetup.securityStarttls", "STARTTLS")}</option>
                      <option value="plain">{t("accountSetup.securityPlain", "None")}</option>
                    </select>
                  </div>
                </div>

                <div style={fieldStyle}>
                  <label htmlFor="accountsetup-cert-policy" style={labelStyle}>
                    {t("accountSetup.tlsCertificateVerification", "TLS certificate verification")}
                  </label>
                  <select
                    id="accountsetup-cert-policy"
                    value={acceptInvalidCerts}
                    onChange={(e) => setAcceptInvalidCerts(e.target.value as "" | "false" | "true")}
                    style={inputStyle}
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
            {rangeLoading && <div role="status">{t("common.loading", "Loading...")}</div>}
            {rangeError && <div role="alert" style={{ color: "var(--color-danger)", fontSize: "12px" }}>
              {t("syncRange.loadError", "Could not load the sync range. Close and reopen this dialog to retry.")} {rangeError}
            </div>}

            {isImap && (
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: "10px",
                  padding: "12px",
                  border: "1px solid var(--color-border)",
                  borderRadius: "8px",
                }}
              >
                <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: "12px" }}>
                  <div>
                    <div style={{ ...labelStyle, marginBottom: "3px" }}>
                      {t("settings.syncFolders", "Folders to sync")}
                    </div>
                    <div style={{ fontSize: "12px", color: "var(--color-text-secondary)", lineHeight: 1.45 }}>
                      {t(
                        "settings.syncFoldersDesc",
                        "Inbox is always synced. Choose any additional IMAP folders to keep locally.",
                      )}
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick={loadImapFolders}
                    disabled={folderLoading || loading}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "6px",
                      padding: "7px 10px",
                      borderRadius: "6px",
                      border: "1px solid var(--color-border)",
                      backgroundColor: "var(--color-bg)",
                      color: "var(--color-text-primary)",
                      fontSize: "12px",
                      cursor: folderLoading || loading ? "wait" : "pointer",
                      opacity: folderLoading || loading ? 0.7 : 1,
                      flexShrink: 0,
                    }}
                  >
                    <RefreshCw size={13} aria-hidden="true" />
                    {folderLoading
                      ? t("settings.loadingFolders", "Loading...")
                      : imapFolderSettings
                        ? t("settings.refreshFolders", "Refresh folders")
                        : t("settings.chooseFolders", "Choose folders")}
                  </button>
                </div>

                {folderError && (
                  <div role="alert" style={{ color: "#ef4444", fontSize: "12px" }}>
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
                        gap: "7px",
                        maxHeight: "180px",
                        overflowY: "auto",
                        padding: "9px",
                        borderRadius: "6px",
                        backgroundColor: "var(--color-bg-secondary, var(--color-bg-hover))",
                      }}
                    >
                      {imapFolderSettings.folders.map((folder) => {
                        const isInboxFolder =
                          folder.role === "inbox" || folder.remote_id.toUpperCase() === "INBOX";
                        return (
                          <label
                            key={folder.remote_id}
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: "8px",
                              color: "var(--color-text-primary)",
                              fontSize: "12px",
                              cursor: isInboxFolder ? "default" : "pointer",
                            }}
                          >
                            <input
                              type="checkbox"
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
                              <span style={{ color: "var(--color-text-secondary)", marginLeft: "auto" }}>
                                {t("settings.alwaysSynced", "Always synced")}
                              </span>
                            )}
                          </label>
                        );
                      })}
                    </div>
                    <div style={{ color: "var(--color-text-secondary)", fontSize: "11px", lineHeight: 1.45 }}>
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
            <div style={fieldStyle}>
              <label style={labelStyle}>{t("settings.signature", "Signature")} <span style={{ color: "var(--color-text-secondary)", fontWeight: 400 }}>({t("settings.optional", "optional")})</span></label>
              <textarea
                style={{ ...inputStyle, minHeight: "80px", resize: "vertical", fontFamily: "inherit" }}
                value={signature}
                onChange={(e) => setSignatureValue(e.target.value)}
                placeholder={t("settings.signaturePlaceholder", "Your email signature...")}
              />
            </div>

            {error && (
              <div role="alert" aria-live="assertive" className="dialog-error" style={{ margin: 0 }}>
                {error}
              </div>
            )}

            <div className="dialog-actions" style={{ marginTop: "4px" }}>
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
        </div>
    </Dialog>
  );
}
