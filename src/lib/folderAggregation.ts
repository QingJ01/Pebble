import type { Folder } from "@/lib/api";

export const ALL_ACCOUNTS_ID = "all";
export const ALL_ACCOUNTS_FOLDER_PREFIX = "all:";
export const ALL_ACCOUNTS_SELECT_VALUE = "__all_accounts__";

export type FolderRole = NonNullable<Folder["role"]>;

const SYSTEM_ROLE_ORDER: FolderRole[] = ["inbox", "sent", "archive", "drafts", "trash", "spam"];

const SYSTEM_ROLE_NAMES: Record<FolderRole, string> = {
  inbox: "Inbox",
  sent: "Sent",
  archive: "Archive",
  drafts: "Drafts",
  trash: "Trash",
  spam: "Spam",
};

export function allAccountsFolderId(role: FolderRole): string {
  return `${ALL_ACCOUNTS_FOLDER_PREFIX}${role}`;
}

export function isAllAccountsFolderId(folderId: string | null | undefined): boolean {
  return !!folderId && folderId.startsWith(ALL_ACCOUNTS_FOLDER_PREFIX);
}

export function roleFromAllAccountsFolderId(folderId: string | null | undefined): FolderRole | null {
  if (!folderId || !isAllAccountsFolderId(folderId)) return null;
  const role = folderId.slice(ALL_ACCOUNTS_FOLDER_PREFIX.length);
  return SYSTEM_ROLE_ORDER.includes(role as FolderRole) ? role as FolderRole : null;
}

export function buildAllAccountsFolders(folders: Folder[]): Folder[] {
  const roles = new Set(folders.map((folder) => folder.role).filter(Boolean) as FolderRole[]);
  const virtualFolders = SYSTEM_ROLE_ORDER
    .filter((role) => roles.has(role))
    .map((role, index): Folder => ({
      id: allAccountsFolderId(role),
      account_id: ALL_ACCOUNTS_ID,
      remote_id: allAccountsFolderId(role),
      name: SYSTEM_ROLE_NAMES[role],
      folder_type: "folder",
      role,
      parent_id: null,
      color: null,
      is_system: true,
      sort_order: index,
    }));

  const customFolders = folders
    .filter((folder) => !folder.role)
    .sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name));

  return [...virtualFolders, ...customFolders];
}

export function sortFoldersForSidebar(folders: Folder[]): Folder[] {
  const firstFolderByRole = new Map<FolderRole, Folder>();
  const customFolders: Folder[] = [];

  for (const folder of folders) {
    if (!folder.role) {
      customFolders.push(folder);
      continue;
    }
    if (!firstFolderByRole.has(folder.role)) {
      firstFolderByRole.set(folder.role, folder);
    }
  }

  const systemFolders = SYSTEM_ROLE_ORDER.flatMap((role) => {
    const folder = firstFolderByRole.get(role);
    return folder ? [folder] : [];
  });

  return [
    ...systemFolders,
    ...customFolders.sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name)),
  ];
}

export function folderIdsForSelection(folderId: string | null, folders: Folder[]): string[] {
  if (!folderId) return [];
  const role = roleFromAllAccountsFolderId(folderId) ?? folders.find((folder) => folder.id === folderId)?.role;
  if (!role) return folders.some((folder) => folder.id === folderId) ? [folderId] : [];
  return folders.filter((folder) => folder.role === role).map((folder) => folder.id);
}

export function roleForSelection(folderId: string | null, folders: Folder[]): Folder["role"] {
  return roleFromAllAccountsFolderId(folderId) ?? folders.find((folder) => folder.id === folderId)?.role ?? null;
}

export function unreadCountForFolder(
  folderId: string,
  folders: Folder[],
  countsByFolderId: Record<string, number>,
): number {
  const matchingFolderIds = folderIdsForSelection(folderId, folders);
  return matchingFolderIds.reduce((sum, id) => sum + (countsByFolderId[id] ?? 0), 0);
}
