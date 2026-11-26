import { useEffect, useState } from "react";
import Button from "@/components/ui/Button";
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
  return <article aria-label={job.subject || t("sendControls.noSubject")}>
    <strong>{job.subject || t("sendControls.noSubject")}</strong>
    <p>{new Date(job.dueAt * 1000).toLocaleString(i18n.language, { timeZoneName: "longOffset" })}</p>
    <p role="status">{t(`sendControls.${job.state}`)}</p>
    {job.state === "review" && <p>{t("sendControls.reviewWarning")}</p>}
    {job.state === "draft" && <p>{t("sendControls.restored")}</p>}
    {mutable && <ScheduleTimeInput value={date} onChange={setDate} />}
    <div className="send-controls-actions">
      {mutable && <Button size="sm" variant="primary" disabled={busy || dateInstant === null} onClick={() => void run(() => rescheduleControlledSend(job, dateInstant!))}>{t("sendControls.reschedule")}</Button>}
      {job.state !== "dispatching" && job.state !== "sent" && <Button size="sm" disabled={busy} onClick={() => void run(async () => {
        if (!(await useConfirmStore.getState().confirm({ title: t("sendControls.edit"), message: t("sendControls.restoreWarning"), confirmLabel: t("sendControls.edit") }))) return;
        if (isComposeDirty()) throw new Error("Existing compose is dirty");
        const snapshot = await restoreControlledSend(job);
        if (!openSendSnapshot(snapshot)) throw new Error("Existing compose is dirty");
      })}>{t("sendControls.edit")}</Button>}
      {(job.state === "draft" || job.state === "sent") && <Button size="sm" variant="danger" disabled={busy} onClick={() => void run(async () => {
        if (await useConfirmStore.getState().confirm({ title: t("sendControls.remove"), message: t("sendControls.removeWarning"), confirmLabel: t("sendControls.remove"), destructive: true })) await deleteControlledSend(job);
      })}>{t("sendControls.remove")}</Button>}
    </div>
    {error && <p role="alert">{t("sendControls.error")}</p>}
  </article>;
}

export function UndoSendPreference() {
  const { t } = useTranslation();
  const [seconds, setSeconds] = useState(loadUndoSeconds);
  return <div className="send-controls-options">
    <label>{t("sendControls.undoLabel")} <select aria-label={t("sendControls.undoLabel")} value={seconds} onChange={(e) => { const value = Number(e.target.value); setSeconds(value); saveUndoSeconds(value); }}>
      {UNDO_OPTIONS.map((value) => <option key={value} value={value}>{value === 0 ? t("sendControls.off") : t("sendControls.seconds", { count: value })}</option>)}
    </select></label>
    <small>{t("sendControls.undoHint")}</small>
  </div>;
}

export default function ScheduledSendsView() {
  const { t } = useTranslation();
  const query = useControlledSends();
  return <section className="send-controls-panel" aria-label={t("sendControls.title")}>
    <h3>{t("sendControls.title")}</h3>
    <p>{t("sendControls.policy")}</p>
    <UndoSendPreference />
    <Button size="sm" onClick={() => void query.refetch()}>{t("sendControls.refresh")}</Button>
    {query.isError && <p role="alert">{t("sendControls.loadError")}</p>}
    {query.data?.length === 0 && <p>{t("sendControls.empty")}</p>}
    {query.data?.map((job) => <ScheduledRow key={`${job.id}:${job.revision}`} job={job} />)}
  </section>;
}
