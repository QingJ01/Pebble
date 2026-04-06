import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import ConfirmDialog from "@/components/ConfirmDialog";
import {
  testWebdavConnection,
  backupToWebdav,
  restoreFromWebdav,
} from "../../lib/api";

const LAST_BACKUP_KEY = "pebble-cloud-sync-last-backup";

const labelStyle: React.CSSProperties = {
  display: "block",
  fontSize: "12px",
  fontWeight: 500,
  color: "var(--color-text-secondary)",
  marginBottom: "4px",
};

const inputStyle: React.CSSProperties = {
  width: "100%",
  padding: "8px 10px",
  fontSize: "13px",
  border: "1px solid var(--color-border)",
  borderRadius: "6px",
  background: "var(--color-bg-secondary)",
  color: "var(--color-text-primary)",
  boxSizing: "border-box",
};

const fieldGroupStyle: React.CSSProperties = {
  marginBottom: "14px",
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
        `${t("cloudSync.connectionFailed")}: ${err instanceof Error ? err.message : String(err)}`,
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
        t("cloudSync.backupFailed", { error: err instanceof Error ? err.message : String(err) }),
      );
      setStatusType("error");
    } finally {
      setBacking(false);
    }
  }

  const [showRestoreConfirm, setShowRestoreConfirm] = useState(false);

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
        t("cloudSync.restoreFailed", { error: err instanceof Error ? err.message : String(err) }),
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
          "Back up rules, cards, and account metadata to WebDAV. This does not sync mail data, attachments, or OAuth secrets.",
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
          onClick={() => setShowRestoreConfirm(true)}
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

      {/* Restore confirmation */}
      {showRestoreConfirm && (
        <ConfirmDialog
          title={t("cloudSync.restore", "Restore Settings Backup")}
          message={t(
            "cloudSync.restoreConfirm",
            "This will replace your local rules, cards, and saved account metadata with the backup. Reauthentication will still be required. Continue?",
          )}
          destructive
          onCancel={() => setShowRestoreConfirm(false)}
          onConfirm={() => {
            setShowRestoreConfirm(false);
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
