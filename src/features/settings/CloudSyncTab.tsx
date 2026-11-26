import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  inputStyle as baseInputStyle,
  labelStyle as baseLabelStyle,
  fieldGroupStyle,
} from "../../styles/form";
import { useQueryClient } from "@tanstack/react-query";
import ConfirmDialog from "@/components/ConfirmDialog";
import {
  testWebdavConnection,
  backupToWebdav,
  exportBackupFile,
  importBackupFile,
  previewBackupFile,
  previewWebdavBackup,
  restoreFromWebdav,
  saveAutoBackupConfig,
  loadAutoBackupConfig,
  type BackupPreview,
} from "../../lib/api";
import { extractErrorMessage as errorMessage } from "@/lib/extractErrorMessage";
import { profileLocalStorage } from "@/lib/profileStorage";
import { formatDateTime } from "@/lib/formatDate";

const LAST_BACKUP_KEY = "pebble-cloud-sync-last-backup";

/** Last-backup time is stored as ISO; older builds stored a preformatted string. */
function displayLastBackup(value: string): string {
  return /^\d{4}-\d{2}-\d{2}T/.test(value) ? formatDateTime(new Date(value)) : value;
}
/** Must match `MIN_PASSPHRASE_CHARS` in pebble-crypto. */
const MIN_SECRET_PASSPHRASE_CHARS = 12;

type RestoreRequest =
  | { source: "webdav"; preview: BackupPreview }
  | { source: "file"; preview: BackupPreview; data: string };

const labelStyle: React.CSSProperties = {
  ...baseLabelStyle,
  fontWeight: 500,
};

const inputStyle: React.CSSProperties = {
  ...baseInputStyle,
  padding: "8px 10px",
  backgroundColor: "var(--color-bg-secondary)",
};

const buttonStyle: React.CSSProperties = {
  padding: "8px 18px",
  fontSize: "13px",
  fontWeight: 500,
  border: "none",
  borderRadius: "6px",
  cursor: "pointer",
};

