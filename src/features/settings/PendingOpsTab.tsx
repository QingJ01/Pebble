import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Clock, RefreshCw, Trash2 } from "lucide-react";
import type { PendingMailOp } from "@/lib/api";
import { dismissFailedPendingMailOps, retryFailedSend } from "@/lib/api";
import { useConfirmStore } from "@/stores/confirm.store";
import { toastError } from "@/stores/toast.store";
import {
  pendingMailOpsQueryKey,
  pendingMailOpsSummaryQueryKey,
  useAccountsQuery,
  usePendingMailOpsQuery,
  usePendingMailOpsSummary,
} from "@/hooks/queries";
import { useMailStore } from "@/stores/mail.store";
import { formatDateTime } from "@/lib/formatDate";
import Button from "@/components/ui/Button";
import IconButton from "@/components/ui/IconButton";
import Select from "@/components/ui/Select";
import { SettingsNotice, SettingsSection, StatusChip, type NoticeTone } from "./SettingsLayout";

const metricStyle: React.CSSProperties = {
  padding: "var(--space-3)",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-lg)",
  background: "var(--color-surface-raised)",
};

const tableHeaderStyle: React.CSSProperties = {
  padding: "var(--space-2) var(--space-3)",
  textAlign: "left",
  fontSize: "var(--font-xs)",
  fontWeight: "var(--weight-semibold)" as React.CSSProperties["fontWeight"],
  color: "var(--color-text-secondary)",
  borderBottom: "1px solid var(--color-border)",
  whiteSpace: "nowrap",
};

const tableCellStyle: React.CSSProperties = {
  padding: "var(--space-2) var(--space-3)",
  fontSize: "var(--font-sm)",
  color: "var(--color-text-primary)",
  borderBottom: "1px solid var(--color-divider)",
  verticalAlign: "top",
};

function formatTimestamp(timestamp: number) {
  return formatDateTime(timestamp);
}

function statusTone(status: PendingMailOp["status"]): NoticeTone {
  if (status === "failed") return "warning";
  if (status === "in_progress") return "info";
  return "neutral";
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
      toastError(error, "errors.retrySend");
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

  const fetching = summaryQuery.isFetching || opsQuery.isFetching;

  return (
    <div>
      <SettingsSection
        title={t("pendingOps.queue", "Queue")}
        actions={
          <>
            {summary.failed_count > 0 && (
              <Button
                size="sm"
                variant="danger-ghost"
                onClick={dismissFailed}
                icon={<Trash2 size={14} />}
              >
                {t("pendingOps.dismissFailed", "Dismiss failed")}
              </Button>
            )}
            <IconButton
              size="sm"
              variant="secondary"
              label={t("pendingOps.refresh", "Refresh")}
              onClick={refresh}
              disabled={fetching}
              icon={<RefreshCw size={14} />}
            />
          </>
        }
      >
        <div className="field" style={{ maxWidth: "360px" }}>
          <label htmlFor="pending-ops-account" className="field__label">
            {t("pendingOps.accountFilter", "Account")}
          </label>
          <Select
            id="pending-ops-account"
            value={selectedAccountId ?? "all"}
            onChange={(event) => {
              const value = event.target.value;
              setSelectedAccountId(value === "all" ? null : value);
            }}
          >
            <option value="all">{t("pendingOps.allAccounts", "All accounts")}</option>
            {(accountsQuery.data ?? []).map((account) => (
              <option key={account.id} value={account.id}>
                {account.email}
              </option>
            ))}
          </Select>
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(128px, 1fr))", gap: "var(--space-2)" }}>
          <Metric label={t("pendingOps.pending", "Pending")} value={summary.pending_count} />
          <Metric label={t("pendingOps.retrying", "Retrying")} value={summary.in_progress_count} />
          <Metric label={t("pendingOps.failed", "Failed")} value={summary.failed_count} />
          <Metric label={t("pendingOps.active", "Active")} value={summary.total_active_count} />
        </div>

        {summary.last_error && (
          <SettingsNotice
            tone="warning"
            role="alert"
            style={{ display: "flex", gap: "var(--space-2)", alignItems: "flex-start" }}
          >
            <AlertCircle size={14} style={{ marginTop: "2px", flexShrink: 0 }} />
            <span>
              <strong style={{ color: "inherit" }}>{t("pendingOps.latestError", "Latest error")}: </strong>
              {summary.last_error}
            </span>
          </SettingsNotice>
        )}

        {loading ? (
          <p className="settings-empty">{t("common.loading", "Loading...")}</p>
        ) : ops.length === 0 ? (
          <p className="settings-empty" style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
            <Clock size={14} />
            <span>{t("pendingOps.empty", "No pending remote writes.")}</span>
          </p>
        ) : (
          <div
            className="scroll-region pending-ops-table-scroll"
            style={{ overflowX: "auto", border: "1px solid var(--color-border)", borderRadius: "var(--radius-lg)", background: "var(--color-surface-raised)" }}
          >
            <table style={{ width: "100%", minWidth: "840px", borderCollapse: "collapse" }}>
              <thead>
                <tr style={{ background: "var(--color-surface-sunken)" }}>
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
                {ops.map((op) => (
                  <tr key={op.id}>
                    <td style={tableCellStyle}>
                      <StatusChip tone={statusTone(op.status)}>{statusLabel(op.status)}</StatusChip>
                    </td>
                    <td style={tableCellStyle}>{operationLabel(op.op_type)}</td>
                    <td style={tableCellStyle}>{op.attempts}</td>
                    <td style={{ ...tableCellStyle, fontFamily: "var(--font-mono)" }}>
                      {op.message_id}
                    </td>
                    <td style={tableCellStyle}>{accountLabel(op.account_id)}</td>
                    <td style={tableCellStyle}>{formatTimestamp(op.updated_at)}</td>
                    <td style={{ ...tableCellStyle, color: op.last_error ? "var(--color-warning)" : "var(--color-text-tertiary)" }}>
                      <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", gap: "var(--space-2)" }}>
                        <span>{op.last_error ?? "-"}</span>
                        {op.op_type === "send" && op.status === "failed" && (
                          <Button size="sm" disabled={retryingSend !== null} onClick={() => { void retrySend(op); }}>
                            {t("pendingOps.retrySend", "Retry send")}
                          </Button>
                        )}
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </SettingsSection>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: number }) {
  return (
    <div style={metricStyle}>
      <div
        style={{
          fontSize: "var(--font-xs)",
          fontWeight: "var(--weight-semibold)",
          color: "var(--color-text-secondary)",
          marginBottom: "var(--space-1)",
        }}
      >
        {label}
      </div>
      <div
        style={{
          fontSize: "var(--font-xl)",
          fontWeight: "var(--weight-semibold)",
          lineHeight: "var(--leading-tight)",
          color: "var(--color-text-primary)",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {value}
      </div>
    </div>
  );
}
