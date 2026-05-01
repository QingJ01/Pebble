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
import { useAccountsQuery } from "@/hooks/queries";
import { useToastStore } from "@/stores/toast.store";

type AccountProxyDraft = {
  mode: AccountProxyMode;
  host: string;
  port: string;
  loading: boolean;
  saving: boolean;
  error: string | null;
};

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
      const loadProxy =
        account.provider === "imap" ? getAccountProxySetting : getOAuthAccountProxySetting;
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
    const parsedPort = trimmedPort ? Number.parseInt(trimmedPort, 10) : undefined;
    const normalizedPort =
      parsedPort === undefined || Number.isNaN(parsedPort) ? undefined : parsedPort;
    try {
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
    const parsedPort = trimmedPort ? Number.parseInt(trimmedPort, 10) : undefined;
    const normalizedPort =
      parsedPort === undefined || Number.isNaN(parsedPort) ? undefined : parsedPort;
    try {
      if (account.provider === "imap") {
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
      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "8px" }}>
        {t("settings.globalProxy", "Global Proxy")}
      </h3>
      <p style={{ fontSize: "12px", color: "var(--color-text-secondary)", marginBottom: "12px", marginTop: 0 }}>
        {t("settings.globalProxyDesc", "Used by OAuth, Gmail/Outlook API requests, IMAP, and SMTP when an account does not define its own SOCKS5 proxy.")}
      </p>
      <div
        role="group"
        aria-label={t("settings.globalProxy", "Global Proxy")}
        style={{ display: "flex", gap: "8px", flexWrap: "wrap", alignItems: "flex-end" }}
      >
        <label style={{ display: "grid", gap: "6px", fontSize: "12px", color: "var(--color-text-secondary)", flex: "1 1 220px", minWidth: 0 }}>
          {t("settings.globalProxyHost", "SOCKS5 Proxy")}
          <input
            aria-label={t("settings.globalProxyHost", "SOCKS5 Proxy")}
            type="text"
            value={proxyHost}
            onChange={(e) => setProxyHost(e.target.value)}
            placeholder="127.0.0.1"
            disabled={proxyLoading || proxySaving}
            style={{
              width: "100%",
              minWidth: 0,
              boxSizing: "border-box",
              padding: "8px 10px",
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              backgroundColor: "var(--color-bg-primary)",
              color: "var(--color-text-primary)",
              fontSize: "13px",
            }}
          />
        </label>
        <label style={{ display: "grid", gap: "6px", fontSize: "12px", color: "var(--color-text-secondary)", width: "110px", minWidth: 0 }}>
          {t("settings.globalProxyPort", "Port")}
          <input
            aria-label={t("settings.globalProxyPort", "Port")}
            type="number"
            value={proxyPort}
            onChange={(e) => setProxyPort(e.target.value)}
            placeholder="7890"
            disabled={proxyLoading || proxySaving}
            style={{
              width: "100%",
              minWidth: 0,
              boxSizing: "border-box",
              padding: "8px 10px",
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              backgroundColor: "var(--color-bg-primary)",
              color: "var(--color-text-primary)",
              fontSize: "13px",
            }}
          />
        </label>
        <button
          type="button"
          onClick={saveGlobalProxy}
          disabled={proxyLoading || proxySaving}
          style={{
            padding: "8px 12px",
            borderRadius: "6px",
            border: "1px solid var(--color-border)",
            backgroundColor: "var(--color-bg-hover)",
            color: "var(--color-text-primary)",
            cursor: proxyLoading || proxySaving ? "default" : "pointer",
            fontSize: "13px",
            fontWeight: 500,
          }}
        >
          {proxySaving ? t("common.saving", "Saving...") : t("common.save", "Save")}
        </button>
      </div>
      {proxyError && (
        <p style={{ fontSize: "12px", color: "var(--color-error)", marginTop: "8px", marginBottom: 0 }}>
          {proxyError}
        </p>
      )}

      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "8px", marginTop: "32px" }}>
        {t("settings.accountProxies", "Account Proxies")}
      </h3>
      <p style={{ fontSize: "12px", color: "var(--color-text-secondary)", marginBottom: "12px", marginTop: 0 }}>
        {t("settings.accountProxiesDesc", "Override the global SOCKS5 proxy for individual accounts.")}
      </p>

      {proxyAccounts.length === 0 ? (
        <p id="account-proxy-list" style={{ fontSize: "13px", color: "var(--color-text-secondary)", margin: 0 }}>
          {t("settings.noAccounts", "No accounts added yet")}
        </p>
      ) : (
        <div id="account-proxy-list" style={{ display: "grid", gap: "10px" }}>
            {proxyAccounts.map((account) => {
              const draft = accountProxyDrafts[account.id] ?? {
                ...emptyAccountDraft(),
                loading: true,
              };
              const fieldsDisabled = draft.loading || draft.saving;
              const controlsDisabled = draft.loading || draft.saving;
              const showProxyFields = draft.mode === "custom";
              return (
                <div
                  key={account.id}
                  role="group"
                  aria-label={`${account.email} ${t("settings.accountProxy", "Account Proxy")}`}
                  style={{
                    border: "1px solid var(--color-border)",
                    borderRadius: "8px",
                    padding: "12px",
                    display: "grid",
                    gap: "10px",
                  }}
                >
                  <div style={{ display: "flex", justifyContent: "space-between", gap: "12px", alignItems: "baseline" }}>
                    <div style={{ minWidth: 0 }}>
                      <div style={{ fontSize: "13px", fontWeight: 600, color: "var(--color-text-primary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {account.display_name || account.email}
                      </div>
                      <div style={{ fontSize: "12px", color: "var(--color-text-secondary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {account.email}
                      </div>
                    </div>
                    <span style={{ fontSize: "11px", color: "var(--color-text-secondary)", textTransform: "capitalize" }}>
                      {account.provider}
                    </span>
                  </div>
                  <div role="group" aria-label={t("settings.accountProxyMode", "Proxy mode")} style={{ display: "flex", flexWrap: "wrap", gap: "8px" }}>
                    {modeOptions.map((option) => {
                      const selected = draft.mode === option.mode;
                      return (
                        <button
                          key={option.mode}
                          type="button"
                          aria-pressed={selected}
                          onClick={() => updateAccountDraft(account.id, { mode: option.mode })}
                          disabled={controlsDisabled}
                          style={{
                            flex: "1 1 180px",
                            minWidth: 0,
                            padding: "8px 10px",
                            borderRadius: "6px",
                            border: selected ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
                            backgroundColor: selected ? "var(--color-bg-hover)" : "transparent",
                            color: "var(--color-text-primary)",
                            cursor: controlsDisabled ? "default" : "pointer",
                            textAlign: "left",
                            fontSize: "13px",
                            fontWeight: selected ? 600 : 500,
                          }}
                        >
                          {option.label}
                        </button>
                      );
                    })}
                  </div>
                  {showProxyFields && (
                    <div style={{ display: "flex", gap: "8px", flexWrap: "wrap", alignItems: "flex-end" }}>
                      <label style={{ display: "grid", gap: "6px", fontSize: "12px", color: "var(--color-text-secondary)", flex: "1 1 220px", minWidth: 0 }}>
                        {t("settings.globalProxyHost", "SOCKS5 Proxy")}
                        <input
                          aria-label={t("settings.globalProxyHost", "SOCKS5 Proxy")}
                          type="text"
                          value={draft.host}
                          onChange={(e) => updateAccountDraft(account.id, { host: e.target.value })}
                          placeholder="127.0.0.1"
                          disabled={fieldsDisabled}
                          style={{
                            width: "100%",
                            minWidth: 0,
                            boxSizing: "border-box",
                            padding: "8px 10px",
                            borderRadius: "6px",
                            border: "1px solid var(--color-border)",
                            backgroundColor: "var(--color-bg-primary)",
                            color: "var(--color-text-primary)",
                            fontSize: "13px",
                            opacity: fieldsDisabled ? 0.7 : 1,
                          }}
                        />
                      </label>
                      <label style={{ display: "grid", gap: "6px", fontSize: "12px", color: "var(--color-text-secondary)", width: "110px", minWidth: 0 }}>
                        {t("settings.globalProxyPort", "Port")}
                        <input
                          aria-label={t("settings.globalProxyPort", "Port")}
                          type="number"
                          value={draft.port}
                          onChange={(e) => updateAccountDraft(account.id, { port: e.target.value })}
                          placeholder="7890"
                          disabled={fieldsDisabled}
                          style={{
                            width: "100%",
                            minWidth: 0,
                            boxSizing: "border-box",
                            padding: "8px 10px",
                            borderRadius: "6px",
                            border: "1px solid var(--color-border)",
                            backgroundColor: "var(--color-bg-primary)",
                            color: "var(--color-text-primary)",
                            fontSize: "13px",
                            opacity: fieldsDisabled ? 0.7 : 1,
                          }}
                        />
                      </label>
                    </div>
                  )}
                  <div style={{ display: "flex", justifyContent: "flex-end" }}>
                    <button
                      type="button"
                      aria-label={t("settings.saveAccountProxy", "Save account proxy")}
                      onClick={() => saveAccountProxy(account)}
                      disabled={draft.loading || draft.saving}
                      style={{
                        minWidth: "88px",
                        minHeight: "36px",
                        padding: "8px 16px",
                        borderRadius: "6px",
                        border: draft.loading || draft.saving ? "1px solid var(--color-border)" : "1px solid var(--color-accent)",
                        backgroundColor: draft.loading || draft.saving ? "var(--color-bg-hover)" : "var(--color-accent)",
                        color: draft.loading || draft.saving ? "var(--color-text-secondary)" : "#fff",
                        cursor: draft.loading || draft.saving ? "default" : "pointer",
                        fontSize: "13px",
                        fontWeight: 600,
                        boxShadow: draft.loading || draft.saving ? "none" : "0 1px 2px rgba(0, 0, 0, 0.12)",
                      }}
                    >
                      {draft.saving ? t("common.saving", "Saving...") : t("common.save", "Save")}
                    </button>
                  </div>
                  {draft.error && (
                    <p style={{ fontSize: "12px", color: "var(--color-error)", margin: 0 }}>
                      {draft.error}
                    </p>
                  )}
                </div>
              );
            })}
        </div>
      )}
    </div>
  );
}
