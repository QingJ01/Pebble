import type { QueryClient } from "@tanstack/react-query";
import { listFolders, type Folder } from "@/lib/api";
import { getMailMoveContext, type MailMoveContext } from "@/lib/folders-api";
import { folderIdsForSelection, isAllAccountsFolderId } from "@/lib/folderAggregation";
import {
  clearMovedSelection,
  crossesLocalBoundary,
  gmailMutableFolder,
  invalidateFolderViews,
  isMoveDestination,
} from "@/lib/folderManagement";
import { moveMessagesWithUndo } from "@/hooks/mutations/mailActions";
import { useMailStore } from "@/stores/mail.store";
import { toastError, useToastStore } from "@/stores/toast.store";
// The default i18next instance (initialised in src/lib/i18n.ts), without
// pulling react-i18next into this module.
import i18n from "i18next";

export interface DraggedMessage {
  id: string;
  account_id: string;
  is_draft: boolean;
  thread_id?: string | null;
}

export type DropPlan =
  | {
    ok: true;
    accountId: string;
    target: Folder;
    sourceFolderId: string | null;
    sourceByMessage: Record<string, string | undefined>;
  }
  | { ok: false; reasonKey: string; reason: string };

const refuse = (reasonKey: string, reason: string): DropPlan => ({ ok: false, reasonKey, reason });

/**
 * Decide how dropping `messages` on `targetFolderId` moves them, with the
 * same rules as the Move to folder dialog. The source is the list the drag
 * started from (`listFolderId`), so a Gmail message with several labels
 * loses only the one being viewed.
 */
export function planDragMove(
  messages: DraggedMessage[],
  targetFolderId: string,
  folders: Folder[],
  context: MailMoveContext,
  listFolderId: string | null,
): DropPlan {
  const accountId = messages[0]?.account_id;
  if (!accountId) return refuse("folderManagement.chooseFolder", "Nothing to move");
  if (messages.some((m) => m.account_id !== accountId)) {
    return refuse("folderManagement.sameAccount", "Select messages from one account");
  }
  if (messages.some((m) => m.is_draft)) {
    return refuse("folderManagement.draftsProtected", "Drafts cannot be moved; finish editing first");
  }
  const accountFolders = folders.filter((f) => f.account_id === accountId);
  const targetId = isAllAccountsFolderId(targetFolderId)
    ? folderIdsForSelection(targetFolderId, accountFolders, accountId)[0]
    : targetFolderId;
  const target = accountFolders.find((f) => f.id === targetId);
  if (!target) return refuse("folderManagement.dragOtherAccount", "Drop on a folder of the same account");
  if (!isMoveDestination(target, context.provider)) {
    return refuse("folderManagement.dragNotDestination", "Mail can't be moved into this folder");
  }
  const ids = messages.map((m) => m.id);
  const memberships = context.folder_ids_by_message;
  if (ids.some((id) => memberships[id]?.includes(target.id))) {
    return refuse("folderManagement.dragAlreadyThere", "Already in this folder");
  }
  // Source: the folder being viewed, when every message is in it.
  const listed = folderIdsForSelection(listFolderId, accountFolders, accountId);
  const sourceFolderId = listed.find((id) => ids.every((m) => memberships[m]?.includes(id))) ?? null;
  const needsSource = ids.some((id) => (memberships[id]?.length ?? 0) > 1);
  if (needsSource && !sourceFolderId) {
    return refuse("folderManagement.dragChooseSource", "These messages are in several folders; use Move to folder to choose which one to leave");
  }
  const sourceFolder = sourceFolderId ? accountFolders.find((f) => f.id === sourceFolderId) : undefined;
  if (context.provider === "gmail" && sourceFolder && !gmailMutableFolder(sourceFolder)) {
    return refuse("folderManagement.gmailDestination", "Gmail permits moving only between user labels, Inbox, Spam and Trash.");
  }
  const singleSource = (id: string) => (memberships[id]?.length === 1 ? memberships[id][0] : undefined);
  const effectiveSources = ids.map((id) => sourceFolderId ?? singleSource(id));
  for (const id of effectiveSources) {
    const folder = id ? accountFolders.find((f) => f.id === id) : undefined;
    if (crossesLocalBoundary(folder, target, context.provider)) {
      return refuse("folderManagement.dragLocalBoundary", "Mail can't move between server folders and device-only folders");
    }
  }
  return {
    ok: true,
    accountId,
    target,
    sourceFolderId,
    sourceByMessage: Object.fromEntries(ids.map((id) => [id, singleSource(id)])),
  };
}

/**
 * Quick check while hovering a folder, with only what is already loaded
 * (no move context): the rules of [`planDragMove`] that don't depend on
 * which folders each message is in. The drop still runs the full plan.
 */
export function canDropOnFolder(
  messages: DraggedMessage[],
  targetFolderId: string,
  folders: Folder[],
  provider: MailMoveContext["provider"] | undefined,
  listFolderId: string | null,
): boolean {
  const accountId = messages[0]?.account_id;
  if (!accountId || messages.some((m) => m.account_id !== accountId || m.is_draft)) return false;
  const accountFolders = folders.filter((f) => f.account_id === accountId);
  const targetId = isAllAccountsFolderId(targetFolderId)
    ? folderIdsForSelection(targetFolderId, accountFolders, accountId)[0]
    : targetFolderId;
  const target = accountFolders.find((f) => f.id === targetId);
  if (!target || !isMoveDestination(target, provider)) return false;
  const listed = folderIdsForSelection(listFolderId, accountFolders, accountId);
  if (listed.includes(target.id)) return false;
  const source = listed.length === 1 ? accountFolders.find((f) => f.id === listed[0]) : undefined;
  return !crossesLocalBoundary(source, target, provider);
}

/** Move dragged messages onto a sidebar folder, reporting problems as toasts. */
export async function dropMessagesOnFolder(
  client: QueryClient,
  messages: DraggedMessage[],
  targetFolderId: string,
): Promise<void> {
  const accountId = messages[0]?.account_id;
  if (!accountId) return;
  const t = i18n.t.bind(i18n);
  const toast = useToastStore.getState().addToast;
  try {
    const [folders, context] = await Promise.all([
      client.fetchQuery({ queryKey: ["folders", accountId], queryFn: () => listFolders(accountId) }),
      getMailMoveContext(accountId, messages.map((m) => m.id)),
    ]);
    const plan = planDragMove(messages, targetFolderId, folders, context, useMailStore.getState().activeFolderId);
    if (!plan.ok) {
      toast({ message: t(plan.reasonKey, plan.reason), type: "error" });
      return;
    }
    const result = await moveMessagesWithUndo(client, {
      accountId: plan.accountId,
      messageIds: messages.map((m) => m.id),
      sourceFolderId: plan.sourceFolderId,
      targetFolderId: plan.target.id,
      targetFolderName: plan.target.name,
      sourceByMessage: plan.sourceByMessage,
    });
    clearMovedSelection(result.moved_ids, messages);
    invalidateFolderViews(client);
    if (result.failures.length) {
      toast({
        message: t("folderManagement.partial", {
          defaultValue: "{{moved}} moved; {{failed}} failed. Failed messages remain selected.",
          moved: result.moved_ids.length,
          failed: result.failures.length,
        }),
        type: "error",
      });
    }
  } catch (error) {
    invalidateFolderViews(client);
    toastError(error, "folderManagement.moveFailed");
  }
}
