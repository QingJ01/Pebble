/**
 * Undoable mail actions — the single implementation behind the E key, the
 * command palette, row hover actions, row context menus, the detail toolbar
 * and the batch toolbar, so all of them behave the same.
 *
 *   const plan = await planMailAction(client, "trash", messages);
 *   if (plan.needsConfirm && !(await confirmSomehow(plan))) return;
 *   await executeMailAction(client, plan, { onRemoved });
 *
 * or simply `await runMailAction(client, kind, messages, { confirm })`.
 *
 * Before acting, every message's source folder is recorded (from the local
 * folder memberships), so the success toast can offer "Undo", which moves the
 * messages back. Undo is offered only when the action keeps the message in a
 * real folder (archive folder, trash folder, spam folder); permanent deletes
 * (from Trash, local-only drafts) and soft deletes (account without a trash /
 * archive folder) are not undoable and need a confirmation instead.
 */
import type { QueryClient } from "@tanstack/react-query";
// The default i18next instance (initialised in src/lib/i18n.ts); imported
// directly so this module stays free of react-i18next side effects.
import i18n from "i18next";
import type { Folder } from "@/lib/api";
import {
  archiveMessage,
  batchArchive,
  batchDelete,
  deleteMessage,
  listFolders,
  listThreadMessages,
  moveToFolder,
} from "@/lib/api";
import { getMailMoveContext, moveMailMessages, type MailMoveResult } from "@/lib/folders-api";
import { folderIdsForSelection } from "@/lib/folderAggregation";
import { foldersQueryKey } from "@/hooks/queries/useFoldersQuery";
import {
  patchMessagesCache,
  restoreMessagesInCache,
  snapshotMessagesCache,
} from "@/hooks/queries/useMessagesQuery";
import { invalidateKanbanQueries } from "@/hooks/queries/useKanbanCardsQuery";
import { toastError, useToastStore } from "@/stores/toast.store";
import { useMailStore } from "@/stores/mail.store";

export type MailActionKind = "archive" | "trash" | "spam";

export interface MailActionTarget {
  id: string;
  account_id: string;
  is_draft?: boolean;
  remote_id?: string;
}

interface Origin {
  id: string;
  accountId: string;
  sourceFolderId: string;
  /** The message was in more than one folder/label (e.g. Gmail labels). */
  multiFolder: boolean;
}

export interface MailActionPlan {
  kind: MailActionKind;
  messages: MailActionTarget[];
  origins: Map<string, Origin>;
  foldersByAccount: Map<string, Folder[]>;
  /** Target folder for "spam". */
  spamFolderId?: string;
  /** The action can be undone from the toast. */
  undoable: boolean;
  /** At least one message is deleted permanently (Trash / local-only draft). */
  permanent: boolean;
  /** Some messages are deleted permanently and the rest go to Trash. */
  mixedPermanent?: boolean;
  /** Ask before acting: not undoable trash. */
  needsConfirm: boolean;
}

export interface MailActionResult {
  ok: boolean;
  count: number;
  /** Partial success: the messages known to have failed (keep them selected). */
  failedIds?: string[];
}

const MAX_CONTEXT_IDS = 500;

function t(key: string, defaultValue: string, options: Record<string, unknown> = {}): string {
  return i18n.t(key, { defaultValue, ...options }) as string;
}

function groupBy<T>(items: T[], key: (item: T) => string): Map<string, T[]> {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const k = key(item);
    const list = groups.get(k);
    if (list) list.push(item);
    else groups.set(k, [item]);
  }
  return groups;
}

function chunk<T>(items: T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size));
  return out;
}

async function loadFolders(client: QueryClient, accountId: string): Promise<Folder[]> {
  try {
    return await client.fetchQuery({
      queryKey: foldersQueryKey(accountId),
      queryFn: () => listFolders(accountId),
      staleTime: 60_000,
    });
  } catch {
    return [];
  }
}

function folderByRole(folders: Folder[] | undefined, role: Folder["role"]): Folder | undefined {
  return folders?.find((folder) => folder.role === role);
}

function isLocalOnlyDraft(message: MailActionTarget) {
  return Boolean(message.is_draft) && message.remote_id === "";
}

