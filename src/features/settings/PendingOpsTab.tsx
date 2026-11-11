import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Clock, RefreshCw, Trash2 } from "lucide-react";
import type { PendingMailOp } from "@/lib/api";
import { dismissFailedPendingMailOps, retryFailedSend } from "@/lib/api";
import { useConfirmStore } from "@/stores/confirm.store";
import { useToastStore } from "@/stores/toast.store";
import {
  pendingMailOpsQueryKey,
  pendingMailOpsSummaryQueryKey,
  useAccountsQuery,
  usePendingMailOpsQuery,
  usePendingMailOpsSummary,
} from "@/hooks/queries";
import { useMailStore } from "@/stores/mail.store";

const metricStyle: React.CSSProperties = {
  minWidth: "132px",
  padding: "12px",
  border: "1px solid var(--color-border)",
  borderRadius: "6px",
  background: "var(--color-bg-secondary)",
};

const tableHeaderStyle: React.CSSProperties = {
  padding: "9px 10px",
  textAlign: "left",
  fontSize: "11px",
  fontWeight: 600,
  color: "var(--color-text-secondary)",
  borderBottom: "1px solid var(--color-border)",
  whiteSpace: "nowrap",
};

const tableCellStyle: React.CSSProperties = {
  padding: "10px",
  fontSize: "12px",
  color: "var(--color-text-primary)",
  borderBottom: "1px solid var(--color-border)",
  verticalAlign: "top",
};

function formatTimestamp(timestamp: number) {
  return new Date(timestamp * 1000).toLocaleString();
}

function statusTone(status: PendingMailOp["status"]) {
  if (status === "failed") {
    return {
      color: "var(--color-warning, #d97706)",
      background: "rgba(217, 119, 6, 0.12)",
    };
  }
  if (status === "in_progress") {
    return {
      color: "var(--color-accent)",
      background: "rgba(37, 99, 235, 0.12)",
    };
  }
  return {
    color: "var(--color-text-secondary)",
    background: "var(--color-bg-hover)",
  };
}

