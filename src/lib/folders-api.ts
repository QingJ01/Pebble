import { invoke } from "@tauri-apps/api/core";
import type { Folder } from "./api";

export interface MailMoveContext {
  provider: "imap" | "gmail" | "outlook" | "pop3";
  folder_ids_by_message: Record<string, string[]>;
}
export interface MailMoveResult {
  moved_ids: string[];
  failures: { message_id: string; error: string }[];
}
export interface MailFolderParent { remote_id: string; name: string; delimiter: string | null; selectable: boolean; is_system: boolean }
export const getMailFolderParents = (accountId: string) => invoke<MailFolderParent[]>("get_mail_folder_parents",{accountId});
export const createMailFolder = (accountId: string, name: string, parentId: string | null, parentRemoteId: string | null = null) =>
  invoke<Folder>("create_mail_folder", { accountId, name, parentId, parentRemoteId });
export const renameMailFolder = (accountId: string, folderId: string, name: string) =>
  invoke<Folder[]>("rename_mail_folder", { accountId, folderId, name });
export const renameMailFolderNamespace = (accountId:string,remoteId:string,name:string) => invoke<Folder[]>("rename_mail_folder_namespace",{accountId,remoteId,name});
export const getMailMoveContext = (accountId: string, messageIds: string[]) =>
  invoke<MailMoveContext>("get_mail_move_context", { accountId, messageIds });
export const moveMailMessages = (accountId: string, messageIds: string[], sourceFolderId: string | null, targetFolderId: string) =>
  invoke<MailMoveResult>("move_mail_messages", { accountId, messageIds, sourceFolderId, targetFolderId });
