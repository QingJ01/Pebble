import { useRef, useState } from "react";
import i18n from "@/lib/i18n";
import { useConfirmStore } from "@/stores/confirm.store";
import { X } from "lucide-react";
import Dialog from "./ui/Dialog";
import Button from "./ui/Button";
import IconButton from "./ui/IconButton";
import { useTranslation } from "react-i18next";
import { parseProxyPort, PROXY_PORT_ERROR_KEY } from "@/lib/proxy-port";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { addAccount, startSync, testImapConnection, testPop3Connection, completeOAuthFlow } from "@/lib/api";
import type { AddAccountRequest } from "@/lib/api";
import { accountsQueryKey } from "@/hooks/queries";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import { realtimePreferenceToPollInterval, useUIStore } from "@/stores/ui.store";
import { useToastStore } from "@/stores/toast.store";
import SyncRangeFields, { syncRangeDays, syncRangeDraft } from "./SyncRangeFields";
import {
  providerById,
  providerForEmail,
  QUICK_SETUP_PROVIDER_IDS,
  type MailProvider,
  type ServerSettings,
} from "@/lib/mailProviders";

const SERVER_FIELDS = [
  "imap_host", "imap_port", "imap_security", "smtp_host", "smtp_port", "smtp_security",
] as const satisfies ReadonlyArray<keyof ServerSettings>;