/**
 * Whether move_mail_messages accepts moving between these folders. Mirrors
 * validate_move_route in src-tauri/src/commands/folder_management.rs: Gmail
 * only moves between user labels, Inbox, Spam and Trash (not Sent, Drafts,
 * All Mail…), and server and device-only folders don't mix (except POP3).
 */
export function canManagedMove(provider: string | undefined, source: Folder | undefined, target: Folder | undefined): boolean {
  if (!source || !target || source.id === target.id) return false;
  const local = (f: Folder) => f.remote_id.startsWith("__local_");
  if (provider === "gmail" && !local(target)) {
    const mutable = (f: Folder) => !f.is_system || f.role === "inbox" || f.role === "spam" || f.role === "trash";
    if (!mutable(source) || !mutable(target)) return false;
  }
  if (provider !== "pop3" && local(source) !== local(target)) return false;
  return true;
}

/** Record the folder each message is in now (the folder Undo returns it to). */
async function captureOrigins(
  messages: MailActionTarget[],
  foldersByAccount: Map<string, Folder[]>,
  providers: Map<string, string>,
): Promise<Map<string, Origin>> {
  const origins = new Map<string, Origin>();
  const activeFolderId = useMailStore.getState().activeFolderId;
  for (const [accountId, group] of groupBy(messages, (m) => m.account_id)) {
    const viewFolderIds = folderIdsForSelection(activeFolderId, foldersByAccount.get(accountId) ?? []);
    for (const ids of chunk(group.map((m) => m.id), MAX_CONTEXT_IDS)) {
      try {
        const context = await getMailMoveContext(accountId, ids);
        if (context?.provider) providers.set(accountId, context.provider);
        for (const id of ids) {
          const memberships = context?.folder_ids_by_message?.[id] ?? [];
          const source = memberships.find((folderId) => viewFolderIds.includes(folderId)) ?? memberships[0];
          // Category labels (Gmail Promotions…) aren't folders the user files into.
          const folders = foldersByAccount.get(accountId) ?? [];
          const realMemberships = memberships.filter((folderId) =>
            folders.find((f) => f.id === folderId)?.folder_type !== "category");
          if (source) origins.set(id, { id, accountId, sourceFolderId: source, multiFolder: realMemberships.length > 1 });
        }
      } catch {
        // No origin → no undo for these messages; the action itself still works.
      }
    }
  }
  return origins;
}

