import { useState } from "react";
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
  previewWebdavBackup,
  restoreFromWebdav,
  type BackupPreview,
} from "../../lib/api";
import { extractErrorMessage as errorMessage } from "@/lib/extractErrorMessage";

const LAST_BACKUP_KEY = "pebble-cloud-sync-last-backup";

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

  const [url, setUrl] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");

  const [statusMsg, setStatusMsg] = useState("");
  const [statusType, setStatusType] = useState<"success" | "error" | "">("");
  const [testing, setTesting] = useState(false);
  const [backing, setBacking] = useState(false);
  const [restoring, setRestoring] = useState(false);

  const [lastBackup, setLastBackup] = useState<string | null>(() =>
    localStorage.getItem(LAST_BACKUP_KEY),
  );

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
      await backupToWebdav(url, username, password);
      const now = new Date().toLocaleString();
      localStorage.setItem(LAST_BACKUP_KEY, now);
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

  const [restorePreview, setRestorePreview] = useState<BackupPreview | null>(null);

  async function handleRestoreClick() {
    setRestoring(true);
    setStatusMsg("");
    try {
      const preview = await previewWebdavBackup(url, username, password);
      setRestorePreview(preview);
    } catch (err: unknown) {
      setStatusMsg(
        t("cloudSync.restoreFailed", { error: errorMessage(err) }),
      );
      setStatusType("error");
    } finally {
      setRestoring(false);
    }
  }

  async function doRestore() {
    setRestoring(true);
    setStatusMsg("");
    try {
      await restoreFromWebdav(url, username, password);
      setStatusMsg(t("cloudSync.restoreSuccess"));
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

  const anyLoading = testing || backing || restoring;

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
          "Back up rules, Kanban cards and notes, and account metadata to WebDAV. This does not sync mail data, attachments, or OAuth secrets.",
        )}
        {" "}
        <span style={{ color: "var(--color-warning, #e67e22)" }}>
          {t(
            "cloudSync.encryptionWarning",
            "Note: Backups are uploaded as unencrypted JSON. Ensure your WebDAV server is trusted.",
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
          "WebDAV backup includes settings, rules, Kanban cards, and Kanban notes. Message bodies and attachments are not included unless you saved text into a Kanban note.",
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
          placeholder={t("cloudSync.password")}
          autoComplete="current-password"
        />
      </div>

      {/* Action buttons */}
      <div style={{ display: "flex", gap: "10px", marginTop: "20px" }}>
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
          "Restoring is partial: email accounts will be recreated without passwords or OAuth tokens, and translation providers may need to be reconnected.",
        )}
      </div>

      {/* Restore confirmation with backup preview */}
      {restorePreview && (
        <ConfirmDialog
          title={t("cloudSync.restore", "Restore Settings Backup")}
          message={
            t("cloudSync.restorePreviewHeader", "Backup contents to restore:") +
            "\n" +
            t("cloudSync.restorePreviewSchema", "Schema version: {{version}}", { version: restorePreview.version }) +
            "\n" +
            t("cloudSync.restorePreviewExported", "Exported: {{date}}", {
              date: new Date(restorePreview.exported_at * 1000).toLocaleString(),
            }) +
            "\n" +
            t("cloudSync.restorePreviewAccounts", "Accounts: {{count}}", { count: restorePreview.account_count }) +
            "\n" +
            t("cloudSync.restorePreviewRules", "Rules: {{count}}", { count: restorePreview.rule_count }) +
            "\n" +
            t("cloudSync.restorePreviewKanban", "Kanban cards: {{count}}", { count: restorePreview.kanban_card_count }) +
            "\n" +
            t("cloudSync.restorePreviewKanbanNotes", "Kanban notes: {{count}}", { count: restorePreview.kanban_note_count }) +
            "\n" +
            t("cloudSync.restorePreviewSize", "Size: {{kb}} KB", { kb: (restorePreview.size_bytes / 1024).toFixed(1) }) +
            "\n\n" +
            t(
              "cloudSync.restoreConfirm",
              "This will replace local rules and Kanban cards/notes, and merge account metadata from the backup. Reauthentication will still be required for restored accounts. Continue?",
            )
          }
          destructive
          onCancel={() => setRestorePreview(null)}
          onConfirm={() => {
            setRestorePreview(null);
            doRestore();
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
          {t("cloudSync.lastBackup")}: {lastBackup}
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
