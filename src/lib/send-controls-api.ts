import { invoke } from "@tauri-apps/api/core";
import type { EditorMode } from "@/hooks/useComposeEditor";
import type { ComposeMode } from "@/stores/compose.store";
import type { ComposeAttachment } from "@/features/compose/compose-draft";

export interface SendSnapshot {
  accountId: string;
  to: string[]; cc: string[]; bcc: string[];
  subject: string; bodyText: string; bodyHtml?: string;
  inReplyTo?: string;
  editorMode: EditorMode; composeMode: ComposeMode;
  rawSource: string; richTextHtml: string; quotedReplyHtml: string;
  attachments: ComposeAttachment[];
}

export interface ControlledSend {
  id: string; accountId: string; subject: string;
  dueAt: number; revision: number;
  state: "pending" | "dispatching" | "review" | "draft" | "sent";
  kind: "undo" | "scheduled";
}

export function queueControlledSend(snapshot: SendSnapshot, timing: { dueAt: number } | { undoSeconds: number }, requestId: string) {
  return invoke<ControlledSend>("queue_controlled_send", { snapshot, requestId, ...timing });
}
export const listControlledSends = () => invoke<ControlledSend[]>("list_controlled_sends");
export const restoreControlledSend = (job: ControlledSend) => invoke<SendSnapshot>("restore_controlled_send", { id: job.id, revision: job.revision });
export const rescheduleControlledSend = (job: ControlledSend, dueAt: number) => invoke<void>("reschedule_controlled_send", { id: job.id, revision: job.revision, dueAt });
export const deleteControlledSend = (job: ControlledSend) => invoke<void>("delete_controlled_send", { id: job.id, revision: job.revision });
export const resolveSendControlExit = (confirm: boolean) => invoke<void>("resolve_send_control_exit", { confirm });
export const sendControlExitPending = () => invoke<boolean>("send_control_exit_pending");
