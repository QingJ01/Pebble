import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { RefreshCw } from "lucide-react";
import { useUIStore } from "../stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import { startSync, stopSync } from "@/lib/api";
import { retryQueue } from "../lib/retry-queue";

interface MailErrorPayload {
  error_type: string;
  message: string;
  timestamp: number;
}

export default function StatusBar() {
  const { t } = useTranslation();
  const syncStatus = useUIStore((s) => s.syncStatus);
  const setSyncStatus = useUIStore((s) => s.setSyncStatus);
  const networkStatus = useUIStore((s) => s.networkStatus);
  const lastMailError = useUIStore((s) => s.lastMailError);
  const setLastMailError = useUIStore((s) => s.setLastMailError);
  const activeAccountId = useMailStore((s) => s.activeAccountId);

  // Listen for mail:error events from Rust backend
  useEffect(() => {
    const unlisten = listen<MailErrorPayload>("mail:error", (event) => {
      setLastMailError(event.payload.message);
      // Auto-clear error after 10 seconds
      setTimeout(() => setLastMailError(null), 10_000);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setLastMailError]);

  // Pause/resume retry queue based on network status
  useEffect(() => {
    if (networkStatus === "offline") {
      retryQueue.pause();
    } else {
      retryQueue.resume();
    }
  }, [networkStatus]);

  async function handleSync() {
    if (!activeAccountId) return;
    if (syncStatus === "syncing") {
      try {
        await stopSync(activeAccountId);
      } catch {}
      setSyncStatus("idle");
    } else {
      setSyncStatus("syncing");
      try {
        await startSync(activeAccountId);
      } catch {
        setSyncStatus("error");
      } finally {
        setSyncStatus("idle");
      }
    }
  }

  const syncText = {
    idle: t("status.ready", "Ready"),
    syncing: t("status.syncing", "Syncing..."),
    error: t("status.syncError", "Sync error"),
  }[syncStatus];

  const pendingCount = retryQueue.pendingCount;
  const notificationsEnabled = typeof window !== "undefined" && localStorage.getItem("pebble-notifications-enabled") === "true";

  return (
    <footer
      className="flex items-center px-3 h-6 text-xs border-t gap-3"
      style={{
        backgroundColor: "var(--color-statusbar-bg)",
        borderColor: "var(--color-border)",
        color: "var(--color-text-secondary)",
      }}
    >
      {networkStatus === "offline" ? (
        <span
          className="flex items-center gap-1"
          style={{ color: "var(--color-error, #ef4444)" }}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <line x1="1" y1="1" x2="23" y2="23" />
            <path d="M16.72 11.06A10.94 10.94 0 0 1 19 12.55" />
            <path d="M5 12.55a10.94 10.94 0 0 1 5.17-2.39" />
            <path d="M10.71 5.05A16 16 0 0 1 22.56 9" />
            <path d="M1.42 9a15.91 15.91 0 0 1 4.7-2.88" />
            <path d="M8.53 16.11a6 6 0 0 1 6.95 0" />
            <line x1="12" y1="20" x2="12.01" y2="20" />
          </svg>
          {t("status.offline", "Offline")}
        </span>
      ) : (
        <>
          <span>{syncText}</span>
          <button
            onClick={handleSync}
            disabled={!activeAccountId}
            title={syncStatus === "syncing" ? t("status.stopSync") : t("status.syncNow")}
            style={{
              background: "none",
              border: "none",
              cursor: activeAccountId ? "pointer" : "default",
              padding: "2px",
              color: "var(--color-text-secondary)",
              display: "flex",
              alignItems: "center",
              opacity: activeAccountId ? 1 : 0.4,
            }}
          >
            <RefreshCw
              size={13}
              style={{
                animation: syncStatus === "syncing" ? "spin 1s linear infinite" : "none",
              }}
            />
          </button>
        </>
      )}

      {pendingCount > 0 && (
        <span style={{ color: "var(--color-warning, #f59e0b)" }}>
          {t("status.pendingActions", "{{count}} pending", { count: pendingCount })}
        </span>
      )}

      {lastMailError && (
        <span
          className="truncate"
          style={{ color: "var(--color-error, #ef4444)" }}
        >
          {lastMailError}
        </span>
      )}

      <span className="ml-auto flex items-center gap-1">
        {notificationsEnabled && (
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
            <path d="M13.73 21a2 2 0 0 1-3.46 0" />
          </svg>
        )}
      </span>
    </footer>
  );
}
