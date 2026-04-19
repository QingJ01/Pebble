import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { listen } from "@tauri-apps/api/event";
import { AlertCircle, Clock, RefreshCw } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { useUIStore, type RealtimeStatus } from "../stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import { stopSync } from "@/lib/api";
import { useSyncMutation } from "@/hooks/mutations/useSyncMutation";
import {
  pendingMailOpsSummaryQueryKey,
  usePendingMailOpsSummary,
} from "@/hooks/queries";

interface MailErrorPayload {
  error_type: string;
  message: string;
  timestamp: number;
}

interface MailNewPayload {
  account_id?: string;
  message_id?: string;
  folder_ids?: string[];
  thread_id?: string | null;
  subject?: string;
  from?: string;
  received_at?: number;
}

export default function StatusBar() {
  const { t } = useTranslation();
  const syncStatus = useUIStore((s) => s.syncStatus);
  const setSyncStatus = useUIStore((s) => s.setSyncStatus);
  const networkStatus = useUIStore((s) => s.networkStatus);
  const lastMailError = useUIStore((s) => s.lastMailError);
  const setLastMailError = useUIStore((s) => s.setLastMailError);
  const realtimeStatusByAccount = useUIStore((s) => s.realtimeStatusByAccount);
  const setRealtimeStatus = useUIStore((s) => s.setRealtimeStatus);
  const notificationsEnabled = useUIStore((s) => s.notificationsEnabled);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const syncMutation = useSyncMutation();
  const queryClient = useQueryClient();
  const { data: pendingOpsSummary } = usePendingMailOpsSummary(activeAccountId);

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

  // Listen for sync-complete: set idle + refresh data
  useEffect(() => {
    const unlisten = listen("mail:sync-complete", () => {
      setSyncStatus("idle");
      queryClient.invalidateQueries({ queryKey: ["folders"] });
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["threads"] });
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [setSyncStatus, queryClient]);

  // Listen for new mail events: incremental data refresh
  useEffect(() => {
    const unlisten = listen<MailNewPayload>("mail:new", (event) => {
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["threads"] });
      if (event.payload.account_id) {
        queryClient.invalidateQueries({ queryKey: ["folders", event.payload.account_id] });
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [queryClient]);

  useEffect(() => {
    const unlisten = listen("mail:pending-ops-changed", () => {
      queryClient.invalidateQueries({
        queryKey: pendingMailOpsSummaryQueryKey(activeAccountId),
      });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["threads"] });
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [activeAccountId, queryClient]);

  useEffect(() => {
    const unlisten = listen<RealtimeStatus>("mail:realtime-status", (event) => {
      setRealtimeStatus(event.payload.account_id, event.payload);
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [setRealtimeStatus]);

  async function handleSync() {
    if (!activeAccountId) return;
    if (syncStatus === "syncing") {
      try { await stopSync(activeAccountId); } catch {}
      setSyncStatus("idle");
    } else {
      setSyncStatus("syncing");
      try {
        await syncMutation.mutateAsync(activeAccountId);
        // Don't set idle here — wait for mail:sync-complete event
      } catch {
        setSyncStatus("error");
      }
    }
  }

  const syncText = {
    idle: t("status.ready", "Ready"),
    syncing: t("status.syncing", "Syncing..."),
    error: t("status.syncError", "Sync error"),
  }[syncStatus];
  const realtimeStatus = activeAccountId ? realtimeStatusByAccount[activeAccountId] : undefined;
  const realtimeStatusText = getRealtimeStatusText(realtimeStatus, t);

  const pendingRemoteWrites = pendingOpsSummary?.total_active_count ?? 0;
  const failedRemoteWrites = pendingOpsSummary?.failed_count ?? 0;
  const retryingRemoteWrites = pendingOpsSummary?.in_progress_count ?? 0;
  const pendingRemoteText = retryingRemoteWrites > 0
    ? t("status.remoteWritesRetrying", "{{count}} remote writes retrying", { count: retryingRemoteWrites })
    : failedRemoteWrites > 0
      ? t("status.remoteWritesPending", "{{count}} remote writes pending", { count: pendingRemoteWrites })
      : t("status.remoteWritesQueued", "{{count}} remote writes queued", { count: pendingRemoteWrites });

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
          role="status"
          aria-live="polite"
          aria-atomic="true"
          className="flex items-center gap-1"
          style={{ color: "var(--color-error, #ef4444)" }}
        >
          <svg aria-hidden="true" focusable="false" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
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
          <span role="status" aria-live="polite" aria-atomic="true">{syncText}</span>
          {realtimeStatusText && (
            <span
              role="status"
              aria-live="polite"
              aria-atomic="true"
              aria-label={realtimeStatusText}
              className="truncate"
              title={realtimeStatus?.message ?? realtimeStatusText}
              style={{ maxWidth: "180px" }}
            >
              {realtimeStatusText}
            </span>
          )}
          <button
            onClick={handleSync}
            disabled={!activeAccountId}
            title={syncStatus === "syncing" ? t("status.stopSync") : t("status.syncNow")}
            aria-label={syncStatus === "syncing" ? t("status.stopSync") : t("status.syncNow")}
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
              aria-hidden="true"
              size={13}
              style={{
                animation: syncStatus === "syncing" ? "spin 1s linear infinite" : "none",
              }}
            />
          </button>
          {pendingRemoteWrites > 0 && (
            <span
              role={failedRemoteWrites > 0 ? "alert" : "status"}
              aria-live={failedRemoteWrites > 0 ? "assertive" : "polite"}
              aria-atomic="true"
              className="flex items-center gap-1 truncate"
              title={pendingOpsSummary?.last_error ?? pendingRemoteText}
              style={{
                color: failedRemoteWrites > 0
                  ? "var(--color-warning, #d97706)"
                  : "var(--color-text-secondary)",
                maxWidth: "220px",
              }}
            >
              {failedRemoteWrites > 0 ? <AlertCircle aria-hidden="true" size={13} /> : <Clock aria-hidden="true" size={13} />}
              <span className="truncate">{pendingRemoteText}</span>
            </span>
          )}
        </>
      )}

      {lastMailError && (
        <span
          role="alert"
          aria-live="assertive"
          aria-atomic="true"
          className="truncate"
          style={{ color: "var(--color-error, #ef4444)" }}
        >
          {lastMailError}
        </span>
      )}

      <span className="ml-auto flex items-center gap-1">
        {notificationsEnabled && (
          <svg aria-hidden="true" focusable="false" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
            <path d="M13.73 21a2 2 0 0 1-3.46 0" />
          </svg>
        )}
      </span>
    </footer>
  );
}

function getRealtimeStatusText(
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
