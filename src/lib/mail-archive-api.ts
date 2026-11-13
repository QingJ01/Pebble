import { invoke } from "@tauri-apps/api/core";
import type { EmailAddress } from "./ipc-types";

export interface MailArchive { id: string; account_id: string; account_label: string; name: string }
export interface ArchiveFolder { id: string; archive_id: string; name: string }
export interface ArchiveAttachment { filename: string; mime_type: string; size: number; content_id: string | null; is_inline: boolean; sha256: string }
export interface ArchiveMetadata {
  message_id_header: string | null; subject: string; from_address: string; from_name: string;
  to_list: EmailAddress[]; cc_list: EmailAddress[]; bcc_list: EmailAddress[];
  date: number; body_text: string; has_html: boolean; attachments: ArchiveAttachment[];
}
export interface ArchiveMessageSummary { id: string; folder_id: string; subject: string; sender: string; date: number; is_read: boolean; is_starred: boolean }
export interface ArchiveMessageDetail { id: string; folder_id: string; fingerprint: string; is_read: boolean; is_starred: boolean; metadata: ArchiveMetadata }
export interface ArchiveSelection { id: string; files: { name: string; bytes: number }[] }
export interface ArchiveReport { processed: number; imported: number; duplicates: number; failed: number; cancelled: boolean; errors: string[] }
export interface ArchiveProgress { job_id: string; processed: number; imported: number; duplicates: number; failed: number }
export type ArchiveExportSource =
  | { kind: "archive"; archive_id: string; folder_id: string | null; message_ids: string[] }
  | { kind: "mailbox"; account_id: string; folder_id: string | null; message_ids: string[] };
export type ArchiveExportFormat = "eml" | "mbox";

export const listMailArchives = () => invoke<MailArchive[]>("mail_archive_list");
export const createMailArchive = (accountId: string, name: string) => invoke<MailArchive>("mail_archive_create", { accountId, name });
export const listArchiveFolders = (archiveId: string) => invoke<ArchiveFolder[]>("mail_archive_folders", { archiveId });
export const createArchiveFolder = (archiveId: string, name: string) => invoke<ArchiveFolder>("mail_archive_create_folder", { archiveId, name });
export const listArchiveMessages = (archiveId: string, folderId: string | null, query: string, offset: number) => invoke<ArchiveMessageSummary[]>("mail_archive_messages", { archiveId, folderId, query, offset });
export const readArchiveMessage = (archiveId: string, messageId: string) => invoke<ArchiveMessageDetail>("mail_archive_read", { archiveId, messageId });
export const updateArchiveMessage = (archiveId: string, messageId: string, folderId: string, isRead: boolean, isStarred: boolean) => invoke<void>("mail_archive_update", { archiveId, messageId, folderId, isRead, isStarred });
export const deleteArchiveMessage = (archiveId: string, messageId: string) => invoke<void>("mail_archive_delete", { archiveId, messageId });
export const chooseArchiveFiles = () => invoke<ArchiveSelection | null>("mail_archive_choose_files");
export const importMailArchive = (jobId: string, selectionId: string, archiveId: string, folderId: string, preview: boolean) => invoke<ArchiveReport>("mail_archive_import", { request: { job_id: jobId, selection_id: selectionId, archive_id: archiveId, folder_id: folderId, preview } });
export const cancelArchiveJob = (jobId: string) => invoke<void>("mail_archive_cancel", { jobId });
export const exportMailArchive = (jobId: string, source: ArchiveExportSource, format: ArchiveExportFormat) => invoke<ArchiveReport | null>("mail_archive_export", { request: { job_id: jobId, source, format } });
export const saveArchiveAttachment = (archiveId: string, messageId: string, attachmentIndex: number) => invoke<string | null>("mail_archive_save_attachment", { archiveId, messageId, attachmentIndex });
