import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getAccountProxySetting,
  getGlobalProxy,
  getOAuthAccountProxySetting,
  updateAccountProxySetting,
  updateGlobalProxy,
  updateOAuthAccountProxySetting,
} from "@/lib/api";
import type { Account, AccountProxyMode } from "@/lib/api";
import { extractErrorMessage } from "@/lib/extractErrorMessage";
import { parseProxyPort, PROXY_PORT_ERROR_KEY } from "@/lib/proxy-port";
import { useAccountsQuery } from "@/hooks/queries";
import { useToastStore } from "@/stores/toast.store";
import Button from "@/components/ui/Button";
import Input from "@/components/ui/Input";
import {
  OptionCard,
  OptionCardGroup,
  SettingsEmpty,
  SettingsList,
  SettingsSection,
  StatusChip,
} from "./SettingsLayout";
import { providerLabel } from "./providerLabel";

type AccountProxyDraft = {
  mode: AccountProxyMode;
  host: string;
  port: string;
  loading: boolean;
  saving: boolean;
  error: string | null;
};

const usesMailProxySetting = (provider: Account["provider"]) =>
  provider === "imap" || provider === "pop3";

const emptyAccountDraft = (): AccountProxyDraft => ({
  mode: "inherit",
  host: "",
  port: "",
  loading: false,
  saving: false,
  error: null,
});