export default function CloudSyncTab() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const [url, setUrl] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [includeSecrets, setIncludeSecrets] = useState(false);
  const [secretPassphrase, setSecretPassphrase] = useState("");
  // The backend never returns saved secrets, only whether they exist. A blank
  // field then means "keep the saved value".
  const [hasStoredPassword, setHasStoredPassword] = useState(false);
  // The saved WebDAV password is only reused for the server it was saved for.
  const [storedServer, setStoredServer] = useState<{ url: string; username: string } | null>(null);
  const [hasStoredPassphrase, setHasStoredPassphrase] = useState(false);
  const [storedPassphraseIsWeak, setStoredPassphraseIsWeak] = useState(false);

  const [statusMsg, setStatusMsg] = useState("");
  const [statusType, setStatusType] = useState<"success" | "error" | "">("");
  const [testing, setTesting] = useState(false);
  const [backing, setBacking] = useState(false);
  const [restoring, setRestoring] = useState(false);

  const [lastBackup, setLastBackup] = useState<string | null>(() =>
    profileLocalStorage.getItem(LAST_BACKUP_KEY),
  );

  const [autoEnabled, setAutoEnabled] = useState(false);
  const [autoInterval, setAutoInterval] = useState(60);
  const [autoLoaded, setAutoLoaded] = useState(false);
  const [savingAutoConfig, setSavingAutoConfig] = useState(false);

  useEffect(() => {
    loadAutoBackupConfig().then((config) => {
      if (config) {
        setUrl(config.url);
        setUsername(config.username);
        setHasStoredPassword(config.has_password);
        setStoredServer({ url: config.url, username: config.username });
        setAutoEnabled(config.enabled);
        setAutoInterval(config.interval_minutes);
        if (config.has_secret_passphrase) {
          setIncludeSecrets(true);
          setHasStoredPassphrase(true);
          setStoredPassphraseIsWeak(Boolean(config.secret_passphrase_is_weak));
        }
      }
      setAutoLoaded(true);
    }).catch(() => setAutoLoaded(true));
  }, []);

  const storedPasswordApplies =
    hasStoredPassword &&
    storedServer !== null &&
    storedServer.url === url &&
    storedServer.username === username;
  const hasPassword = Boolean(password) || storedPasswordApplies;
  const hasPassphrase = Boolean(secretPassphrase.trim()) || hasStoredPassphrase;

  function passphraseTooShort() {
    const entered = secretPassphrase.trim();
    return entered.length > 0 && [...entered].length < MIN_SECRET_PASSPHRASE_CHARS;
  }

  function passphraseTooShortMessage() {
    return t(
      "cloudSync.secretPassphraseTooShort",
      "The backup encryption password must be at least 12 characters.",
    );
  }

  async function handleSaveAutoBackupConfig() {
    setStatusMsg("");
    setStatusType("");

    if (
      autoEnabled &&
      (!url.trim() || !username.trim() || !hasPassword)
    ) {
      setStatusMsg(
        t(
          "cloudSync.autoBackupCredentialsRequired",
          "Enter a WebDAV URL, username, and password before enabling automatic backup.",
        ),
      );
      setStatusType("error");
      return;
    }

    if (includeSecrets && !hasPassphrase) {
      setStatusMsg(
        t(
          "cloudSync.autoBackupSecretPassphraseRequired",
          "Enter an encryption password before including secrets.",
        ),
      );
      setStatusType("error");
      return;
    }

    if (includeSecrets && passphraseTooShort()) {
      setStatusMsg(passphraseTooShortMessage());
      setStatusType("error");
      return;
    }

    setSavingAutoConfig(true);
    try {
      await saveAutoBackupConfig({
        url,
        username,
        password,
        secret_passphrase: includeSecrets ? secretPassphrase : null,
        interval_minutes: autoInterval,
        enabled: autoEnabled,
      });
      setHasStoredPassword(Boolean(password) || storedPasswordApplies);
      setStoredServer({ url, username });
      if (includeSecrets && secretPassphrase.trim()) {
        setHasStoredPassphrase(true);
        setStoredPassphraseIsWeak(false);
      }
      if (!includeSecrets) {
        setHasStoredPassphrase(false);
        setStoredPassphraseIsWeak(false);
      }
      setStatusMsg(
        t(
          "cloudSync.autoBackupConfigSaved",
          "Automatic backup configuration saved",
        ),
      );
      setStatusType("success");
    } catch (err: unknown) {
      setStatusMsg(
        t("cloudSync.autoBackupConfigSaveFailed", {
          error: errorMessage(err),
          defaultValue: "Couldn't save automatic backup settings: {{error}}",
        }),
      );
      setStatusType("error");
    } finally {
      setSavingAutoConfig(false);
    }
  }

  async function handleTestConnection() {
    setTesting(true);
    setStatusMsg("");
    try {
      await testWebdavConnection(url, username, password);
      setStatusMsg(t("cloudSync.connectionSuccess"));
      setStatusType("success");
    } catch (err: unknown) {
      setStatusMsg(
        `${t("cloudSync.connectionFailed")}: ${errorMessage(err)}`,
      );
      setStatusType("error");
    } finally {
      setTesting(false);
    }
  }

  async function handleBackup() {
    setBacking(true);
    setStatusMsg("");
    try {
      requireSecretPassphrase();
      await backupToWebdav(
        url,
        username,
        password,
        includeSecrets ? secretPassphrase : undefined,
      );
      const now = new Date().toISOString();
      profileLocalStorage.setItem(LAST_BACKUP_KEY, now);
      setLastBackup(now);
      setStatusMsg(t("cloudSync.backupSuccess"));
      setStatusType("success");
    } catch (err: unknown) {
      setStatusMsg(
        t("cloudSync.backupFailed", { error: errorMessage(err) }),
      );
      setStatusType("error");
    } finally {
      setBacking(false);
    }
  }

  const [restoreRequest, setRestoreRequest] = useState<RestoreRequest | null>(null);
  const [pendingFileImport, setPendingFileImport] = useState<RestoreRequest | null>(null);

  useEffect(() => {
    if (!pendingFileImport || restoreRequest || !secretPassphrase.trim()) return;
    setStatusMsg("");
    setRestoreRequest(pendingFileImport);
    setPendingFileImport(null);
  }, [pendingFileImport, restoreRequest, secretPassphrase]);

  /** Validates the passphrase used to create a new backup. */
  function requireSecretPassphrase() {
    if (!includeSecrets) return;
    if (!hasPassphrase) {
      throw new Error(
        t(
          "cloudSync.secretPassphraseRequired",
          "Enter a backup encryption password to include account passwords, OAuth tokens, and API keys.",
        ),
      );
    }
    if (passphraseTooShort()) {
      throw new Error(passphraseTooShortMessage());
    }
  }

  function downloadBackupJson(data: string) {
    const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
    const filename = `pebble-settings-backup-${timestamp}.json`;
    const blob = new Blob([data], { type: "application/json;charset=utf-8" });
    const href = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = href;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(href);
  }

  async function handleRestoreClick() {
    setRestoring(true);
    setStatusMsg("");
    try {
      const preview = await previewWebdavBackup(url, username, password);
      if (preview.has_encrypted_secrets && !hasPassphrase) {
        setStatusMsg(
          t(
            "cloudSync.restoreSecretPassphraseRequired",
            "This backup contains encrypted secrets. Enter the backup encryption password before restoring.",
          ),
        );
        setStatusType("error");
        return;
      }
      setRestoreRequest({ source: "webdav", preview });
    } catch (err: unknown) {
      setStatusMsg(
        t("cloudSync.restoreFailed", { error: errorMessage(err) }),
      );
      setStatusType("error");
    } finally {
      setRestoring(false);
    }
  }

  async function handleExportFile() {
    setBacking(true);
    setStatusMsg("");
    try {
      requireSecretPassphrase();
      const data = await exportBackupFile(includeSecrets ? secretPassphrase : undefined);
      downloadBackupJson(data);
      const now = new Date().toISOString();
      profileLocalStorage.setItem(LAST_BACKUP_KEY, now);
      setLastBackup(now);
      setStatusMsg(t("cloudSync.exportSuccess", "Backup file exported"));
      setStatusType("success");
    } catch (err: unknown) {
      setStatusMsg(t("cloudSync.exportFailed", { error: errorMessage(err) }));
      setStatusType("error");
    } finally {
      setBacking(false);
    }
  }

  async function handleImportFile(file: File) {
    setRestoring(true);
    setStatusMsg("");
    try {
      const data = await file.text();
      const preview = await previewBackupFile(data);
      const request: RestoreRequest = { source: "file", preview, data };
      if (preview.has_encrypted_secrets && !hasPassphrase) {
        setPendingFileImport(request);
        setStatusMsg(
          t(
            "cloudSync.restoreSecretPassphraseRequired",
            "This backup contains encrypted secrets. Enter the backup encryption password before restoring.",
          ),
        );
        setStatusType("error");
        return;
      }
      setPendingFileImport(null);
      setRestoreRequest(request);
    } catch (err: unknown) {
      setStatusMsg(t("cloudSync.importFailed", { error: errorMessage(err) }));
      setStatusType("error");
      setPendingFileImport(null);
    } finally {
      setRestoring(false);
      if (fileInputRef.current) fileInputRef.current.value = "";
    }
  }

  async function doRestore(request: RestoreRequest) {
    setRestoring(true);
    setStatusMsg("");
    try {
      if (request.source === "file") {
        await importBackupFile(
          request.data,
          request.preview.has_encrypted_secrets ? secretPassphrase : undefined,
        );
      } else {
        await restoreFromWebdav(
          url,
          username,
          password,
          request.preview.has_encrypted_secrets ? secretPassphrase : undefined,
        );
      }
      setStatusMsg(
        request.preview.has_encrypted_secrets
          ? t(
              "cloudSync.restoreSuccessWithSecrets",
              "Backup restored with saved credentials. Existing OAuth connections and mailboxes with local data were preserved.",
            )
          : t("cloudSync.restoreSuccess"),
      );
      setStatusType("success");
      // Refresh all cached data to reflect restored state
      await queryClient.invalidateQueries();
    } catch (err: unknown) {
      setStatusMsg(
        t("cloudSync.restoreFailed", { error: errorMessage(err) }),
      );
      setStatusType("error");
    } finally {
      setRestoring(false);
    }
  }

  const anyLoading = testing || backing || restoring || savingAutoConfig;

  function restorePreviewMessage(preview: BackupPreview) {
    const lines = [
      t("cloudSync.restorePreviewHeader", "Backup contents to restore:"),
      t("cloudSync.restorePreviewSchema", "Schema version: {{version}}", { version: preview.version }),
      t("cloudSync.restorePreviewExported", "Exported: {{date}}", {
        date: formatDateTime(preview.exported_at),
      }),
      t("cloudSync.restorePreviewAccounts", "Accounts: {{count}}", { count: preview.account_count }),
      t("cloudSync.restorePreviewRules", "Rules: {{count}}", { count: preview.rule_count }),
      t("cloudSync.restorePreviewKanban", "Kanban cards: {{count}}", { count: preview.kanban_card_count }),
      t("cloudSync.restorePreviewKanbanNotes", "Kanban notes: {{count}}", { count: preview.kanban_note_count }),
      t("cloudSync.restorePreviewContacts", "Contacts: {{count}}", { count: preview.contact_count }),
    ];

    if (preview.has_encrypted_secrets) {
      lines.push(
        t("cloudSync.restorePreviewEncryptedSecrets", "Encrypted account secrets: {{count}}", {
          count: preview.secret_account_count,
        }),
      );
      if (preview.has_translate_secret) {
        lines.push(
          t("cloudSync.restorePreviewTranslateSecret", "Encrypted translation API keys: included"),
        );
      }
    }

    lines.push(
      t("cloudSync.restorePreviewSize", "Size: {{kb}} KB", {
        kb: (preview.size_bytes / 1024).toFixed(1),
      }),
      "",
      t(
        "cloudSync.restoreConfirm",
        "This will replace local rules, Kanban cards/notes, and contacts; merge account metadata; and restore encrypted secrets when present. Continue?",
      ),
    );

    return lines.join("\n");
  }

  return (
    <div>
      <h2
        style={{
          fontSize: "18px",
          fontWeight: 600,
          color: "var(--color-text-primary)",
          marginTop: 0,
          marginBottom: "20px",
        }}
      >
        {t("cloudSync.title", "Settings Backup")}
      </h2>

      <p
        style={{
          marginTop: "-8px",
          marginBottom: "18px",
          fontSize: "13px",
          lineHeight: 1.5,
          color: "var(--color-text-secondary)",
          maxWidth: "640px",
        }}
      >
        {t(
          "cloudSync.description",
          "Back up contacts, rules, Kanban cards and notes, and account metadata to WebDAV. Account passwords, OAuth tokens, and API keys can be included with a separate backup encryption password.",
        )}
        {" "}
        <span style={{ color: "var(--color-warning, #e67e22)" }}>
          {t(
            "cloudSync.encryptionWarning",
            "Note: regular settings are uploaded as JSON. Secrets are encrypted only when you enable the option below. Ensure your WebDAV server is trusted.",
          )}
        </span>
      </p>

      <p
        style={{
          marginTop: "-8px",
          marginBottom: "18px",
          fontSize: "13px",
          lineHeight: 1.5,
          color: "var(--color-text-secondary)",
          maxWidth: "640px",
          padding: "8px 12px",
          background: "var(--color-bg-secondary)",
          borderRadius: "6px",
          borderLeft: "3px solid var(--color-accent)",
        }}
      >
        {t(
          "cloudSync.scopeNotice",
          "WebDAV backup includes contacts, settings, rules, Kanban cards, and Kanban notes. Optional encrypted secrets include account passwords, OAuth tokens, and translation API keys. Message bodies and attachments are not included unless you saved text into a Kanban note.",
        )}
      </p>

      <div style={fieldGroupStyle}>
        <label htmlFor="settings-backup-webdav-url" style={labelStyle}>{t("cloudSync.webdavUrl")}</label>
        <input
          id="settings-backup-webdav-url"
          name="webdav_url"
          type="url"
          style={inputStyle}
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          disabled={savingAutoConfig}
          placeholder="https://dav.example.com/remote.php/dav/files/user/"
          autoComplete="url"
        />
      </div>

      <div style={fieldGroupStyle}>
        <label htmlFor="settings-backup-username" style={labelStyle}>{t("cloudSync.username")}</label>
        <input
          id="settings-backup-username"
          name="webdav_username"
          style={inputStyle}
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          disabled={savingAutoConfig}
          placeholder={t("cloudSync.username")}
          autoComplete="username"
        />
      </div>

      <div style={fieldGroupStyle}>
        <label htmlFor="settings-backup-password" style={labelStyle}>{t("cloudSync.password")}</label>
        <input
          id="settings-backup-password"
          name="webdav_password"
          style={inputStyle}
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          disabled={savingAutoConfig}
          placeholder={
            storedPasswordApplies
              ? t("cloudSync.savedSecretPlaceholder", "Saved — leave blank to keep")
              : t("cloudSync.password")
          }
          autoComplete="current-password"
        />
      </div>

      <div style={{ ...fieldGroupStyle, display: "flex", alignItems: "flex-start", gap: "8px" }}>
        <input
          id="settings-backup-include-secrets"
          name="include_secrets"
          type="checkbox"
          checked={includeSecrets}
          onChange={(e) => setIncludeSecrets(e.target.checked)}
          disabled={savingAutoConfig}
          style={{ marginTop: "3px" }}
        />
        <label
          htmlFor="settings-backup-include-secrets"
          style={{ fontSize: "13px", lineHeight: 1.5, color: "var(--color-text-primary)" }}
        >
          {t("cloudSync.includeSecrets", "Include account passwords, OAuth tokens, and API keys")}
          <span style={{ display: "block", color: "var(--color-text-secondary)", fontSize: "12px" }}>
            {t(
              "cloudSync.includeSecretsDesc",
              "Secrets are encrypted with the password below before upload. You will need the same password to restore them on another device.",
            )}
          </span>
        </label>
      </div>

      <div style={fieldGroupStyle}>
        <label htmlFor="settings-backup-secret-passphrase" style={labelStyle}>
          {t("cloudSync.secretPassphrase", "Backup encryption password")}
        </label>
        <input
          id="settings-backup-secret-passphrase"
          name="secret_passphrase"
          style={inputStyle}
          type="password"
          value={secretPassphrase}
          onChange={(e) => setSecretPassphrase(e.target.value)}
          disabled={savingAutoConfig}
          placeholder={
            hasStoredPassphrase
              ? t("cloudSync.savedSecretPlaceholder", "Saved — leave blank to keep")
              : t("cloudSync.secretPassphrasePlaceholder", "Required for backing up or restoring secrets")
          }
          autoComplete="new-password"
        />
        {includeSecrets && storedPassphraseIsWeak && !secretPassphrase && (
          <p
            role="note"
            style={{ margin: "6px 0 0", fontSize: "12px", color: "var(--color-warning, #e67e22)" }}
          >
            {t(
              "cloudSync.weakPassphraseHint",
              "Your saved backup encryption password is shorter than 12 characters. Backups still work, but please set a stronger password.",
            )}
          </p>
        )}
      </div>

      {/* Action buttons */}
      <input
        ref={fileInputRef}
        type="file"
        accept="application/json,.json"
        style={{ display: "none" }}
        onChange={(event) => {
          const file = event.currentTarget.files?.[0];
          if (file) {
            void handleImportFile(file);
          }
        }}
      />

      <div style={{ display: "flex", flexWrap: "wrap", gap: "10px", marginTop: "20px" }}>
        <button
          style={{
            ...buttonStyle,
            background: "var(--color-accent)",
            color: "#fff",
            opacity: anyLoading ? 0.6 : 1,
          }}
          onClick={handleExportFile}
          disabled={anyLoading}
        >
          {backing ? t("common.saving") : t("cloudSync.exportFile", "Export File")}
        </button>
        <button
          style={{
            ...buttonStyle,
            background: "var(--color-bg-hover)",
            color: "var(--color-text-primary)",
            opacity: anyLoading ? 0.6 : 1,
          }}
          onClick={() => fileInputRef.current?.click()}
          disabled={anyLoading}
        >
          {restoring ? t("common.loading") : t("cloudSync.importFile", "Import File")}
        </button>
        <button
          style={{
            ...buttonStyle,
            background: "var(--color-bg-hover)",
            color: "var(--color-text-primary)",
            opacity: anyLoading ? 0.6 : 1,
          }}
          onClick={handleTestConnection}
          disabled={anyLoading}
        >
          {testing ? t("common.testing") : t("cloudSync.testConnection")}
        </button>
        <button
          style={{
            ...buttonStyle,
            background: "var(--color-accent)",
            color: "#fff",
            opacity: anyLoading ? 0.6 : 1,
          }}
          onClick={handleBackup}
          disabled={anyLoading}
        >
          {backing ? t("common.saving") : t("cloudSync.backup", "Backup Settings")}
        </button>
        <button
          style={{
            ...buttonStyle,
            background: "var(--color-bg-hover)",
            color: "var(--color-text-primary)",
            opacity: anyLoading ? 0.6 : 1,
          }}
          onClick={handleRestoreClick}
          disabled={anyLoading}
        >
          {restoring ? t("common.loading") : t("cloudSync.restore", "Restore Settings Backup")}
        </button>
      </div>

      <div
        style={{
          marginTop: "12px",
          fontSize: "12px",
          lineHeight: 1.5,
          color: "var(--color-text-secondary)",
          maxWidth: "640px",
        }}
      >
        {t(
          "cloudSync.restoreNotice",
          "Restoring without the backup encryption password is partial. Enter it to restore account passwords, OAuth tokens, and translation API keys when the backup contains them.",
        )}
      </div>

      {/* Restore confirmation with backup preview */}
      {restoreRequest && (
        <ConfirmDialog
          title={t("cloudSync.restore", "Restore Settings Backup")}
          message={restorePreviewMessage(restoreRequest.preview)}
          confirmLabel={t("cloudSync.restoreConfirmAction", "Restore and overwrite")}
          destructive
          onCancel={() => setRestoreRequest(null)}
          onConfirm={() => {
            const request = restoreRequest;
            setRestoreRequest(null);
            doRestore(request);
          }}
        />
      )}

      {/* Last backup timestamp */}
      {lastBackup && (
        <div
          style={{
            marginTop: "14px",
            fontSize: "12px",
            color: "var(--color-text-secondary)",
          }}
        >
          {t("cloudSync.lastBackupAt", { time: displayLastBackup(lastBackup) })}
        </div>
      )}

      {/* Auto backup */}
      {autoLoaded && (
        <div style={{ marginTop: "24px", padding: "16px", border: "1px solid var(--color-border)", borderRadius: "8px", background: "var(--color-bg-secondary)" }}>
          <h3 style={{ fontSize: "14px", fontWeight: 600, color: "var(--color-text-primary)", margin: "0 0 12px" }}>
            {t("cloudSync.autoBackup", "Automatic Backup")}
          </h3>
          <div style={{ display: "flex", alignItems: "center", gap: "10px", marginBottom: "10px" }}>
            <label style={{ display: "flex", alignItems: "center", gap: "8px", fontSize: "13px", color: "var(--color-text-primary)", cursor: "pointer" }}>
              <input
                type="checkbox"
                checked={autoEnabled}
                onChange={(e) => setAutoEnabled(e.target.checked)}
                disabled={
                  savingAutoConfig ||
                  (!autoEnabled &&
                    (!url.trim() || !username.trim() || !hasPassword))
                }
              />
              {t("cloudSync.autoBackupEnable", "Enable automatic WebDAV backup")}
            </label>
          </div>
          {autoEnabled && (
            <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
              <label style={{ fontSize: "12px", color: "var(--color-text-secondary)" }}>
                {t("cloudSync.autoBackupInterval", "Interval")}:
              </label>
              <select
                value={autoInterval}
                onChange={(e) => setAutoInterval(Number(e.target.value))}
                disabled={savingAutoConfig}
                style={{ ...inputStyle, width: "auto", padding: "4px 8px" }}
              >
                <option value={30}>30 min</option>
                <option value={60}>1 h</option>
                <option value={180}>3 h</option>
                <option value={360}>6 h</option>
                <option value={720}>12 h</option>
                <option value={1440}>24 h</option>
              </select>
            </div>
          )}
          {!url && <p style={{ fontSize: "12px", color: "var(--color-text-secondary)", margin: "8px 0 0" }}>{t("cloudSync.autoBackupFillFields", "Fill in WebDAV credentials above to enable auto-backup.")}</p>}
          <button
            type="button"
            style={{
              ...buttonStyle,
              marginTop: "12px",
              background: "var(--color-accent)",
              color: "#fff",
              opacity: anyLoading ? 0.6 : 1,
            }}
            onClick={handleSaveAutoBackupConfig}
            disabled={anyLoading}
          >
            {savingAutoConfig
              ? t("common.saving")
              : t(
                  "cloudSync.saveAutoBackupConfig",
                  "Save automatic backup settings",
                )}
          </button>
        </div>
      )}

      {/* Status message */}
      {statusMsg && (
        <div
          role={statusType === "error" ? "alert" : "status"}
          aria-live="polite"
          style={{
            marginTop: "14px",
            padding: "10px 14px",
            borderRadius: "6px",
            fontSize: "13px",
            background:
              statusType === "success"
                ? "var(--color-bg-hover)"
                : "rgba(220, 53, 69, 0.1)",
            color:
              statusType === "success"
                ? "var(--color-text-primary)"
                : "#dc3545",
            border: `1px solid ${statusType === "success" ? "var(--color-border)" : "rgba(220, 53, 69, 0.3)"}`,
          }}
        >
          {statusMsg}
        </div>
      )}
    </div>
  );
}
