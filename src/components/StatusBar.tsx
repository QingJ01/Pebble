import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { listen } from "@tauri-apps/api/event";
import { AlertCircle, AppWindow, Bell, Clock, RefreshCw, WifiOff, X } from "lucide-react";
import IconButton from "@/components/ui/IconButton";
import { describeError } from "@/lib/extractErrorMessage";
import { toastError } from "@/stores/toast.store";
import { useQueryClient, type Query, type QueryClient } from "@tanstack/react-query";
import { useUIStore, type RealtimeStatus } from "../stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import { stopSync } from "@/lib/api";
import { useSyncMutation } from "@/hooks/mutations/useSyncMutation";
import { invalidateKanbanQueries } from "@/hooks/queries/useKanbanCardsQuery";
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

interface SyncProgressPayload {
  account_id?: string;
  status?: "started" | "completed" | "error";
  phase?: string;
  message?: string | null;
}

interface SyncCompletePayload {
  account_id?: string;
}

export const MAIL_NEW_DEBOUNCE_MS = 500;
export const MAIL_NEW_MAX_WAIT_MS = 2_000;

interface PendingMailNewBatch {
  accountIds: Set<string>;
  folderIds: Set<string>;
  /** Some event carried no folder ids, so every list view must refresh. */
  allFolders: boolean;
}

function mailListQueryTouchesFolders(query: Query, folderIds: Set<string>) {
  // ["messages", folderId, folderIds?] and ["threads", folderId, folderIds?, limit, offset]
  const [, folderId, aggregateFolderIds] = query.queryKey;
  if (typeof folderId === "string" && folderIds.has(folderId)) return true;
  return Array.isArray(aggregateFolderIds)
    && aggregateFolderIds.some((id) => typeof id === "string" && folderIds.has(id));
}

/**
 * Drop every loaded page but the first from inactive infinite message lists so
 * their next mount does not refetch the whole scroll history. Active lists keep
 * their pages so the visible scroll position stays stable.
 */
function trimInactiveMessagePages(queryClient: QueryClient, matches: (query: Query) => boolean) {
  for (const query of queryClient.getQueryCache().findAll({ queryKey: ["messages"] })) {
    if (!matches(query) || query.getObserversCount() > 0) continue;
    const data = query.state.data as { pages?: unknown[]; pageParams?: unknown[] } | undefined;
    if (!data?.pages || data.pages.length <= 1) continue;
    queryClient.setQueryData(query.queryKey, {
      ...data,
      pages: data.pages.slice(0, 1),
      pageParams: (data.pageParams ?? []).slice(0, 1),
    });
  }
}

export function flushMailNewBatch(queryClient: QueryClient, batch: PendingMailNewBatch) {
  for (const accountId of batch.accountIds) {
    queryClient.invalidateQueries({ queryKey: ["folders", accountId] });
    queryClient.invalidateQueries({ queryKey: ["folder-unread-counts", accountId] });
  }

  const matches = batch.allFolders
    ? () => true
    : (query: Query) => mailListQueryTouchesFolders(query, batch.folderIds);
  if (!batch.allFolders && batch.folderIds.size === 0) return;

  trimInactiveMessagePages(queryClient, matches);
  queryClient.invalidateQueries({ queryKey: ["messages"], predicate: matches });
  queryClient.invalidateQueries({ queryKey: ["threads"], predicate: matches });
}

/**
 * Coalesce bursts of `mail:new` events (e.g. a large initial sync) into one
 * scoped invalidation: trailing debounce, but flush at least every max wait.
 */