export async function planMailAction(
  client: QueryClient,
  kind: MailActionKind,
  messages: MailActionTarget[],
  options: { spamFolderId?: string } = {},
): Promise<MailActionPlan> {
  const accountIds = [...new Set(messages.map((m) => m.account_id))];
  const foldersByAccount = new Map<string, Folder[]>();
  await Promise.all(accountIds.map(async (accountId) => {
    foldersByAccount.set(accountId, await loadFolders(client, accountId));
  }));
  const providers = new Map<string, string>();
  const origins = await captureOrigins(messages, foldersByAccount, providers);
  const folderById = (accountId: string, id: string | undefined) =>
    foldersByAccount.get(accountId)?.find((f) => f.id === id);
  /** Every message can be moved from its origin to the account's `role` folder (and back). */
  const allManagedMovable = (role: Folder["role"]) => messages.every((m) => {
    const origin = origins.get(m.id);
    const source = folderById(m.account_id, origin?.sourceFolderId);
    const target = folderByRole(foldersByAccount.get(m.account_id), role);
    const provider = providers.get(m.account_id);
    // Already there: nothing to move now or on Undo.
    if (source && target && source.id === target.id) return true;
    return canManagedMove(provider, source, target) && canManagedMove(provider, target, source);
  });

  const sourceRole = (message: MailActionTarget) => {
    const origin = origins.get(message.id);
    if (!origin) return undefined;
    return foldersByAccount.get(message.account_id)?.find((f) => f.id === origin.sourceFolderId)?.role ?? null;
  };

  // Archive/trash/spam keep only the target folder locally (the store drops
  // every other membership), so Undo could bring back one label of a
  // multi-label message: no Undo for those rather than a lossy one.
  let undoable = messages.length > 0
    && messages.every((m) => origins.has(m.id) && !origins.get(m.id)!.multiFolder);
  let permanent = false;
  let mixedPermanent = false;

  if (kind === "trash") {
    const isPermanent = (m: MailActionTarget) => isLocalOnlyDraft(m) || sourceRole(m) === "trash";
    permanent = messages.some(isPermanent);
    mixedPermanent = permanent && !messages.every(isPermanent);
    const everyAccountHasTrash = accountIds.every((id) => Boolean(folderByRole(foldersByAccount.get(id), "trash")));
    // Batch trash goes through move_mail_messages, which refuses drafts.
    const draftInBatch = messages.length > 1 && messages.some((m) => m.is_draft);
    // Batches move through move_mail_messages (that's what makes them
    // undoable); when any message can't take that route (e.g. Gmail Sent),
    // the batch uses batch_delete instead, without undo.
    const batchMovable = messages.length === 1 || allManagedMovable("trash");
    undoable = undoable && !permanent && everyAccountHasTrash && !draftInBatch && batchMovable;
  } else if (kind === "archive") {
    // Without an archive folder the message is soft-deleted locally: no undo.
    const everyAccountHasArchive = accountIds.every((id) => Boolean(folderByRole(foldersByAccount.get(id), "archive")));
    const unarchiving = messages.some((m) => sourceRole(m) === "archive");
    const everyAccountHasInbox = accountIds.every((id) => Boolean(folderByRole(foldersByAccount.get(id), "inbox")));
    // Gmail's archive is All Mail, which move_mail_messages refuses: no batch undo there.
    if (messages.length === 1) {
      // archive_message toggles: a message already in Archive goes to Inbox.
      undoable = undoable && everyAccountHasArchive && (!unarchiving || everyAccountHasInbox);
    } else {
      // batch_archive always moves to Archive; Undo moves each one back with
      // move_mail_messages, so that route must accept every message.
      undoable = undoable && everyAccountHasArchive && allManagedMovable("archive");
    }
  } else {
    const undoMovable = messages.length === 1 || allManagedMovable("spam");
    undoable = undoable && Boolean(options.spamFolderId) && undoMovable;
  }

  return {
    kind,
    messages,
    origins,
    foldersByAccount,
    spamFolderId: options.spamFolderId,
    undoable,
    permanent,
    mixedPermanent,
    needsConfirm: kind === "trash" && !undoable,
  };
}

export interface UndoMove {
  accountId: string;
  id: string;
  fromFolderId: string;
  toFolderId: string;
  /**
   * Move back with move_mail_messages (explicit source folder) even for a
   * single message. Used for "Move to folder", whose forward move used it too
   * (keeps Gmail labels symmetric).
   */
  explicitSource?: boolean;
}

function invalidateMailViews(client: QueryClient) {
  void client.invalidateQueries({ queryKey: ["messages"] });
  void client.invalidateQueries({ queryKey: ["threads"] });
  void client.invalidateQueries({ queryKey: ["threadMessages"] });
  void client.invalidateQueries({ queryKey: ["starred-messages"] });
  void client.invalidateQueries({ queryKey: ["folder-unread-counts"] });
  void client.invalidateQueries({ queryKey: ["search"] });
  void invalidateKanbanQueries(client);
}

/** Move messages back to where they were. Returns how many could not be restored. */
export async function undoMoves(client: QueryClient, moves: UndoMove[]): Promise<number> {
  let failed = 0;
  const groups = groupBy(moves, (m) => `${m.accountId}\u0000${m.fromFolderId}\u0000${m.toFolderId}`);
  for (const group of groups.values()) {
    const { accountId, fromFolderId, toFolderId } = group[0];
    try {
      if (group.length === 1 && !group[0].explicitSource) {
        await moveToFolder(group[0].id, toFolderId);
      } else {
        for (const ids of chunk(group.map((m) => m.id), MAX_CONTEXT_IDS)) {
          const result = await moveMailMessages(accountId, ids, fromFolderId, toFolderId);
          failed += result?.failures?.length ?? 0;
        }
      }
    } catch {
      failed += group.length;
    }
  }
  invalidateMailViews(client);
  return failed;
}