export default function ProxyTab() {
  const { t } = useTranslation();
  const addToast = useToastStore((s) => s.addToast);
  const { data: accounts = [] } = useAccountsQuery();
  const [proxyHost, setProxyHost] = useState("");
  const [proxyPort, setProxyPort] = useState("");
  const [proxyLoading, setProxyLoading] = useState(true);
  const [proxySaving, setProxySaving] = useState(false);
  const [proxyError, setProxyError] = useState<string | null>(null);
  const [accountProxyDrafts, setAccountProxyDrafts] = useState<Record<string, AccountProxyDraft>>({});
  const accountIdsKey = accounts.map((account) => `${account.id}:${account.provider}`).join("|");
  const proxyAccounts = accounts;

  const modeOptions: Array<{ mode: AccountProxyMode; label: string }> = [
    { mode: "inherit", label: t("settings.accountProxyModeInherit", "Inherit global proxy") },
    { mode: "disabled", label: t("settings.accountProxyModeDisabled", "Do not use proxy") },
    { mode: "custom", label: t("settings.accountProxyModeCustom", "Use custom proxy") },
  ];

  useEffect(() => {
    let cancelled = false;
    setProxyLoading(true);
    getGlobalProxy()
      .then((proxy) => {
        if (cancelled) return;
        setProxyHost(proxy?.host ?? "");
        setProxyPort(proxy?.port ? String(proxy.port) : "");
        setProxyError(null);
      })
      .catch((err) => {
        if (!cancelled) setProxyError(extractErrorMessage(err));
      })
      .finally(() => {
        if (!cancelled) setProxyLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const accountIds = new Set(proxyAccounts.map((account) => account.id));
    setAccountProxyDrafts((current) => {
      const next: Record<string, AccountProxyDraft> = {};
      for (const account of proxyAccounts) {
        next[account.id] = current[account.id] ?? {
          ...emptyAccountDraft(),
          loading: true,
        };
      }
      return next;
    });

    proxyAccounts.forEach((account) => {
      const loadProxy = usesMailProxySetting(account.provider)
        ? getAccountProxySetting
        : getOAuthAccountProxySetting;
      setAccountProxyDrafts((current) => ({
        ...current,
        [account.id]: {
          ...emptyAccountDraft(),
          ...current[account.id],
          loading: true,
          error: null,
        },
      }));
      loadProxy(account.id)
        .then((setting) => {
          if (cancelled || !accountIds.has(account.id)) return;
          setAccountProxyDrafts((current) => ({
            ...current,
            [account.id]: {
              ...emptyAccountDraft(),
              ...current[account.id],
              mode: setting.mode,
              host: setting.proxy?.host ?? "",
              port: setting.proxy?.port ? String(setting.proxy.port) : "",
              loading: false,
              error: null,
            },
          }));
        })
        .catch((err) => {
          if (cancelled || !accountIds.has(account.id)) return;
          setAccountProxyDrafts((current) => ({
            ...current,
            [account.id]: {
              ...emptyAccountDraft(),
              ...current[account.id],
              loading: false,
              saving: false,
              error: extractErrorMessage(err),
            },
          }));
        });
    });

    return () => {
      cancelled = true;
    };
  }, [accountIdsKey]);

  const saveGlobalProxy = useCallback(async () => {
    setProxySaving(true);
    setProxyError(null);
    const trimmedHost = proxyHost.trim();
    const trimmedPort = proxyPort.trim();
    try {
      const normalizedPort = parseProxyPort(trimmedPort, t(PROXY_PORT_ERROR_KEY, "Port must be an integer between 1 and 65535."));
      await updateGlobalProxy(trimmedHost || undefined, normalizedPort);
      addToast({
        message: t("settings.globalProxySaved", "Global proxy saved"),
        type: "success",
      });
    } catch (err) {
      setProxyError(extractErrorMessage(err));
    } finally {
      setProxySaving(false);
    }
  }, [addToast, proxyHost, proxyPort, t]);

  const updateAccountDraft = useCallback((accountId: string, patch: Partial<AccountProxyDraft>) => {
    setAccountProxyDrafts((current) => ({
      ...current,
      [accountId]: {
        ...emptyAccountDraft(),
        ...current[accountId],
        ...patch,
      },
    }));
  }, []);

  const saveAccountProxy = useCallback(async (account: Account) => {
    const draft = accountProxyDrafts[account.id] ?? emptyAccountDraft();
    updateAccountDraft(account.id, { saving: true, error: null });
    const custom = draft.mode === "custom";
    const trimmedHost = custom ? draft.host.trim() : "";
    const trimmedPort = custom ? draft.port.trim() : "";
    try {
      const normalizedPort = parseProxyPort(trimmedPort, t(PROXY_PORT_ERROR_KEY, "Port must be an integer between 1 and 65535."));
      if (usesMailProxySetting(account.provider)) {
        await updateAccountProxySetting(
          account.id,
          draft.mode,
          custom ? trimmedHost || undefined : undefined,
          custom ? normalizedPort : undefined,
        );
      } else {
        await updateOAuthAccountProxySetting(
          account.id,
          draft.mode,
          custom ? trimmedHost || undefined : undefined,
          custom ? normalizedPort : undefined,
        );
      }
      addToast({
        message: t("settings.accountProxySaved", "Account proxy saved"),
        type: "success",
      });
      updateAccountDraft(account.id, { saving: false, error: null });
    } catch (err) {
      updateAccountDraft(account.id, { saving: false, error: extractErrorMessage(err) });
    }
  }, [accountProxyDrafts, addToast, t, updateAccountDraft]);

  return (
    <div>
      <SettingsSection
        title={t("settings.globalProxy", "Global Proxy")}
        description={t("settings.globalProxyDesc", "Used by translation, OAuth, Gmail/Outlook API requests, IMAP, POP3, and SMTP when an account does not define its own SOCKS5 proxy.")}
      >
        <div role="group" aria-label={t("settings.globalProxy", "Global Proxy")} className="settings-row">
          <label className="field">
            <span className="field__label">{t("settings.globalProxyHost", "SOCKS5 Proxy")}</span>
            <Input
              aria-label={t("settings.globalProxyHost", "SOCKS5 Proxy")}
              type="text"
              value={proxyHost}
              onChange={(e) => setProxyHost(e.target.value)}
              placeholder="127.0.0.1"
              disabled={proxyLoading || proxySaving}
            />
          </label>
          <label className="field field--narrow">
            <span className="field__label">{t("settings.globalProxyPort", "Port")}</span>
            <Input
              aria-label={t("settings.globalProxyPort", "Port")}
              type="number" min={1} max={65535} step={1} inputMode="numeric"
              value={proxyPort}
              onChange={(e) => setProxyPort(e.target.value)}
              placeholder="7890"
              disabled={proxyLoading || proxySaving}
            />
          </label>
          <Button
            variant="primary"
            onClick={saveGlobalProxy}
            disabled={proxyLoading}
            loading={proxySaving}
          >
            {proxySaving ? t("common.saving", "Saving...") : t("common.save", "Save")}
          </Button>
        </div>
        {proxyError && <p className="settings-error">{proxyError}</p>}
      </SettingsSection>

      <SettingsSection
        title={t("settings.accountProxies", "Account Proxies")}
        description={t("settings.accountProxiesDesc", "Override the global SOCKS5 proxy for individual accounts.")}
      >
        {proxyAccounts.length === 0 ? (
          <SettingsEmpty id="account-proxy-list">
            {t("settings.noAccounts", "No accounts added yet")}
          </SettingsEmpty>
        ) : (
          <SettingsList id="account-proxy-list">
            {proxyAccounts.map((account) => {
              const draft = accountProxyDrafts[account.id] ?? {
                ...emptyAccountDraft(),
                loading: true,
              };
              const fieldsDisabled = draft.loading || draft.saving;
              const showProxyFields = draft.mode === "custom";
              return (
                <div
                  key={account.id}
                  role="group"
                  aria-label={t("settings.accountProxyFor", { email: account.email, defaultValue: "Proxy for {{email}}" })}
                  className="settings-list__row settings-list__row--stacked"
                >
                  <div style={{ display: "flex", alignItems: "flex-start", gap: "var(--space-3)" }}>
                    <div className="settings-list__main">
                      <div className="settings-list__title">{account.display_name || account.email}</div>
                      <div className="settings-list__meta">{account.email}</div>
                    </div>
                    <StatusChip>{providerLabel(account.provider)}</StatusChip>
                  </div>
                  <OptionCardGroup label={t("settings.accountProxyMode", "Proxy mode")} columns={3}>
                    {modeOptions.map((option) => (
                      <OptionCard
                        key={option.mode}
                        selected={draft.mode === option.mode}
                        onClick={() => updateAccountDraft(account.id, { mode: option.mode })}
                        disabled={fieldsDisabled}
                        title={option.label}
                      />
                    ))}
                  </OptionCardGroup>
                  {showProxyFields && (
                    <div className="settings-row">
                      <label className="field">
                        <span className="field__label">{t("settings.globalProxyHost", "SOCKS5 Proxy")}</span>
                        <Input
                          aria-label={t("settings.globalProxyHost", "SOCKS5 Proxy")}
                          type="text"
                          value={draft.host}
                          onChange={(e) => updateAccountDraft(account.id, { host: e.target.value })}
                          placeholder="127.0.0.1"
                          disabled={fieldsDisabled}
                        />
                      </label>
                      <label className="field field--narrow">
                        <span className="field__label">{t("settings.globalProxyPort", "Port")}</span>
                        <Input
                          aria-label={t("settings.globalProxyPort", "Port")}
                          type="number" min={1} max={65535} step={1} inputMode="numeric"
                          value={draft.port}
                          onChange={(e) => updateAccountDraft(account.id, { port: e.target.value })}
                          placeholder="7890"
                          disabled={fieldsDisabled}
                        />
                      </label>
                    </div>
                  )}
                  <div className="settings-actions settings-actions--end">
                    <Button
                      size="sm"
                      aria-label={t("settings.saveAccountProxy", "Save account proxy")}
                      onClick={() => saveAccountProxy(account)}
                      disabled={draft.loading}
                      loading={draft.saving}
                    >
                      {draft.saving ? t("common.saving", "Saving...") : t("common.save", "Save")}
                    </Button>
                  </div>
                  {draft.error && <p className="settings-error">{draft.error}</p>}
                </div>
              );
            })}
          </SettingsList>
        )}
      </SettingsSection>
    </div>
  );
}