export function createMailNewCoalescer(
  queryClient: QueryClient,
  { debounceMs = MAIL_NEW_DEBOUNCE_MS, maxWaitMs = MAIL_NEW_MAX_WAIT_MS } = {},
) {
  let batch: PendingMailNewBatch | null = null;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let maxWaitTimer: ReturnType<typeof setTimeout> | null = null;

  function clearTimers() {
    if (debounceTimer) clearTimeout(debounceTimer);
    if (maxWaitTimer) clearTimeout(maxWaitTimer);
    debounceTimer = null;
    maxWaitTimer = null;
  }

  function flush() {
    clearTimers();
    const pending = batch;
    batch = null;
    if (pending) flushMailNewBatch(queryClient, pending);
  }

  function push(payload: MailNewPayload | null | undefined) {
    batch ??= { accountIds: new Set(), folderIds: new Set(), allFolders: false };
    if (payload?.account_id) batch.accountIds.add(payload.account_id);
    const folderIds = payload?.folder_ids?.filter((id) => typeof id === "string" && id) ?? [];
    if (folderIds.length === 0) {
      batch.allFolders = true;
    } else {
      folderIds.forEach((id) => batch!.folderIds.add(id));
    }

    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(flush, debounceMs);
    maxWaitTimer ??= setTimeout(flush, maxWaitMs);
  }

  function cancel() {
    clearTimers();
    batch = null;
  }

  return { push, flush, cancel };
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
  const keepRunningInBackground = useUIStore((s) => s.keepRunningInBackground);
  const setKeepRunningInBackground = useUIStore((s) => s.setKeepRunningInBackground);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const syncMutation = useSyncMutation();
  const queryClient = useQueryClient();
  const { data: pendingOpsSummary } = usePendingMailOpsSummary(activeAccountId);
  const syncStatusRef = useRef(syncStatus);
  // Progress of the current pass for the active account: phase + messages received so far.
  const [syncPhase, setSyncPhase] = useState<string | null>(null);
  const [syncedCount, setSyncedCount] = useState(0);
  const activeAccountIdRef = useRef(activeAccountId);
  activeAccountIdRef.current = activeAccountId;

  useEffect(() => {
    syncStatusRef.current = syncStatus;
  }, [syncStatus]);

  function updateSyncStatus(status: typeof syncStatus) {
    syncStatusRef.current = status;
    setSyncStatus(status);
  }

  // Listen for mail:error events from Rust backend
  useEffect(() => {
    // Errors stay visible until dismissed or the next successful sync pass, so
    // an earlier error can never clear a later one.
    const unlisten = listen<MailErrorPayload>("mail:error", (event) => {
      setLastMailError(event.payload.message);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setLastMailError]);

  function refreshMailQueries() {
    queryClient.invalidateQueries({ queryKey: ["folders"] });
    queryClient.invalidateQueries({ queryKey: ["messages"] });
    queryClient.invalidateQueries({ queryKey: ["threads"] });
    queryClient.invalidateQueries({ queryKey: ["folder-unread-counts"] });
    void invalidateKanbanQueries(queryClient);
  }

  function isActiveAccountEvent(accountId?: string | null) {
    return !accountId || !activeAccountId || accountId === activeAccountId;
  }

  // Listen for sync-complete: legacy worker-exit event used by one-shot syncs.
  useEffect(() => {
    const unlisten = listen<SyncCompletePayload>("mail:sync-complete", (event) => {
      if (!isActiveAccountEvent(event.payload?.account_id)) return;
      if (syncStatusRef.current !== "error") {
        updateSyncStatus("idle");
      }
      refreshMailQueries();
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [activeAccountId, setSyncStatus, queryClient]);

  // Listen for per-pass sync progress. Background workers are long-lived, so
  // UI "syncing" must track a concrete pass rather than worker lifetime.
  useEffect(() => {
    const unlisten = listen<SyncProgressPayload>("mail:sync-progress", (event) => {
      const { account_id, status, message, phase } = event.payload;
      if (!isActiveAccountEvent(account_id)) return;
      if (status === "started") {
        setSyncPhase(phase ?? null);
        setSyncedCount(0);
        updateSyncStatus("syncing");
      } else if (status === "completed") {
        updateSyncStatus("idle");
        setSyncPhase(null);
        setLastMailError(null);
        refreshMailQueries();
      } else if (status === "error") {
        updateSyncStatus("error");
        if (message) {
          setLastMailError(message);
        }
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [activeAccountId, setLastMailError, setSyncStatus, queryClient]);

  // Listen for new mail events: incremental data refresh
  useEffect(() => {
    const coalescer = createMailNewCoalescer(queryClient);
    const unlisten = listen<MailNewPayload>("mail:new", (event) => {
      coalescer.push(event.payload);
      const accountId = event.payload?.account_id;
      const active = activeAccountIdRef.current;
      if (syncStatusRef.current === "syncing" && (!accountId || !active || accountId === active)) {
        setSyncedCount((count) => count + 1);
      }
    });
    return () => {
      // Deliver what already arrived rather than dropping it on unmount.
      coalescer.flush();
      unlisten.then((fn) => fn());
    };
  }, [queryClient]);

  useEffect(() => {
    const unlisten = listen("mail:pending-ops-changed", () => {
      queryClient.invalidateQueries({
        queryKey: pendingMailOpsSummaryQueryKey(activeAccountId),
      });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["threads"] });
      // Reverted/abandoned ops can move or delete messages on the board.
      void invalidateKanbanQueries(queryClient);
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
      try {
        await stopSync(activeAccountId);
        updateSyncStatus("idle");
        setSyncPhase(null);
      } catch (error) {
        toastError(error, "status.stopSyncFailed");
      }
    } else {
      updateSyncStatus("syncing");
      try {
        await syncMutation.mutateAsync(activeAccountId);
      } catch {
        updateSyncStatus("error");
      }
    }
  }

  const syncingBase = syncPhase === "initial"
    ? t("status.syncingInitial", "Initial sync…")
    : t("status.syncing", "Syncing...");
  const syncText = {
    idle: t("status.ready", "Ready"),
    syncing: syncedCount > 0
      ? t("status.syncingWithCount", "{{status}} {{count}} new", { status: syncingBase, count: syncedCount })
      : syncingBase,
    error: t("status.syncError", "Sync error"),
  }[syncStatus];
  const mailError = lastMailError ? describeError(lastMailError, { fallbackKey: "errors.sync" }) : null;
  const realtimeStatus = activeAccountId ? realtimeStatusByAccount[activeAccountId] : undefined;
  const realtimeStatusText = getRealtimeStatusText(realtimeStatus, t);

  const pendingRemoteWrites = pendingOpsSummary?.total_active_count ?? 0;
  const failedRemoteWrites = pendingOpsSummary?.failed_count ?? 0;
  const retryingRemoteWrites = pendingOpsSummary?.in_progress_count ?? 0;
  // Constant accessible name plus aria-pressed for the state; the tooltip
  // spells out whether closing keeps the app running.
  const backgroundToggleLabel = t(
    "status.keepRunningInBackground",
    "Keep running in background on close",
  );
  const backgroundToggleTitle = keepRunningInBackground
    ? t("status.keepRunningBackgroundEnabled", "Keep running in background on close: on")
    : t("status.keepRunningBackgroundDisabled", "Keep running in background on close: off");
  const pendingRemoteText = retryingRemoteWrites > 0
    ? t("status.remoteWritesRetrying", "{{count}} remote writes retrying", { count: retryingRemoteWrites })
    : failedRemoteWrites > 0
      ? t("status.remoteWritesPending", "{{count}} remote writes pending", { count: pendingRemoteWrites })
      : t("status.remoteWritesQueued", "{{count}} remote writes queued", { count: pendingRemoteWrites });

  return (
    <footer
      className="status-bar flex items-center gap-3"
    >
      {networkStatus === "offline" ? (
        // The offline banner above the content announces the state; this is a
        // compact, non-live reminder so screen readers hear it only once.
        <span
          className="flex items-center gap-1"
          style={{ color: "var(--color-danger)" }}
        >
          <WifiOff aria-hidden="true" size={12} />
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
          <IconButton
            onClick={() => void handleSync()}
            disabled={!activeAccountId}
            label={syncStatus === "syncing" ? t("status.stopSync") : t("status.syncNow")}
            icon={(
              <RefreshCw
                size={13}
                className={syncStatus === "syncing" ? "status-bar__spin" : undefined}
              />
            )}
          />
          {pendingRemoteWrites > 0 && (
            <span
              role={failedRemoteWrites > 0 ? "alert" : "status"}
              aria-live={failedRemoteWrites > 0 ? "assertive" : "polite"}
              aria-atomic="true"
              className="flex items-center gap-1 truncate"
              title={pendingOpsSummary?.last_error ?? pendingRemoteText}
              style={{
                color: failedRemoteWrites > 0
                  ? "var(--color-warning)"
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

      {mailError && (
        <span className="flex items-center gap-1 min-w-0" style={{ color: "var(--color-danger)" }}>
          <span
            role="alert"
            aria-live="assertive"
            aria-atomic="true"
            className="truncate"
            title={mailError.detail ? `${mailError.title}\n${mailError.detail}` : mailError.title}
          >
            {mailError.title}
          </span>
          <IconButton
            label={t("status.dismissError", "Dismiss error")}
            icon={<X size={12} />}
            onClick={() => setLastMailError(null)}
            style={{ color: "inherit" }}
          />
        </span>
      )}

      <span className="ml-auto flex items-center gap-1">
        <IconButton
          onClick={() => setKeepRunningInBackground(!keepRunningInBackground)}
          label={backgroundToggleLabel}
          tooltip={backgroundToggleTitle}
          pressed={keepRunningInBackground}
          icon={<AppWindow size={13} />}
        />
        {notificationsEnabled && (
          <span title={t("status.notificationsOn", "Notifications on")} className="inline-flex">
            <Bell aria-hidden="true" size={12} />
          </span>
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