function successMessage(plan: MailActionPlan, count: number, unarchived: boolean): string {
  const single = plan.messages.length === 1;
  switch (plan.kind) {
    case "archive":
      if (unarchived) return t("messageActions.unarchiveSuccess", "Message moved to inbox");
      return single
        ? t("messageActions.archiveSuccess", "Message archived")
        : t("batch.archiveSuccess", "{{count}} messages archived", { count });
    case "trash":
      if (plan.mixedPermanent) {
        return t("batch.deleteMixedSuccess", "{{count}} messages deleted: local drafts permanently, the rest moved to Trash", { count });
      }
      if (plan.permanent) {
        return single
          ? t("messageActions.deletePermanentSuccess", "Message permanently deleted")
          : t("batch.deletePermanentSuccess", "{{count}} messages permanently deleted", { count });
      }
      return single
        ? t("messageActions.trashSuccess", "Moved to Trash")
        : t("batch.trashSuccess", "{{count}} messages moved to Trash", { count });
    case "spam":
      return t("messageActions.spamSuccess", "Marked as spam");
  }
}

/** i18n key used by toastError when the error kind has no specific guidance. */
function failureKey(plan: MailActionPlan): string {
  if (plan.kind === "spam") return "messageActions.spamFailed";
  if (plan.messages.length > 1) return "batch.failed";
  return plan.kind === "archive" ? "messageActions.archiveFailed" : "messageActions.deleteFailed";
}

export interface ExecuteOptions {
  /** Called right after the messages were optimistically removed from lists. */
  onRemoved?: (ids: string[]) => void;
  /** Called when the action failed and the lists were restored. */
  onFailed?: (ids: string[]) => void;
}

/** Ids of the plan's messages still in the folder they started in (null = unknown). */
async function idsStillInOrigin(plan: MailActionPlan, ids: string[]): Promise<string[] | null> {
  const stillThere: string[] = [];
  try {
    const byAccount = groupBy(ids.filter((id) => plan.origins.has(id)), (id) => plan.origins.get(id)!.accountId);
    for (const [accountId, group] of byAccount) {
      for (const part of chunk(group, MAX_CONTEXT_IDS)) {
        const context = await getMailMoveContext(accountId, part);
        for (const id of part) {
          if (context?.folder_ids_by_message?.[id]?.includes(plan.origins.get(id)!.sourceFolderId)) stillThere.push(id);
        }
      }
    }
  } catch {
    return null;
  }
  return stillThere;
}

/** Ids of the plan's messages that are now in their account's `role` folder. */
async function idsInRoleFolder(plan: MailActionPlan, role: Folder["role"]): Promise<string[]> {
  const found: string[] = [];
  try {
    for (const [accountId, group] of groupBy(plan.messages, (m) => m.account_id)) {
      const target = folderByRole(plan.foldersByAccount.get(accountId), role)?.id;
      if (!target) continue;
      for (const ids of chunk(group.map((m) => m.id), MAX_CONTEXT_IDS)) {
        const context = await getMailMoveContext(accountId, ids);
        for (const id of ids) {
          if (context?.folder_ids_by_message?.[id]?.includes(target)) found.push(id);
        }
      }
    }
  } catch {
    // Unknown state: offer no undo rather than moving the wrong messages.
    return [];
  }
  return found;
}

