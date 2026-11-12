import { useEffect, useRef, useState } from "react";
import i18n from "@/lib/i18n";
import { useConfirmStore } from "@/stores/confirm.store";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { parseProxyPort } from "@/lib/proxy-port";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { addAccount, startSync, testImapConnection, testPop3Connection, completeOAuthFlow } from "@/lib/api";
import type { AddAccountRequest } from "@/lib/api";
import { accountsQueryKey } from "@/hooks/queries";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import { realtimePreferenceToPollInterval, useUIStore } from "@/stores/ui.store";
import { useToastStore } from "@/stores/toast.store";
import { inputStyle, labelStyle } from "../styles/form";
import SyncRangeFields, { syncRangeDays, syncRangeDraft } from "./SyncRangeFields";

const FOLDER_REFRESH_ATTEMPTS = 5;
const FOLDER_REFRESH_INTERVAL_MS = 2000;

function refreshFoldersAfterSyncStart(queryClient: QueryClient, accountId: string) {
  void queryClient.invalidateQueries({ queryKey: ["folders", accountId] });
  void queryClient.invalidateQueries({ queryKey: ["folders"] });

  const pollFolders = (attempts: number) => {
    if (attempts <= 0) return;
    window.setTimeout(() => {
      void queryClient.invalidateQueries({ queryKey: ["folders"] });
      void queryClient.invalidateQueries({ queryKey: ["folders", accountId] });
      pollFolders(attempts - 1);
    }, FOLDER_REFRESH_INTERVAL_MS);
  };
  pollFolders(FOLDER_REFRESH_ATTEMPTS);
}

const PRESETS: Record<
  string,
  Pick<
    AddAccountRequest,
    "imap_host" | "imap_port" | "smtp_host" | "smtp_port" | "imap_security" | "smtp_security"
  >
> = {
  gmail: {
    imap_host: "imap.gmail.com",
    imap_port: 993,
    imap_security: "tls",
    smtp_host: "smtp.gmail.com",
    smtp_port: 587,
    smtp_security: "starttls",
  },
  outlook: {
    imap_host: "outlook.office365.com",
    imap_port: 993,
    imap_security: "tls",
    smtp_host: "smtp.office365.com",
    smtp_port: 587,
    smtp_security: "starttls",
  },
  qq: {
    imap_host: "imap.qq.com",
    imap_port: 993,
    imap_security: "tls",
    smtp_host: "smtp.qq.com",
    smtp_port: 465,
    smtp_security: "tls",
  },
  "163": {
    imap_host: "imap.163.com",
    imap_port: 993,
    imap_security: "tls",
    smtp_host: "smtp.163.com",
    smtp_port: 465,
    smtp_security: "tls",
  },
};

interface Props {
  onClose: () => void;
}

