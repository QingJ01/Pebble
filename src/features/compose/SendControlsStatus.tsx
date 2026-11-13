import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useMailStore } from "@/stores/mail.store";
import { threadMessagesQueryKey } from "@/hooks/queries/useThreadMessagesQuery";
import { useTranslation } from "react-i18next";
import { isComposeDirty } from "@/stores/compose.store";
import { useSendControlsStore } from "@/stores/send-controls.store";
import { cleanupStagedComposeAttachment } from "@/lib/api";
import { useToastStore } from "@/stores/toast.store";
import { useQueryClient } from "@tanstack/react-query";
import { restoreControlledSend, type ControlledSend } from "@/lib/send-controls-api";
import { openSendSnapshot, SEND_CONTROLS_QUERY, useControlledSends } from "./ScheduledSendsView";
import { countdownSeconds } from "./send-control-time";
import "./send-controls.css";

/** Keeps Undo visible if the compose view is unmounted by navigation. */
export default function SendControlsStatus() {
  const { t } = useTranslation();
  const composeUndoId = useSendControlsStore((s) => s.composeUndoId);
  const query = useControlledSends();
  const client = useQueryClient();
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<string[]>("send-controls-drafts-removed", ({ payload }) => {
      const ids = new Set(payload);
      useMailStore.setState((state) => {
        const messages = state.selectedThreadId ? client.getQueryData<{id: string}[]>(threadMessagesQueryKey(state.selectedThreadId)) : undefined;
        return {
          selectedMessageId: ids.has(state.selectedMessageId ?? "") ? null : state.selectedMessageId,
          selectedThreadId: ids.has(state.selectedThreadId ?? "") || (messages?.length && messages.every((message) => ids.has(message.id))) ? null : state.selectedThreadId,
          selectedMessageIds: new Set([...state.selectedMessageIds].filter((id) => !ids.has(id))),
        };
      });
      for (const key of ["messages", "message", "threads", "threadMessages", "folders", "folder-unread-counts"]) void client.invalidateQueries({ queryKey: [key] });
    }).then((fn) => { if (disposed) fn(); else unlisten = fn; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, [client]);
  const [now, setNow] = useState(Date.now);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const pending = query.data?.filter((job) => job.kind === "undo" && job.state === "pending") ?? [];
  useEffect(() => {
    if (!pending.length) return;
    const timer = setInterval(() => setNow(Date.now()), 100);
    return () => clearInterval(timer);
  }, [pending.length]);
  async function undo(job: ControlledSend) {
    if (busy) return;
    setBusy(true); setError(false);
    try {
      const snapshot = await restoreControlledSend(job);
      if (isComposeDirty()) {
        // Preserve any edits made after a lost queue reply; cancellation already succeeded.
        await Promise.allSettled(snapshot.attachments.map((a) => cleanupStagedComposeAttachment(a.path)));
        useToastStore.getState().addToast({ message: t("sendControls.restored"), type: "info" });
      } else if (!openSendSnapshot(snapshot)) setError(true);
      await client.invalidateQueries({ queryKey: SEND_CONTROLS_QUERY });
    } catch { setError(true); }
    finally { setBusy(false); }
  }
  return <>
    {pending.filter((job) => job.id !== composeUndoId).map((job) => <div className="send-controls-banner" role="status" key={job.id}>
      <span>{job.subject || t("sendControls.noSubject")}: {t("sendControls.countdown", { count: countdownSeconds(job.dueAt, now) })}</span>
      <button disabled={busy} onClick={() => void undo(job)}>{t("sendControls.undo")}</button>
    </div>)}
    {query.data?.some((job) => job.state === "review") && <div className="send-controls-banner" role="status">{t("sendControls.reviewNotice")}</div>}
    {error && <div role="alert">{t("sendControls.error")}</div>}
  </>;
}