export async function executeMailAction(
  client: QueryClient,
  plan: MailActionPlan,
  options: ExecuteOptions = {},
): Promise<MailActionResult> {
  const ids = plan.messages.map((m) => m.id);
  if (ids.length === 0) return { ok: false, count: 0 };
  const idSet = new Set(ids);
  const previousLists = snapshotMessagesCache(client);
  patchMessagesCache(client, (page) => page.filter((m) => !idSet.has(m.id)));
  options.onRemoved?.(ids);

  // Put back only this action's messages (another action may have removed
  // others meanwhile), then refetch so the lists match the server again.
  const rollback = () => {
    restoreMessagesInCache(client, previousLists, ids);
    invalidateMailViews(client);
    options.onFailed?.(ids);
  };

  let count = ids.length;
  let failedIds: string[] | undefined;
  let unarchived = false;
  let undoIds: string[] = ids;
  let currentFolder: (accountId: string) => string | undefined = () => undefined;

  try {
    if (plan.kind === "archive") {
      if (ids.length === 1) {
        const result = await archiveMessage(ids[0]);
        if (result === "skipped") {
          restoreMessagesInCache(client, previousLists, ids);
          return { ok: false, count: 0 };
        }
        unarchived = result === "unarchived";
      } else {
        count = await batchArchive(ids);
        if (count === 0) throw new Error("No messages were archived");
        if (count < ids.length) {
          // batch_archive only reports how many succeeded; look up which ones
          // are now in Archive so Undo moves back only those.
          const archived = await idsInRoleFolder(plan, "archive");
          undoIds = plan.undoable ? archived : [];
          if (archived.length > 0) {
            const archivedSet = new Set(archived);
            failedIds = ids.filter((id) => !archivedSet.has(id));
          }
          invalidateMailViews(client);
          useToastStore.getState().addToast({
            type: "warning",
            message: t("batch.archivePartialFailure", "{{count}} messages could not be archived", { count: ids.length - count }),
          });
        }
      }
      currentFolder = (accountId) => {
        const folders = plan.foldersByAccount.get(accountId);
        return folderByRole(folders, unarchived ? "inbox" : "archive")?.id;
      };
    } else if (plan.kind === "spam") {
      if (!plan.spamFolderId) throw new Error("No spam folder");
      // One move per message: count what really moved, so a failure midway
      // neither reports the moved ones as failed nor loses their Undo.
      const moved: string[] = [];
      let firstError: unknown;
      for (const id of ids) {
        try {
          await moveToFolder(id, plan.spamFolderId);
          moved.push(id);
        } catch (error) {
          firstError ??= error;
        }
      }
      if (moved.length === 0) throw firstError ?? new Error("No messages were marked as spam");
      count = moved.length;
      undoIds = moved;
      if (moved.length < ids.length) {
        const movedSet = new Set(moved);
        const notMoved = ids.filter((id) => !movedSet.has(id));
        failedIds = notMoved;
        restoreMessagesInCache(client, previousLists, notMoved);
        invalidateMailViews(client);
        useToastStore.getState().addToast({
          type: "warning",
          message: t("batch.spamPartialFailure", "{{count}} messages could not be marked as spam", { count: notMoved.length }),
        });
      }
      currentFolder = () => plan.spamFolderId;
    } else if (ids.length === 1) {
      await deleteMessage(ids[0]);
      currentFolder = (accountId) => folderByRole(plan.foldersByAccount.get(accountId), "trash")?.id;
    } else if (plan.undoable) {
      // A real move into each account's Trash keeps local folder membership
      // in sync, which is what makes the batch undoable.
      const moved: string[] = [];
      let failures = 0;
      const groups = groupBy(plan.messages, (m) => `${m.account_id}\u0000${plan.origins.get(m.id)!.sourceFolderId}`);
      for (const group of groups.values()) {
        const accountId = group[0].account_id;
        const source = plan.origins.get(group[0].id)!.sourceFolderId;
        const trash = folderByRole(plan.foldersByAccount.get(accountId), "trash")!;
        for (const batch of chunk(group.map((m) => m.id), MAX_CONTEXT_IDS)) {
          const result = await moveMailMessages(accountId, batch, source, trash.id);
          moved.push(...(result?.moved_ids ?? []));
          failures += result?.failures?.length ?? 0;
        }
      }
      if (moved.length === 0 && failures > 0) throw new Error("No messages were moved to Trash");
      count = moved.length;
      undoIds = moved;
      if (failures > 0) {
        const movedSet = new Set(moved);
        failedIds = ids.filter((id) => !movedSet.has(id));
      }
      currentFolder = (accountId) => folderByRole(plan.foldersByAccount.get(accountId), "trash")?.id;
      if (failures > 0) {
        // Failed ones stay where they were; bring them back into the lists.
        invalidateMailViews(client);
        useToastStore.getState().addToast({
          type: "warning",
          message: t("batch.partialFailure", "{{count}} messages could not be moved", { count: failures }),
        });
      }
    } else {
      count = await batchDelete(ids);
      if (count === 0) throw new Error("No messages were deleted");
      if (count < ids.length) {
        failedIds = (await idsStillInOrigin(plan, ids)) ?? undefined;
        invalidateMailViews(client);
        useToastStore.getState().addToast({
          type: "warning",
          message: t("batch.deletePartialFailure", "{{count}} messages could not be deleted", { count: ids.length - count }),
        });
      }
    }
  } catch (error) {
    rollback();
    toastError(error, failureKey(plan));
    return { ok: false, count: 0 };
  }

  invalidateMailViews(client);

  const undoMovesList: UndoMove[] = [];
  if (plan.undoable) {
    for (const id of undoIds) {
      const origin = plan.origins.get(id);
      if (!origin) continue;
      const from = currentFolder(origin.accountId);
      if (!from || from === origin.sourceFolderId) continue;
      undoMovesList.push({ accountId: origin.accountId, id, fromFolderId: from, toFolderId: origin.sourceFolderId });
    }
  }

  showUndoableToast(client, successMessage(plan, count, unarchived), undoMovesList);
  return { ok: true, count, ...(failedIds?.length ? { failedIds } : {}) };
}