export default function PendingOpsTab() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const [selectedAccountId, setSelectedAccountId] = useState<string | null>(activeAccountId);
  const [retryingSend, setRetryingSend] = useState<string | null>(null);
  async function retrySend(op: PendingMailOp) {
    if (retryingSend) return;
    setRetryingSend(op.id);
    try {
      const confirmed = await useConfirmStore.getState().confirm({
        title: t("pendingOps.retrySend", "Retry send"),
        message: t("pendingOps.retrySendWarning", "Check Sent first. The previous delivery may have succeeded; retrying could send a duplicate. Retry this message?"),
        confirmLabel: t("pendingOps.retrySend", "Retry send"),
        destructive: true,
      });
      if (!confirmed) return;
      await retryFailedSend(op.id);
      await refresh();
    } catch (error) {
      useToastStore.getState().addToast({ type: "error", message: String(error) });
    } finally {
      setRetryingSend(null);
    }
  }

  const accountsQuery = useAccountsQuery();
  const summaryQuery = usePendingMailOpsSummary(selectedAccountId);
  const opsQuery = usePendingMailOpsQuery(selectedAccountId);

  const accountsById = useMemo(() => {
    return new Map((accountsQuery.data ?? []).map((account) => [account.id, account]));
  }, [accountsQuery.data]);

  useEffect(() => {
    const unlisten = listen("mail:pending-ops-changed", () => {
      queryClient.invalidateQueries({ queryKey: pendingMailOpsSummaryQueryKey(selectedAccountId) });
      queryClient.invalidateQueries({ queryKey: pendingMailOpsQueryKey(selectedAccountId) });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [queryClient, selectedAccountId]);

  function statusLabel(status: PendingMailOp["status"]) {
    if (status === "failed") return t("pendingOps.statusValue.failed", "Failed");
    if (status === "in_progress") return t("pendingOps.statusValue.inProgress", "Retrying");
    return t("pendingOps.statusValue.pending", "Pending");
  }

  function operationLabel(opType: string) {
    const key = `pendingOps.ops.${opType}`;
    const fallbacks: Record<string, string> = {
      archive: "Archive",
      unarchive: "Unarchive",
      restore: "Restore",
      delete: "Delete",
      delete_permanent: "Delete permanently",
      update_flags: "Update flags",
      move_to_folder: "Move to folder",
      send: "Send",
    };
    return t(key, fallbacks[opType] ?? opType);
  }

  function accountLabel(accountId: string) {
    const account = accountsById.get(accountId);
    return account?.email ?? t("pendingOps.unknownAccount", "Unknown account");
  }

  async function refresh() {
    await Promise.all([summaryQuery.refetch(), opsQuery.refetch()]);
  }

  async function dismissFailed() {
    await dismissFailedPendingMailOps(selectedAccountId);
    await refresh();
  }

  const summary = summaryQuery.data ?? {
    pending_count: 0,
    in_progress_count: 0,
    failed_count: 0,
    total_active_count: 0,
    last_error: null,
    updated_at: null,
  };
  const ops = opsQuery.data ?? [];
  const loading = summaryQuery.isLoading || opsQuery.isLoading;

  return (
    <div>
      <div
        style={{
          display: "flex",
          alignItems: "flex-start",
          justifyContent: "space-between",
          gap: "16px",
          marginBottom: "18px",
        }}
      >
        <div>
          <h2
            style={{
              fontSize: "18px",
              fontWeight: 600,
              color: "var(--color-text-primary)",
              marginTop: 0,
              marginBottom: "8px",
            }}
          >
            {t("pendingOps.title", "Remote Writes")}
          </h2>
          <p
            style={{
              margin: 0,
              fontSize: "13px",
              lineHeight: 1.5,
              color: "var(--color-text-secondary)",
              maxWidth: "640px",
            }}
          >
            {t(
              "pendingOps.description",
              "Queued mail changes are retried in the background until the provider accepts them.",
            )}
          </p>
        </div>

        <div style={{ display: "flex", gap: "6px", flexShrink: 0 }}>
          {summary.failed_count > 0 && (
            <button
              type="button"
              onClick={dismissFailed}
              title={t("pendingOps.dismissFailed", "Dismiss failed")}
              aria-label={t("pendingOps.dismissFailed", "Dismiss failed")}
              style={{
                height: "34px",
                display: "inline-flex",
                alignItems: "center",
                gap: "6px",
                padding: "0 12px",
                borderRadius: "6px",
                border: "1px solid var(--color-border)",
                background: "var(--color-bg-secondary)",
                color: "var(--color-warning, #d97706)",
                cursor: "pointer",
                fontSize: "12px",
                fontWeight: 500,
                whiteSpace: "nowrap",
              }}
            >
              <Trash2 size={14} />
              {t("pendingOps.dismissFailed", "Dismiss failed")}
            </button>
          )}
          <button
            type="button"
            onClick={refresh}
            title={t("pendingOps.refresh", "Refresh")}
            aria-label={t("pendingOps.refresh", "Refresh")}
            disabled={summaryQuery.isFetching || opsQuery.isFetching}
            style={{
              width: "34px",
              height: "34px",
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              background: "var(--color-bg-secondary)",
              color: "var(--color-text-primary)",
              cursor: "pointer",
              opacity: summaryQuery.isFetching || opsQuery.isFetching ? 0.65 : 1,
              flexShrink: 0,
            }}
          >
            <RefreshCw size={15} />
          </button>
        </div>
      </div>

      <label
        htmlFor="pending-ops-account"
        style={{
          display: "block",
          fontSize: "12px",
          fontWeight: 600,
          color: "var(--color-text-secondary)",
          marginBottom: "6px",
        }}
      >
        {t("pendingOps.accountFilter", "Account")}
      </label>
      <select
        id="pending-ops-account"
        value={selectedAccountId ?? "all"}
        onChange={(event) => {
          const value = event.target.value;
          setSelectedAccountId(value === "all" ? null : value);
        }}
        style={{
          width: "min(360px, 100%)",
          padding: "8px 10px",
          borderRadius: "6px",
          border: "1px solid var(--color-border)",
          background: "var(--color-bg-secondary)",
          color: "var(--color-text-primary)",
          fontSize: "13px",
          marginBottom: "18px",
        }}
      >
        <option value="all">{t("pendingOps.allAccounts", "All accounts")}</option>
        {(accountsQuery.data ?? []).map((account) => (
          <option key={account.id} value={account.id}>
            {account.email}
          </option>
        ))}
      </select>

      <div style={{ display: "flex", gap: "10px", flexWrap: "wrap", marginBottom: "18px" }}>
        <Metric label={t("pendingOps.pending", "Pending")} value={summary.pending_count} />
        <Metric label={t("pendingOps.retrying", "Retrying")} value={summary.in_progress_count} />
        <Metric label={t("pendingOps.failed", "Failed")} value={summary.failed_count} />
        <Metric label={t("pendingOps.active", "Active")} value={summary.total_active_count} />
      </div>

      {summary.last_error && (
        <div
          role="alert"
          style={{
            display: "flex",
            gap: "8px",
            alignItems: "flex-start",
            padding: "10px 12px",
            marginBottom: "18px",
            borderRadius: "6px",
            border: "1px solid rgba(217, 119, 6, 0.3)",
            background: "rgba(217, 119, 6, 0.1)",
            color: "var(--color-warning, #d97706)",
            fontSize: "12px",
            lineHeight: 1.45,
          }}
        >
          <AlertCircle size={15} style={{ marginTop: "1px", flexShrink: 0 }} />
          <span>
            <strong>{t("pendingOps.latestError", "Latest error")}: </strong>
            {summary.last_error}
          </span>
        </div>
      )}

      <h3
        style={{
          fontSize: "14px",
          fontWeight: 600,
          marginTop: 0,
          marginBottom: "10px",
          color: "var(--color-text-primary)",
        }}
      >
        {t("pendingOps.queue", "Queue")}
      </h3>

      {loading ? (
        <div style={{ fontSize: "13px", color: "var(--color-text-secondary)" }}>
          {t("common.loading", "Loading...")}
        </div>
      ) : ops.length === 0 ? (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "8px",
            padding: "16px 0",
            color: "var(--color-text-secondary)",
            fontSize: "13px",
          }}
        >
          <Clock size={15} />
          <span>{t("pendingOps.empty", "No pending remote writes.")}</span>
        </div>
      ) : (
        <div className="scroll-region pending-ops-table-scroll" style={{ overflowX: "auto", border: "1px solid var(--color-border)", borderRadius: "6px" }}>
          <table style={{ width: "100%", minWidth: "840px", borderCollapse: "collapse" }}>
            <thead>
              <tr style={{ background: "var(--color-bg-secondary)" }}>
                <th style={tableHeaderStyle}>{t("pendingOps.status", "Status")}</th>
                <th style={tableHeaderStyle}>{t("pendingOps.operation", "Operation")}</th>
                <th style={tableHeaderStyle}>{t("pendingOps.attempts", "Attempts")}</th>
                <th style={tableHeaderStyle}>{t("pendingOps.message", "Message")}</th>
                <th style={tableHeaderStyle}>{t("pendingOps.account", "Account")}</th>
                <th style={tableHeaderStyle}>{t("pendingOps.updated", "Updated")}</th>
                <th style={tableHeaderStyle}>{t("pendingOps.lastError", "Last error")}</th>
              </tr>
            </thead>
            <tbody>
              {ops.map((op) => {
                const tone = statusTone(op.status);
                return (
                  <tr key={op.id}>
                    <td style={tableCellStyle}>
                      <span
                        style={{
                          display: "inline-flex",
                          alignItems: "center",
                          borderRadius: "999px",
                          padding: "2px 8px",
                          fontSize: "11px",
                          fontWeight: 600,
                          color: tone.color,
                          background: tone.background,
                          whiteSpace: "nowrap",
                        }}
                      >
                        {statusLabel(op.status)}
                      </span>
                    </td>
                    <td style={tableCellStyle}>{operationLabel(op.op_type)}</td>
                    <td style={tableCellStyle}>{op.attempts}</td>
                    <td style={{ ...tableCellStyle, fontFamily: "var(--font-mono, monospace)" }}>
                      {op.message_id}
                    </td>
                    <td style={tableCellStyle}>{accountLabel(op.account_id)}</td>
                    <td style={tableCellStyle}>{formatTimestamp(op.updated_at)}</td>
                    <td style={{ ...tableCellStyle, color: op.last_error ? "var(--color-warning, #d97706)" : "var(--color-text-secondary)" }}>
                      {op.last_error ?? "-"}
                      {op.op_type === "send" && op.status === "failed" && (
                        <button disabled={retryingSend !== null} onClick={() => { void retrySend(op); }}>
                          {t("pendingOps.retrySend", "Retry send")}
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: number }) {
  return (
    <div style={metricStyle}>
      <div
        style={{
          fontSize: "11px",
          fontWeight: 600,
          color: "var(--color-text-secondary)",
          marginBottom: "5px",
        }}
      >
        {label}
      </div>
      <div
        style={{
          fontSize: "22px",
          fontWeight: 650,
          lineHeight: 1,
          color: "var(--color-text-primary)",
        }}
      >
        {value}
      </div>
    </div>
  );
}
