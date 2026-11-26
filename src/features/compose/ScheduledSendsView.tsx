import { useEffect, useId, useState } from "react";
import Button from "@/components/ui/Button";
import Select from "@/components/ui/Select";
import { SettingsNotice, StatusChip, type NoticeTone } from "@/features/settings/SettingsLayout";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { deleteControlledSend, listControlledSends, rescheduleControlledSend, restoreControlledSend, type ControlledSend, type SendSnapshot } from "@/lib/send-controls-api";
import { isComposeDirty, useComposeStore } from "@/stores/compose.store";
import { cleanupStagedComposeAttachment } from "@/lib/api";
import { useConfirmStore } from "@/stores/confirm.store";
import { localScheduleValue, scheduledInstant } from "./send-control-time";
import { loadUndoSeconds, saveUndoSeconds, UNDO_OPTIONS } from "./send-controls-settings";
import ScheduleTimeInput from "./ScheduleTimeInput";
import "./send-controls.css";

export const SEND_CONTROLS_QUERY = ["send-controls"] as const;

export function useControlledSends(enabled = true) {
  const client = useQueryClient();
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen("send-controls-changed", () => { void client.invalidateQueries({ queryKey: SEND_CONTROLS_QUERY }); })
      .then((fn) => { if (disposed) fn(); else unlisten = fn; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, [client]);
  return useQuery({ queryKey: SEND_CONTROLS_QUERY, queryFn: listControlledSends, enabled,
    refetchInterval: enabled ? (query) => query.state.data?.some((job) => job.kind === "undo" && (job.state === "pending" || job.state === "dispatching")) ? 1000 : 15000 : false });
}

export function openSendSnapshot(snapshot: SendSnapshot) {
  const compose = useComposeStore.getState();
  // openDraft owns the existing dirty-compose navigation guard.
  if (isComposeDirty()) {
    for (const attachment of snapshot.attachments) void cleanupStagedComposeAttachment(attachment.path).catch(() => {});
    return false;
  }
  compose.openDraft({ ...snapshot, savedAt: Date.now() });
  useComposeStore.setState({ composeMode: snapshot.composeMode });
  return true;
}

const STATE_TONE: Record<ControlledSend["state"], NoticeTone> = {
  pending: "info",
  dispatching: "info",
  review: "warning",
  draft: "neutral",
  sent: "success",
};

function ScheduledRow({ job }: { job: ControlledSend }) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const [date, setDate] = useState(() => localScheduleValue(Math.max(job.dueAt, Date.now() / 1000 + 3600)));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const dateInstant = scheduledInstant(date);
  const mutable = job.state === "pending";
  async function run(action: () => Promise<unknown>) {
    if (busy) return;
    setBusy(true); setError(false);
    try { await action(); } catch { setError(true); }
    finally { setBusy(false); await client.invalidateQueries({ queryKey: SEND_CONTROLS_QUERY }); }
  }
  const subject = job.subject || t("sendControls.noSubject");
  return <article className="settings-list__row settings-list__row--stacked send-controls-job" aria-label={subject}>
    <div className="send-controls-job__head">
      <div className="settings-list__main">
        <strong className="settings-list__title">{subject}</strong>
        <span className="settings-list__meta">{new Date(job.dueAt * 1000).toLocaleString(i18n.language, { timeZoneName: "longOffset" })}</span>
      </div>
      <StatusChip tone={STATE_TONE[job.state] ?? "neutral"} role="status">{t(`sendControls.${job.state}`)}</StatusChip>
    </div>
    {job.state === "review" && <SettingsNotice tone="warning">{t("sendControls.reviewWarning")}</SettingsNotice>}
    {job.state === "draft" && <p className="settings-hint">{t("sendControls.restored")}</p>}
    {mutable && <ScheduleTimeInput value={date} onChange={setDate} />}
    <div className="settings-actions">
      {mutable && <Button size="sm" disabled={busy || dateInstant === null} onClick={() => void run(() => rescheduleControlledSend(job, dateInstant!))}>{t("sendControls.reschedule")}</Button>}
      {job.state !== "dispatching" && job.state !== "sent" && <Button size="sm" disabled={busy} onClick={() => void run(async () => {
        if (!(await useConfirmStore.getState().confirm({ title: t("sendControls.edit"), message: t("sendControls.restoreWarning"), confirmLabel: t("sendControls.edit") }))) return;
        if (isComposeDirty()) throw new Error("Existing compose is dirty");
        const snapshot = await restoreControlledSend(job);
        if (!openSendSnapshot(snapshot)) throw new Error("Existing compose is dirty");
      })}>{t("sendControls.edit")}</Button>}
      {(job.state === "draft" || job.state === "sent") && <Button size="sm" variant="danger-ghost" disabled={busy} onClick={() => void run(async () => {
        if (await useConfirmStore.getState().confirm({ title: t("sendControls.remove"), message: t("sendControls.removeWarning"), confirmLabel: t("sendControls.remove"), destructive: true })) await deleteControlledSend(job);
      })}>{t("sendControls.remove")}</Button>}
    </div>
    {error && <p role="alert" className="settings-error">{t("sendControls.error")}</p>}
  </article>;
}