function serverFieldsMatch(form: AddAccountRequest, settings: ServerSettings): boolean {
  return SERVER_FIELDS.every((field) => form[field] === settings[field]);
}

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
      confirmLabel: i18n.t("accountSetup.discardAction", "Discard"),
      destructive: true,
    });
    if (confirmed) onClose();
  };
  const requestCloseRef = useRef(requestClose);
  requestCloseRef.current = requestClose;


  function validatedProxyPort() {
    return parseProxyPort(String(form.proxy_port ?? ""), t(PROXY_PORT_ERROR_KEY, "Port must be an integer between 1 and 65535."));
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
              form.proxy_host ?? undefined,
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
              form.proxy_host ?? undefined,
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

  /** Server settings last filled in by a preset or the email domain. */
  const autoFilledRef = useRef<ServerSettings | null>(null);
  const [detectedProvider, setDetectedProvider] = useState<MailProvider | null>(null);
  // The note only shows while the fields still hold that provider's settings.
  const providerNote = detectedProvider && form.provider === "imap"
    && serverFieldsMatch(form, detectedProvider.settings) ? detectedProvider : null;

  function applyPreset(key: string) {
    const preset = providerById(key);
    if (!preset) return;
    autoFilledRef.current = preset.settings;
    setDetectedProvider(preset);
    setForm((prev) => ({ ...prev, provider: "imap", ...preset.settings }));
  }

  /**
   * Fill the server settings from the email's domain, unless the user has
   * typed their own (only blank or previously auto-filled fields change).
   */
  function autoFillFromEmail(email: string) {
    const prev = formRef.current;
    if (prev.provider !== "imap") return;
    const provider = providerForEmail(email);
    const untouched = (!prev.imap_host && !prev.smtp_host)
      || (autoFilledRef.current !== null && serverFieldsMatch(prev, autoFilledRef.current));
    if (!provider || !untouched) {
      setDetectedProvider(null);
      return;
    }
    autoFilledRef.current = provider.settings;
    setDetectedProvider(provider);
    setForm((current) => ({ ...current, ...provider.settings }));
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

  const isPop3 = form.provider === "pop3";
  const incomingHostLabel = isPop3
    ? t("accountSetup.pop3Host", "POP3 host")
    : t("accountSetup.imapHost", "IMAP host");
  const incomingPortLabel = isPop3
    ? t("accountSetup.pop3Port", "POP3 port")
    : t("accountSetup.imapPort", "IMAP port");
  const incomingHostPlaceholder = isPop3 ? "pop.example.com" : "imap.example.com";
  const formId = "account-setup-form";
  const canTestConnection =
    !!form.imap_host && (!isPop3 || (!!form.username && !!form.password));

  const proxyFields = (
    <div className="account-setup__row account-setup__row--proxy">
      <div className="field">
        <label htmlFor="setup-proxy-host" className="field__label">
          {t("accountSetup.proxyHost", "SOCKS5 Proxy")}
          {" "}<span className="account-setup__optional">({t("settings.optional", "optional")})</span>
        </label>
        <input
          id="setup-proxy-host"
          aria-label={t("accountSetup.proxyHost", "SOCKS5 Proxy")}
          className="input"
          type="text"
          value={form.proxy_host ?? ""}
          onChange={(e) => setForm((prev) => ({ ...prev, proxy_host: e.target.value || undefined }))}
          placeholder="127.0.0.1"
        />
      </div>
      <div className="field">
        <label htmlFor="setup-proxy-port" className="field__label">{t("accountSetup.proxyPort", "Port")}</label>
        <input
          id="setup-proxy-port"
          aria-label={t("accountSetup.proxyPort", "Port")}
          className="input account-setup__port"
          type="number" inputMode="numeric"
          value={form.proxy_port ?? ""}
          onChange={(e) => setForm((prev) => ({ ...prev, proxy_port: e.target.value ? Number(e.target.value) : undefined }))}
          placeholder="7890"
        />
      </div>
    </div>
  );

  return (
    <Dialog
      onClose={() => void requestClose()}
      closeOnBackdrop={false}
      labelledBy="account-setup-title"
      initialFocus={emailInputRef}
      busy={loading || oauthLoading}
      panelClassName="dialog-panel dialog-panel--flush account-setup"
    >
        <div className="dialog-panel__header">
          <h2 id="account-setup-title" className="dialog-title">
            {t("accountSetup.title", "Add Email Account")}
          </h2>
          <IconButton label={t("common.close", "Close")} icon={<X size={16} />} onClick={() => void requestClose()} />
        </div>

        {/* Scrollable body */}
        <div className="dialog-panel__body scroll-region account-setup__body">
          <div className="field">
            <label htmlFor="setup-account-label" className="field__label">{t("accountSetup.accountLabel", "Account label")}</label>
            <input id="setup-account-label" className="input" type="text" maxLength={120}
              value={form.account_label || ""} onChange={(e) => handleChange("account_label", e.target.value)} />
            <small className="field__hint">{t("accountSetup.accountLabelHelp", "Only used to identify this account in Pebble; never sent as your sender name.")}</small>
          </div>
          {/* OAuth sign-in buttons */}
          <SyncRangeFields value={syncRange} onChange={setSyncRange} disabled={loading || oauthLoading} />
          <div className="account-setup__oauth">
            <Button
              size="lg"
              disabled={loading || oauthLoading || syncRangeDays(syncRange) === undefined}
              onClick={() => handleOAuth("gmail")}
              icon={<svg width="16" height="16" viewBox="0 0 48 48"><path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"/><path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"/><path fill="#34A853" d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z"/><path fill="#FBBC05" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"/></svg>}
            >
              {oauthLoading ? t("accountSetup.adding", "Signing in...") : t("accountSetup.signInGoogle", "Sign in with Google")}
            </Button>
            <Button
              size="lg"
              disabled={loading || oauthLoading || syncRangeDays(syncRange) === undefined}
              onClick={() => handleOAuth("outlook")}
              icon={<svg width="16" height="16" viewBox="0 0 23 23"><path fill="#f35325" d="M1 1h10v10H1z"/><path fill="#81bc06" d="M12 1h10v10H12z"/><path fill="#05a6f0" d="M1 12h10v10H1z"/><path fill="#ffba08" d="M12 12h10v10H12z"/></svg>}
            >
              {oauthLoading ? t("accountSetup.adding", "Signing in...") : t("accountSetup.signInOutlook", "Sign in with Microsoft")}
            </Button>
          </div>

          {proxyFields}

          <div className="account-setup__divider">
            {t("accountSetup.orManual", "or add account manually")}
          </div>

          {/* Preset buttons */}
          <div className="field">
            <span className="field__label">{t("accountSetup.quickSetup", "Quick setup")}</span>
            <div className="account-setup__presets">
              {QUICK_SETUP_PROVIDER_IDS.map((key) => (
                <Button key={key} size="sm" onClick={() => applyPreset(key)}>
                  {providerById(key)?.name ?? key}
                </Button>
              ))}
            </div>
          </div>

          <form id={formId} onSubmit={handleSubmit} className="account-setup__form">
            {/* Email */}
            <div className="field">
              <label htmlFor="setup-email" className="field__label label-required">{t("accountSetup.emailAddress", "Email address")}</label>
                <input
                  ref={emailInputRef}
                  id="setup-email"
                  name="email"
                  autoComplete="email"
                className="input"
                type="email"
                required
                value={form.email}
                onChange={(e) => handleChange("email", e.target.value)}
                onBlur={(e) => autoFillFromEmail(e.target.value)}
                placeholder={t("accountSetup.emailPlaceholder", "you@example.com")}
                aria-describedby={providerNote ? "setup-email-provider" : undefined}
              />
              {providerNote && (
                <p id="setup-email-provider" className="field__hint" role="status">
                  {t("accountSetup.providerDetected", "Server settings filled in for {{name}}.", { name: providerNote.name })}
                  {providerNote.auth === "app-password" && " " + t("accountSetup.providerAppPassword", "{{name}} usually needs an app password (authorization code), not your sign-in password.", { name: providerNote.name })}
                  {(providerNote.auth === "google-oauth" || providerNote.auth === "microsoft-oauth") && " " + t("accountSetup.providerOAuth", "For {{name}}, the sign-in button above is easier; manual setup needs an app password.", { name: providerNote.name })}
                </p>
              )}
            </div>

            {/* Display name */}
            <div className="field">
              <label htmlFor="setup-display-name" className="field__label">{t("accountSetup.displayName", "Sender name")}</label>
              <input
                id="setup-display-name"
                name="display_name"
                autoComplete="name"
                className="input"
                type="text"
                value={form.display_name}
                onChange={(e) => handleChange("display_name", e.target.value)}
                placeholder={t("accountSetup.namePlaceholder", "Your Name")}
              />
            </div>

            {/* Incoming */}
            <div className="field">
              <label htmlFor="setup-incoming-protocol" className="field__label">
                {t("accountSetup.incomingProtocol", "Incoming protocol")}
              </label>
              <select
                id="setup-incoming-protocol"
                value={isPop3 ? "pop3" : "imap"}
                onChange={(e) => handleIncomingProtocolChange(e.target.value as "imap" | "pop3")}
                className="select"
              >
                <option value="imap">{t("accountSetup.incomingProtocolImap", "IMAP")}</option>
                <option value="pop3">{t("accountSetup.incomingProtocolPop3", "POP3")}</option>
              </select>
              {isPop3 && <div className="field__hint">
                {t("syncRange.pop3", "POP3 does not support server-side date filtering; all mail will be synced.")}
              </div>}
            </div>

            <div className="account-setup__row">
              <div className="field">
                <label htmlFor="setup-imap-host" className="field__label label-required">{incomingHostLabel}</label>
                <input
                  id="setup-imap-host"
                  name="imap_host"
                  className="input"
                  type="text"
                  required
                  value={form.imap_host}
                  onChange={(e) => handleChange("imap_host", e.target.value)}
                  placeholder={incomingHostPlaceholder}
                />
              </div>
              <div className="field">
                <label htmlFor="setup-imap-port" className="field__label label-required">{incomingPortLabel}</label>
                <input
                  id="setup-imap-port"
                  name="imap_port"
                  className="input account-setup__port"
                  type="number" min={1} max={65535} step={1} inputMode="numeric"
                  required
                  value={form.imap_port}
                  onChange={(e) => handleChange("imap_port", parseInt(e.target.value, 10))}
                />
              </div>
              <div className="field">
                <label htmlFor="setup-imap-security" className="field__label">{t("accountSetup.security", "Security")}</label>
                <select
                  id="setup-imap-security"
                  value={form.imap_security}
                  onChange={(e) => handleChange("imap_security", e.target.value)}
                  className="select account-setup__security"
                >
                  <option value="tls">{t("accountSetup.securityTls", "SSL/TLS")}</option>
                  <option value="starttls">{t("accountSetup.securityStarttls", "STARTTLS")}</option>
                  <option value="plain">{t("accountSetup.securityPlain", "None")}</option>
                </select>
              </div>
            </div>

            {/* SMTP */}
            <div className="account-setup__row">
              <div className="field">
                <label htmlFor="setup-smtp-host" className="field__label label-required">{t("accountSetup.smtpHost", "SMTP host")}</label>
                <input
                  id="setup-smtp-host"
                  name="smtp_host"
                  className="input"
                  type="text"
                  required
                  value={form.smtp_host}
                  onChange={(e) => handleChange("smtp_host", e.target.value)}
                  placeholder="smtp.example.com"
                />
              </div>
              <div className="field">
                <label htmlFor="setup-smtp-port" className="field__label label-required">{t("accountSetup.smtpPort", "SMTP port")}</label>
                <input
                  id="setup-smtp-port"
                  name="smtp_port"
                  className="input account-setup__port"
                  type="number" min={1} max={65535} step={1} inputMode="numeric"
                  required
                  value={form.smtp_port}
                  onChange={(e) => handleChange("smtp_port", parseInt(e.target.value, 10))}
                />
              </div>
              <div className="field">
                <label htmlFor="setup-smtp-security" className="field__label">{t("accountSetup.security", "Security")}</label>
                <select
                  id="setup-smtp-security"
                  value={form.smtp_security}
                  onChange={(e) => handleChange("smtp_security", e.target.value)}
                  className="select account-setup__security"
                >
                  <option value="tls">{t("accountSetup.securityTls", "SSL/TLS")}</option>
                  <option value="starttls">{t("accountSetup.securityStarttls", "STARTTLS")}</option>
                  <option value="plain">{t("accountSetup.securityPlain", "None")}</option>
                </select>
              </div>
            </div>

            {/* TLS certificate policy */}
            <label htmlFor="setup-accept-invalid-certs" className="choice account-setup__choice">
              <input
                id="setup-accept-invalid-certs"
                type="checkbox"
                className="checkbox"
                checked={!!form.accept_invalid_certs}
                onChange={(e) => handleChange("accept_invalid_certs", e.target.checked)}
              />
              {t("accountSetup.acceptInvalidCerts", "Allow invalid TLS certificates")}
            </label>

            {(form.imap_security === "plain" || form.smtp_security === "plain") && (
              <label htmlFor="setup-allow-plaintext" className="choice account-setup__choice account-setup__choice--warning">
                <input
                  id="setup-allow-plaintext"
                  type="checkbox"
                  className="checkbox"
                  checked={!!form.allow_plaintext}
                  onChange={(e) => handleChange("allow_plaintext", e.target.checked)}
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
            <div className="field">
              <label htmlFor="setup-username" className="field__label">{t("accountSetup.username", "Username")}</label>
              <input
                id="setup-username"
                name="username"
                autoComplete="username"
                className="input"
                type="text"
                value={form.username}
                onChange={(e) => handleChange("username", e.target.value)}
                placeholder={t("accountSetup.usernameHint", "Defaults to email address")}
              />
            </div>

            {/* Password */}
            <div className="field">
              <label htmlFor="setup-password" className="field__label label-required">{t("accountSetup.password", "Password / App password")}</label>
              <input
                id="setup-password"
                name="password"
                autoComplete="current-password"
                className="input"
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
                className={`account-setup__test-result ${testResult.ok ? "is-ok" : "is-error"}`}
              >
                {testResult.message}
              </div>
            )}

            {/* Error */}
            {error && (
              <div role="alert" aria-live="assertive" className="dialog-error account-setup__error">
                {error}
              </div>
            )}
          </form>
        </div>
        <div className="dialog-panel__footer">
          <Button
            variant="secondary"
            loading={testLoading}
            disabled={!canTestConnection}
            onClick={handleTestConnection}
          >
            {testLoading ? t("accountSetup.testing", "Testing...") : t("accountSetup.testConnection", "Test Connection")}
          </Button>
          <Button
            type="submit"
            form={formId}
            variant="primary"
            loading={loading}
            disabled={oauthLoading || (!isPop3 && syncRangeDays(syncRange) === undefined)}
          >
            {loading ? t("accountSetup.adding", "Adding account…") : t("accountSetup.submit", "Add Account & Sync")}
          </Button>
        </div>
    </Dialog>
  );
}