/**
 * Success toast with an "Undo" action that moves the messages back (no action
 * when `moves` is empty). The one undo implementation for every mail action.
 */
export function showUndoableToast(client: QueryClient, message: string, moves: UndoMove[]): string {
  return useToastStore.getState().addToast({
    type: "success",
    message,
    action: moves.length > 0
      ? {
          label: t("common.undo", "Undo"),
          onClick: () => {
            // Visible until the move back finishes, so a slow or stuck Undo
            // doesn't look like a click that did nothing.
            const pending = useToastStore.getState().addToast({
              type: "info",
              message: t("messageActions.undoing", "Undoing…"),
              duration: 30_000,
              // One indicator per Undo: merged ones would vanish when the first finishes.
              mergeable: false,
            });
            void undoMoves(client, moves).then((failed) => {
              useToastStore.getState().removeToast(pending);
              if (failed === 0) {
                useToastStore.getState().addToast({ type: "success", message: t("messageActions.undoSuccess", "Undone") });
              } else {
                useToastStore.getState().addToast({
                  type: "error",
                  message: t("messageActions.undoFailed", "Couldn't undo for {{count}} messages", { count: failed }),
                });
              }
            });
          },
        }
      : undefined,
  });
}

export interface MoveToFolderRequest {
  accountId: string;
  messageIds: string[];
  /** The folder (or Gmail label) to move out of; null lets the backend pick the only one. */
  sourceFolderId: string | null;
  targetFolderId: string;
  /** Shown in the success toast. */
  targetFolderName?: string;
  /** Folder each message is in now, used when `sourceFolderId` is null. */
  sourceByMessage?: Record<string, string | undefined>;
}

/**
 * "Move to folder": move, then offer Undo (moves the moved ones back to where
 * they came from). Throws when the whole move failed; partial failures are in
 * the result for the caller to show.
 */
export async function moveMessagesWithUndo(client: QueryClient, request: MoveToFolderRequest): Promise<MailMoveResult> {
  const { accountId, messageIds, sourceFolderId, targetFolderId } = request;
  const result = await moveMailMessages(accountId, messageIds, sourceFolderId, targetFolderId);
  const movedIds = result?.moved_ids ?? [];
  if (movedIds.length === 0) return result;
  const moves: UndoMove[] = [];
  for (const id of movedIds) {
    const origin = sourceFolderId ?? request.sourceByMessage?.[id];
    if (!origin || origin === targetFolderId) continue;
    moves.push({ accountId, id, fromFolderId: targetFolderId, toFolderId: origin, explicitSource: true });
  }
  const message = request.targetFolderName
    ? t("folderManagement.movedTo", "Moved to “{{folder}}”", {
      folder: request.targetFolderName,
      interpolation: { escapeValue: false },
    })
    : t("folderManagement.moved", "Moved");
  showUndoableToast(client, message, moves);
  return result;
}

export interface RunOptions extends ExecuteOptions {
  spamFolderId?: string;
  /** Asked when the action cannot be undone. Resolve false to cancel. */
  confirm?: (plan: MailActionPlan) => Promise<boolean>;
}