export function UndoSendPreference() {
  const { t } = useTranslation();
  const id = useId();
  const [seconds, setSeconds] = useState(loadUndoSeconds);
  return <div className="field send-controls-undo">
    <label className="field__label" htmlFor={id}>{t("sendControls.undoLabel")}</label>
    <Select id={id} aria-label={t("sendControls.undoLabel")} aria-describedby={`${id}-hint`} value={seconds} onChange={(e) => { const value = Number(e.target.value); setSeconds(value); saveUndoSeconds(value); }}>
      {UNDO_OPTIONS.map((value) => <option key={value} value={value}>{value === 0 ? t("sendControls.off") : t("sendControls.seconds", { count: value })}</option>)}
    </Select>
    <p id={`${id}-hint`} className="field__hint">{t("sendControls.undoHint")}</p>
  </div>;
}

/**
 * Scheduled sends + Undo-send preference.
 * - variant "section" (Settings → General): a .settings-section.
 * - variant "page" (default, standalone): padded panel with a page header.
 */
export default function ScheduledSendsView({ variant = "page" }: { variant?: "page" | "section" }) {
  const { t } = useTranslation();
  const query = useControlledSends();
  const headingId = useId();
  const refresh = <Button size="sm" onClick={() => void query.refetch()}>{t("sendControls.refresh")}</Button>;
  const body = <>
    <UndoSendPreference />
    {query.isError && <p role="alert" className="settings-error">{t("sendControls.loadError")}</p>}
    {query.data?.length === 0 && <p className="settings-empty">{t("sendControls.empty")}</p>}
    {query.data && query.data.length > 0 && <div className="settings-list">
      {query.data.map((job) => <ScheduledRow key={`${job.id}:${job.revision}`} job={job} />)}
    </div>}
  </>;
  if (variant === "section") {
    return <section className="settings-section" aria-labelledby={headingId}>
      <div className="settings-section__header">
        <div className="settings-section__heading">
          <h3 id={headingId} className="settings-section__title">{t("sendControls.title")}</h3>
          <p className="settings-section__description">{t("sendControls.policy")}</p>
        </div>
        <div className="settings-section__actions">{refresh}</div>
      </div>
      <div className="settings-section__body">{body}</div>
    </section>;
  }
  return <section className="send-controls-panel" aria-labelledby={headingId}>
    <header className="page-header page-header--flush">
      <div className="page-header__text">
        <h2 id={headingId} className="page-header__title">{t("sendControls.title")}</h2>
        <p className="page-header__description">{t("sendControls.policy")}</p>
      </div>
      <div className="page-header__actions">{refresh}</div>
    </header>
    <div className="send-controls-panel__body">{body}</div>
  </section>;
}