export default function AccountSetup({ onClose }: Props) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const emailInputRef = useRef<HTMLInputElement>(null);
  const realtimeMode = useUIStore((state) => state.realtimeMode);
  const syncPollInterval = realtimePreferenceToPollInterval(realtimeMode);

  const initialForm: AddAccountRequest = {
    email: "",
    display_name: "",
    account_label: "",
    provider: "imap",
    imap_host: "",
    imap_port: 993,
    imap_security: "tls",
    smtp_host: "",
    smtp_port: 587,
    smtp_security: "starttls",
    accept_invalid_certs: false,
    allow_plaintext: false,
    username: "",
    password: "",
  };
  const [form, setForm] = useState<AddAccountRequest>(initialForm);
  const [syncRange, setSyncRange] = useState(() => syncRangeDraft(null));
  const initialFormRef = useRef(initialForm);

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [testLoading, setTestLoading] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  const dialogRef = useRef<HTMLDivElement>(null);
  const formRef = useRef(form);
  formRef.current = form;

  const requestClose = async () => {
    const current = formRef.current;
    const initial = initialFormRef.current;
    const isDirty = (Object.keys(current) as Array<keyof AddAccountRequest>).some(
      (k) => current[k] !== initial[k],
    );
    if (!isDirty && syncRange.preset === "all" && !syncRange.customDays) {
      onClose();
      return;
    }
    const confirmed = await useConfirmStore.getState().confirm({
      title: i18n.t("accountSetup.discardTitle", "Discard form"),
      message: i18n.t("accountSetup.discardConfirm", "Discard this form?"),
      destructive: true,
    });
    if (confirmed) onClose();
  };
  const requestCloseRef = useRef(requestClose);
  requestCloseRef.current = requestClose;

  useEffect(() => {
    const previousFocus =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;

    emailInputRef.current?.focus();

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        void requestCloseRef.current();
        return;
      }
      // Focus trap: keep Tab within the dialog
      if (event.key === "Tab" && dialogRef.current) {
        const focusable = dialogRef.current.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        );
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    }

    document.addEventListener("keydown", handleKeyDown);

    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      previousFocus?.focus();
    };
  }, [onClose]);

  function validatedProxyPort() {
    return parseProxyPort(String(form.proxy_port ?? ""), t("settings.proxyPortInvalid", "Port must be an integer between 1 and 65535."));
  }

  async function handleTestConnection() {
    setTestResult(null);
    setTestLoading(true);
    try {
      const proxyPort = validatedProxyPort();
      const report =
        form.provider === "pop3"
          ? await testPop3Connection(
              form.imap_host,
              form.imap_port,
              form.imap_security,
              form.accept_invalid_certs,
              form.proxy_host,
              proxyPort,
              form.username || undefined,
              form.password || undefined,
              form.allow_plaintext,
            )
          : await testImapConnection(
              form.imap_host,
              form.imap_port,
              form.imap_security,
              form.accept_invalid_certs,
              form.proxy_host,
              proxyPort,
              form.username || undefined,
              form.password || undefined,
              form.email || undefined,
              form.allow_plaintext,
            );
      setTestResult({ ok: true, message: report });
    } catch (err) {
      const msg = extractErrorMessage(err);
      setTestResult({ ok: false, message: msg });
    } finally {
      setTestLoading(false);
    }
  }

  const [oauthLoading, setOauthLoading] = useState(false);

  async function handleOAuth(provider: "gmail" | "outlook") {
    const days = syncRangeDays(syncRange);
    if (days === undefined) return;
    setOauthLoading(true);
    setError(null);
    try {
      const account = await completeOAuthFlow(
        provider,
        form.email || "",
        form.display_name || "",
        form.proxy_host?.trim() || undefined,
        validatedProxyPort(),
        form.account_label || undefined,
        days,
      );
      await queryClient.invalidateQueries({ queryKey: accountsQueryKey });
      await startSync(account.id, syncPollInterval);
      refreshFoldersAfterSyncStart(queryClient, account.id);
      onClose();
    } catch (err) {
      const msg = extractErrorMessage(err);
      setError(msg);
    } finally {
      setOauthLoading(false);
    }
  }

  function applyPreset(key: string) {
    const preset = PRESETS[key];
    if (!preset) return;
    setForm((prev) => ({ ...prev, provider: "imap", ...preset }));
  }

  function handleChange(field: keyof AddAccountRequest, value: string | number | boolean) {
    setForm((prev) => {
      const updated = { ...prev, [field]: value };
      // Keep username in sync with email when username hasn't been manually changed
      if (field === "email" && prev.username === prev.email) {
        updated.username = value as string;
      }
      // The plaintext opt-in checkbox only shows while a connection is
      // unencrypted. If the user switches both sides back to an encrypted
      // mode, clear the flag so a transiently-checked box can't persist
      // allow_plaintext=true onto an encrypted account (issue #70).
      if (field === "imap_security" || field === "smtp_security") {
        const anyPlain =
          updated.imap_security === "plain" || updated.smtp_security === "plain";
        if (!anyPlain) {
          updated.allow_plaintext = false;
        }
      }
      return updated;
    });
  }

  function handleIncomingProtocolChange(provider: "imap" | "pop3") {
    setForm((prev) => ({
      ...prev,
      provider,
      imap_port: provider === "pop3" ? 995 : 993,
      imap_security: "tls",
    }));
  }

  async function handleSubmit(e: React.SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    const days = form.provider === "pop3" ? null : syncRangeDays(syncRange);
    if (days === undefined) return;
    setError(null);
    setLoading(true);
    try {
      const account = await addAccount({ ...form, proxy_port: validatedProxyPort(), sync_range_days: days });
      // Invalidate accounts immediately so UI reflects the new account
      await queryClient.invalidateQueries({ queryKey: accountsQueryKey });
      onClose();
      useToastStore.getState().addToast({
        message: t("accountSetup.accountAdded", "Account added successfully"),
        type: "success",
      });
      // Start sync in background; poll folders until they appear
      startSync(account.id, syncPollInterval).catch((err) =>
        console.warn("Initial sync failed (will retry later):", err),
      );
      // Poll for folders a few times so sidebar updates without manual refresh
      refreshFoldersAfterSyncStart(queryClient, account.id);
    } catch (err) {
      setError(extractErrorMessage(err));
    } finally {
      setLoading(false);
    }
  }

  const fieldStyle: React.CSSProperties = {
    display: "flex",
    flexDirection: "column",
    gap: "0",
  };
  const isPop3 = form.provider === "pop3";
  const incomingHostLabel = isPop3
    ? t("accountSetup.pop3Host", "POP3 host")
    : t("accountSetup.imapHost", "IMAP host");
  const incomingPortLabel = isPop3
    ? t("accountSetup.pop3Port", "POP3 port")
    : t("accountSetup.imapPort", "IMAP port");
  const incomingHostPlaceholder = isPop3 ? "pop.example.com" : "imap.example.com";
  const canTestConnection =
    !!form.imap_host && (!isPop3 || (!!form.username && !!form.password));

  const proxyFields = (
    <div style={{ display: "grid", gridTemplateColumns: "1fr auto", gap: "12px", marginBottom: "16px" }}>
      <div style={fieldStyle}>
        <label htmlFor="setup-proxy-host" style={labelStyle}>
          {t("accountSetup.proxyHost", "SOCKS5 Proxy")}
          {" "}<span style={{ color: "var(--color-text-secondary)", fontWeight: 400 }}>({t("settings.optional", "optional")})</span>
        </label>
        <input
          id="setup-proxy-host"
          aria-label={t("accountSetup.proxyHost", "SOCKS5 Proxy")}
          style={inputStyle}
          type="text"
          value={form.proxy_host ?? ""}
          onChange={(e) => setForm((prev) => ({ ...prev, proxy_host: e.target.value || undefined }))}
          placeholder="127.0.0.1"
        />
      </div>
      <div style={fieldStyle}>
        <label htmlFor="setup-proxy-port" style={labelStyle}>{t("accountSetup.proxyPort", "Port")}</label>
        <input
          id="setup-proxy-port"
          aria-label={t("accountSetup.proxyPort", "Port")}
          style={{ ...inputStyle, width: "80px" }}
          type="number"
          value={form.proxy_port ?? ""}
          onChange={(e) => setForm((prev) => ({ ...prev, proxy_port: e.target.value ? Number(e.target.value) : undefined }))}
          placeholder="7890"
        />
      </div>
    </div>
  );

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="account-setup-title"
      style={{
        position: "fixed",
        inset: 0,
        backgroundColor: "rgba(0,0,0,0.5)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
      }}
    >
      <div
        ref={dialogRef}
        style={{
          width: "min(480px, calc(100vw - 32px))",
          backgroundColor: "var(--color-bg)",
          borderRadius: "10px",
          boxShadow: "0 20px 60px rgba(0,0,0,0.3)",
          display: "flex",
          flexDirection: "column",
          maxHeight: "90vh",
          overflow: "hidden",
        }}
      >
        {/* Header */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "16px 20px",
            borderBottom: "1px solid var(--color-border)",
          }}
        >
          <h2
            id="account-setup-title"
            style={{
              margin: 0,
              fontSize: "15px",
              fontWeight: 600,
              color: "var(--color-text-primary)",
            }}
          >
            {t("accountSetup.title", "Add Email Account")}
          </h2>
          <button
            onClick={() => void requestClose()}
            aria-label={t("common.close", "Close")}
            style={{
              background: "none",
              border: "none",
              cursor: "pointer",
              padding: "4px",
              borderRadius: "4px",
              color: "var(--color-text-secondary)",
              display: "flex",
              alignItems: "center",
            }}
          >
            <X size={18} />
          </button>
        </div>

        {/* Scrollable body */}
        <div className="scroll-region account-setup-scroll" style={{ overflowY: "auto", padding: "20px" }}>
          <div style={{ ...fieldStyle, marginBottom: "16px" }}>
            <label htmlFor="setup-account-label" style={labelStyle}>{t("accountSetup.accountLabel", "Account label")}</label>
            <input id="setup-account-label" style={inputStyle} type="text" maxLength={120}
              value={form.account_label || ""} onChange={(e) => handleChange("account_label", e.target.value)} />
            <small>{t("accountSetup.accountLabelHelp", "Only used to identify this account in Pebble; never sent as your sender name.")}</small>
          </div>
          {/* OAuth sign-in buttons */}
          <SyncRangeFields value={syncRange} onChange={setSyncRange} disabled={loading || oauthLoading} />
          <div style={{ display: "flex", gap: "8px", marginBottom: "16px" }}>
            <button
              type="button"
              disabled={loading || oauthLoading || syncRangeDays(syncRange) === undefined}
              onClick={() => handleOAuth("gmail")}
              style={{
                flex: 1,
                padding: "10px 16px",
                borderRadius: "6px",
                border: "1px solid var(--color-border)",
                backgroundColor: "var(--color-bg)",
                color: "var(--color-text-primary)",
                fontSize: "13px",
                fontWeight: 500,
                cursor: oauthLoading ? "wait" : "pointer",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                gap: "8px",
                opacity: oauthLoading ? 0.7 : 1,
              }}
            >
              <svg width="16" height="16" viewBox="0 0 48 48"><path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"/><path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"/><path fill="#34A853" d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z"/><path fill="#FBBC05" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"/></svg>
              {oauthLoading ? t("accountSetup.adding", "Signing in...") : t("accountSetup.signInGoogle", "Sign in with Google")}
            </button>
            <button
              type="button"
              disabled={loading || oauthLoading || syncRangeDays(syncRange) === undefined}
              onClick={() => handleOAuth("outlook")}
              style={{
                flex: 1,
                padding: "10px 16px",
                borderRadius: "6px",
                border: "1px solid var(--color-border)",
                backgroundColor: "var(--color-bg)",
                color: "var(--color-text-primary)",
                fontSize: "13px",
                fontWeight: 500,
                cursor: oauthLoading ? "wait" : "pointer",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                gap: "8px",
                opacity: oauthLoading ? 0.7 : 1,
              }}
            >
              <svg width="16" height="16" viewBox="0 0 23 23"><path fill="#f35325" d="M1 1h10v10H1z"/><path fill="#81bc06" d="M12 1h10v10H12z"/><path fill="#05a6f0" d="M1 12h10v10H1z"/><path fill="#ffba08" d="M12 12h10v10H12z"/></svg>
              {oauthLoading ? t("accountSetup.adding", "Signing in...") : t("accountSetup.signInOutlook", "Sign in with Microsoft")}
            </button>
          </div>

          {proxyFields}

          <div style={{ textAlign: "center", color: "var(--color-text-secondary)", fontSize: "12px", marginBottom: "16px" }}>
            {t("accountSetup.orManual", "or add account manually")}
          </div>

          {/* Preset buttons */}
          <div style={{ marginBottom: "20px" }}>
            <span style={{ ...labelStyle, marginBottom: "8px" }}>{t("accountSetup.quickSetup", "Quick setup")}</span>
            <div style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}>
              {Object.keys(PRESETS).map((key) => (
                <button
                  key={key}
                  type="button"
                  onClick={() => applyPreset(key)}
                  style={{
                    padding: "5px 14px",
                    borderRadius: "20px",
                    border: "1px solid var(--color-border)",
                    backgroundColor: "transparent",
                    color: "var(--color-text-primary)",
                    fontSize: "12px",
                    cursor: "pointer",
                    textTransform: "capitalize",
                  }}
                >
                  {key === "163" ? "163" : key.charAt(0).toUpperCase() + key.slice(1)}
                </button>
              ))}
            </div>
          </div>

          <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
            {/* Email */}
            <div style={fieldStyle}>
              <label htmlFor="setup-email" style={labelStyle}>{t("accountSetup.emailAddress", "Email address")}</label>
                <input
                  ref={emailInputRef}
                  id="setup-email"
                  name="email"
                  autoComplete="email"
                style={inputStyle}
                type="email"
                required
                value={form.email}
                onChange={(e) => handleChange("email", e.target.value)}
                placeholder={t("accountSetup.emailPlaceholder", "you@example.com")}
              />
            </div>

            {/* Display name */}
            <div style={fieldStyle}>
              <label htmlFor="setup-display-name" style={labelStyle}>{t("accountSetup.displayName", "Sender name")}</label>
              <input
                id="setup-display-name"
                name="display_name"
                autoComplete="name"
                style={inputStyle}
                type="text"
                value={form.display_name}
                onChange={(e) => handleChange("display_name", e.target.value)}
                placeholder={t("accountSetup.namePlaceholder", "Your Name")}
              />
            </div>

            {/* Incoming */}
            <div style={fieldStyle}>
              <label htmlFor="setup-incoming-protocol" style={labelStyle}>
                {t("accountSetup.incomingProtocol", "Incoming protocol")}
              </label>
              <select
                id="setup-incoming-protocol"
                value={isPop3 ? "pop3" : "imap"}
                onChange={(e) => handleIncomingProtocolChange(e.target.value as "imap" | "pop3")}
                style={inputStyle}
              >
                <option value="imap">{t("accountSetup.incomingProtocolImap", "IMAP")}</option>
                <option value="pop3">{t("accountSetup.incomingProtocolPop3", "POP3")}</option>
              </select>
              {isPop3 && <div style={{ color: "var(--color-text-secondary)", fontSize: "12px", marginTop: "6px" }}>
                {t("syncRange.pop3", "POP3 does not support server-side date filtering; all mail will be synced.")}
              </div>}
            </div>

            <div style={{ display: "grid", gridTemplateColumns: "1fr auto auto", gap: "12px" }}>
              <div style={fieldStyle}>
                <label htmlFor="setup-imap-host" style={labelStyle}>{incomingHostLabel}</label>
                <input
                  id="setup-imap-host"
                  name="imap_host"
                  style={inputStyle}
                  type="text"
                  required
                  value={form.imap_host}
                  onChange={(e) => handleChange("imap_host", e.target.value)}
                  placeholder={incomingHostPlaceholder}
                />
              </div>
              <div style={fieldStyle}>
                <label htmlFor="setup-imap-port" style={labelStyle}>{incomingPortLabel}</label>
                <input
                  id="setup-imap-port"
                  name="imap_port"
                  style={{ ...inputStyle, width: "70px" }}
                  type="number"
                  required
                  value={form.imap_port}
                  onChange={(e) => handleChange("imap_port", parseInt(e.target.value, 10))}
                />
              </div>
              <div style={fieldStyle}>
                <label htmlFor="setup-imap-security" style={labelStyle}>{t("accountSetup.security", "Security")}</label>
                <select
                  id="setup-imap-security"
                  value={form.imap_security}
                  onChange={(e) => handleChange("imap_security", e.target.value)}
                  style={{ ...inputStyle, width: "110px" }}
                >
                  <option value="tls">{t("accountSetup.securityTls", "SSL/TLS")}</option>
                  <option value="starttls">{t("accountSetup.securityStarttls", "STARTTLS")}</option>
                  <option value="plain">{t("accountSetup.securityPlain", "None")}</option>
                </select>
              </div>
            </div>

            {/* SMTP */}
            <div style={{ display: "grid", gridTemplateColumns: "1fr auto auto", gap: "12px" }}>
              <div style={fieldStyle}>
                <label htmlFor="setup-smtp-host" style={labelStyle}>{t("accountSetup.smtpHost", "SMTP host")}</label>
                <input
                  id="setup-smtp-host"
                  name="smtp_host"
                  style={inputStyle}
                  type="text"
                  required
                  value={form.smtp_host}
                  onChange={(e) => handleChange("smtp_host", e.target.value)}
                  placeholder="smtp.example.com"
                />
              </div>
              <div style={fieldStyle}>
                <label htmlFor="setup-smtp-port" style={labelStyle}>{t("accountSetup.smtpPort", "SMTP port")}</label>
                <input
                  id="setup-smtp-port"
                  name="smtp_port"
                  style={{ ...inputStyle, width: "70px" }}
                  type="number"
                  required
                  value={form.smtp_port}
                  onChange={(e) => handleChange("smtp_port", parseInt(e.target.value, 10))}
                />
              </div>
              <div style={fieldStyle}>
                <label htmlFor="setup-smtp-security" style={labelStyle}>{t("accountSetup.security", "Security")}</label>
                <select
                  id="setup-smtp-security"
                  value={form.smtp_security}
                  onChange={(e) => handleChange("smtp_security", e.target.value)}
                  style={{ ...inputStyle, width: "110px" }}
                >
                  <option value="tls">{t("accountSetup.securityTls", "SSL/TLS")}</option>
                  <option value="starttls">{t("accountSetup.securityStarttls", "STARTTLS")}</option>
                  <option value="plain">{t("accountSetup.securityPlain", "None")}</option>
                </select>
              </div>
            </div>

            {/* TLS certificate policy */}
            <label
              htmlFor="setup-accept-invalid-certs"
              style={{
                display: "flex",
                alignItems: "center",
                gap: "8px",
                fontSize: "12px",
                color: "var(--color-text-primary)",
              }}
            >
              <input
                id="setup-accept-invalid-certs"
                type="checkbox"
                checked={!!form.accept_invalid_certs}
                onChange={(e) => handleChange("accept_invalid_certs", e.target.checked)}
              />
              {t("accountSetup.acceptInvalidCerts", "Allow invalid TLS certificates")}
            </label>

            {(form.imap_security === "plain" || form.smtp_security === "plain") && (
              <label
                htmlFor="setup-allow-plaintext"
                style={{
                  display: "flex",
                  alignItems: "flex-start",
                  gap: "8px",
                  fontSize: "12px",
                  color: "var(--color-warning, #b45309)",
                }}
              >
                <input
                  id="setup-allow-plaintext"
                  type="checkbox"
                  checked={!!form.allow_plaintext}
                  onChange={(e) => handleChange("allow_plaintext", e.target.checked)}
                  style={{ marginTop: "2px" }}
                />
                <span>
                  {t(
                    "accountSetup.allowPlaintext",
                    "Allow unencrypted connection (insecure — your password is sent in cleartext)",
                  )}
                </span>
              </label>
            )}

            {/* Username */}
            <div style={fieldStyle}>
              <label htmlFor="setup-username" style={labelStyle}>{t("accountSetup.username", "Username")}</label>
              <input
                id="setup-username"
                name="username"
                autoComplete="username"
                style={inputStyle}
                type="text"
                value={form.username}
                onChange={(e) => handleChange("username", e.target.value)}
                placeholder={t("accountSetup.usernameHint", "Defaults to email address")}
              />
            </div>

            {/* Password */}
            <div style={fieldStyle}>
              <label htmlFor="setup-password" style={labelStyle}>{t("accountSetup.password", "Password / App password")}</label>
              <input
                id="setup-password"
                name="password"
                autoComplete="current-password"
                style={inputStyle}
                type="password"
                required
                value={form.password}
                onChange={(e) => handleChange("password", e.target.value)}
              />
            </div>

            {/* Test Connection */}
            {testResult && (
              <div
                role={testResult.ok ? "status" : "alert"}
                aria-live={testResult.ok ? "polite" : "assertive"}
                style={{
                  padding: "10px 12px",
                  borderRadius: "6px",
                  backgroundColor: testResult.ok ? "rgba(34,197,94,0.1)" : "rgba(239,68,68,0.1)",
                  border: `1px solid ${testResult.ok ? "rgba(34,197,94,0.3)" : "rgba(239,68,68,0.3)"}`,
                  color: testResult.ok ? "#22c55e" : "#ef4444",
                  fontSize: "12px",
                  whiteSpace: "pre-wrap",
                  fontFamily: "monospace",
                  lineHeight: 1.5,
                }}
              >
                {testResult.message}
              </div>
            )}

            {/* Error */}
            {error && (
              <div
                role="alert"
                aria-live="assertive"
                style={{
                  padding: "10px 12px",
                  borderRadius: "6px",
                  backgroundColor: "rgba(239,68,68,0.1)",
                  border: "1px solid rgba(239,68,68,0.3)",
                  color: "#ef4444",
                  fontSize: "13px",
                }}
              >
                {error}
              </div>
            )}

            {/* Buttons */}
            <div style={{ display: "flex", gap: "10px", marginTop: "4px" }}>
              <button
                type="button"
                disabled={testLoading || !canTestConnection}
                onClick={handleTestConnection}
                style={{
                  padding: "9px 16px",
                  borderRadius: "6px",
                  border: "1px solid var(--color-border)",
                  backgroundColor: "transparent",
                  color: "var(--color-text-primary)",
                  fontSize: "13px",
                  fontWeight: 500,
                  cursor: testLoading || !canTestConnection ? "not-allowed" : "pointer",
                  opacity: testLoading || !canTestConnection ? 0.6 : 1,
                }}
              >
                {testLoading ? t("accountSetup.testing", "Testing...") : t("accountSetup.testConnection", "Test Connection")}
              </button>
              <button
                type="submit"
                disabled={loading || oauthLoading || (!isPop3 && syncRangeDays(syncRange) === undefined)}
                style={{
                  flex: 1,
                  padding: "9px 16px",
                  borderRadius: "6px",
                  border: "none",
                  backgroundColor: "var(--color-accent)",
                  color: "#fff",
                  fontSize: "13px",
                  fontWeight: 600,
                  cursor: loading ? "not-allowed" : "pointer",
                  opacity: loading ? 0.7 : 1,
                }}
              >
                {loading ? t("accountSetup.adding", "Adding account…") : t("accountSetup.submit", "Add Account & Sync")}
              </button>
            </div>
          </form>
        </div>
      </div>
    </div>
  );
}