// Message ids with an action still being planned or run. Archive toggles on the
// backend (an archived message goes back to the inbox), so a second E press or
// palette Archive while the first is in flight must not act on it again.
const actionsInFlight = new Set<string>();

/** Plan, confirm if needed, then execute. */
export async function runMailAction(
  client: QueryClient,
  kind: MailActionKind,
  messages: MailActionTarget[],
  options: RunOptions = {},
): Promise<MailActionResult> {
  const targets = messages.filter((m) => !actionsInFlight.has(m.id));
  // Messages still busy with another action are left alone; report them as
  // not done so callers keep them selected instead of silently dropping them.
  const busyIds = messages.filter((m) => actionsInFlight.has(m.id)).map((m) => m.id);
  if (targets.length === 0) return { ok: false, count: 0, ...(busyIds.length ? { failedIds: busyIds } : {}) };
  for (const m of targets) actionsInFlight.add(m.id);
  try {
    const plan = await planMailAction(client, kind, targets, { spamFolderId: options.spamFolderId });
    if (plan.needsConfirm && options.confirm && !(await options.confirm(plan))) {
      return { ok: false, count: 0 };
    }
    const result = await executeMailAction(client, plan, options);
    if (!result.ok || busyIds.length === 0) return result;
    return { ...result, failedIds: [...(result.failedIds ?? []), ...busyIds] };
  } finally {
    for (const m of targets) actionsInFlight.delete(m.id);
  }
}

/** Standard confirmation copy for a trash plan that cannot be undone. */
export function trashConfirmCopy(plan: MailActionPlan) {
  const count = plan.messages.length;
  if (count === 1) {
    const draft = isLocalOnlyDraft(plan.messages[0]);
    return {
      title: plan.permanent
        ? t("messageActions.deletePermanentTitle", "Delete permanently?")
        : t("messageActions.delete", "Delete"),
      message: draft
        ? t("messageActions.deleteLocalDraftConfirm", "Permanently delete this local draft? This cannot be undone.")
        : plan.permanent
          ? t("messageActions.deletePermanentConfirm", "Permanently delete this message? This cannot be undone.")
          : t("messageActions.deleteConfirm", "Move this message to trash?"),
      confirmLabel: plan.permanent || draft
        ? t("messageActions.deletePermanently", "Delete permanently")
        : t("messageActions.moveToTrash", "Move to Trash"),
    };
  }
  const includesLocalDraft = plan.messages.some(isLocalOnlyDraft);
  return {
    title: t("batch.deleteTitle", "Delete {{count}} messages?", { count }),
    message: includesLocalDraft
      ? t("batch.deleteWithLocalDraftsConfirm",
        "Local-only drafts in this selection will be permanently deleted and cannot be restored. Other messages will move to Trash.")
      : plan.permanent
        ? t("batch.deletePermanentConfirm", "Permanently delete {{count}} messages? This cannot be undone.", { count })
        : t("batch.deleteConfirm", "Move {{count}} messages to Trash? You can restore them from Trash.", { count }),
    confirmLabel: plan.permanent
      ? t("messageActions.deletePermanently", "Delete permanently")
      : t("messageActions.moveToTrash", "Move to Trash"),
  };
}

/**
 * The messages of a thread that live in the folders being viewed (a thread can
 * also contain sent mail etc. that a folder action must not touch).
 */
export async function scopedThreadMessages(
  accountId: string,
  threadId: string,
  folderIds: string[],
): Promise<MailActionTarget[]> {
  const messages = await listThreadMessages(accountId, threadId);
  if (folderIds.length === 0 || messages.length === 0) return messages;
  const scoped: MailActionTarget[] = [];
  for (const [account, group] of groupBy(messages, (m) => m.account_id)) {
    for (const part of chunk(group, MAX_CONTEXT_IDS)) {
      const context = await getMailMoveContext(account, part.map((m) => m.id));
      for (const message of part) {
        const memberships = context?.folder_ids_by_message?.[message.id] ?? [];
        if (memberships.some((id) => folderIds.includes(id))) scoped.push(message);
      }
    }
  }
  return scoped;
}
