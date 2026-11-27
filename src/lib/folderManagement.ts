import type { Folder } from "./api";
import type { QueryClient } from "@tanstack/react-query";
import { isThreadSelected, useMailStore } from "@/stores/mail.store";

export interface FolderEntry { folder: Folder; depth: number; path: string }
/** Providers can return parent remote IDs before their local ID is known. */
export function folderTree(folders: Folder[]): FolderEntry[] {
  const byId = new Map(folders.flatMap((f) => [[f.id, f], [f.remote_id, f]]));
  const result = folders.map((folder) => {
    const parents: Folder[] = [];
    const seen = new Set([folder.id]);
    let parent = folder.parent_id ? byId.get(folder.parent_id) : undefined;
    while (parent && parent.account_id === folder.account_id && !seen.has(parent.id)) {
      seen.add(parent.id); parents.unshift(parent);
      parent = parent.parent_id ? byId.get(parent.parent_id) : undefined;
    }
    // IMAP/Gmail names already include their path. Keep those verbatim, while
    // Graph and local folders have independent display names at each level.
    let path = folder.name;
    for (const p of [...parents].reverse()) {
      if (!path.startsWith(`${p.name}/`) && !path.startsWith(`${p.name}.`)) path = `${p.name} / ${path}`;
    }
    return { folder, depth: parents.length, path };
  });
  return result.sort((a,b) => a.path.localeCompare(b.path));
}

/** Gmail moves only between user labels, Inbox, Spam, Trash and local folders. */
export function gmailMutableFolder(folder: Folder): boolean {
  return !folder.is_system || folder.remote_id.startsWith("__local_") || ["inbox", "spam", "trash"].includes(folder.role ?? "");
}

/** Whether mail can be moved into `folder` at all (for this provider). */
export function isMoveDestination(folder: Folder, provider: string | undefined): boolean {
  return folder.role !== "drafts"
    && folder.remote_id !== "__local_outbox__"
    && folder.folder_type !== "category"
    && (provider !== "gmail" || gmailMutableFolder(folder));
}

/** Moves between server folders and device-only folders are not supported (except POP3). */
export function crossesLocalBoundary(source: Folder | undefined, target: Folder, provider: string | undefined): boolean {
  return provider !== "pop3" && !!source
    && source.remote_id.startsWith("__local_") !== target.remote_id.startsWith("__local_");
}

export function renameDisabledReason(folder: Folder | undefined, folders: Folder[], provider: string): string | null {
  if (!folder) return "folderManagement.chooseFolder";
  if ((folder.is_system && provider !== "imap") || folder.role || (folder.remote_id.startsWith("__local_") && !folder.remote_id.startsWith("__local_folder_"))) return "folderManagement.systemProtected";
  if (provider === "gmail" && folders.some((f) => f.name.startsWith(`${folder.name}/`))) return "folderManagement.gmailParentRename";
  return null;
}

export function invalidateFolderViews(client: QueryClient) {
  for (const key of ["folders", "managed-folder-parents", "folder-unread-counts", "messages", "message", "threads", "threadMessages", "starred-messages", "message-labels", "pendingMailOpsList", "pendingMailOps", "mail-move-context", "search"]) {
    void client.invalidateQueries({ queryKey: [key] });
  }
}

export function clearMovedSelection(
  movedIds: string[],
  selection: { id: string; thread_id?: string | null; account_id?: string | null }[],
) {
  if (!movedIds.length) return;
  const moved = new Set(movedIds);
  const state = useMailStore.getState();
  const selectedThread = selection.filter((message) =>
    isThreadSelected(state, { thread_id: message.thread_id ?? message.id, account_id: message.account_id }));
  useMailStore.setState({
    selectedMessageIds: new Set([...state.selectedMessageIds].filter((id) => !moved.has(id))),
    ...(state.selectedMessageId && moved.has(state.selectedMessageId) ? { selectedMessageId: null } : {}),
    ...(selectedThread.length && selectedThread.every((message) => moved.has(message.id)) ? { selectedThreadId: null } : {}),
  });
}
