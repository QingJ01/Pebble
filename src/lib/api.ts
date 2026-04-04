import { invoke } from "@tauri-apps/api/core";

// ─── Types ────────────────────────────────────────────────────────────────────

export interface Account {
  id: string;
  email: string;
  display_name: string;
  provider: "imap" | "gmail" | "outlook";
  created_at: number;
  updated_at: number;
}

export interface Folder {
  id: string;
  account_id: string;
  remote_id: string;
  name: string;
  folder_type: "folder" | "label" | "category";
  role: "inbox" | "sent" | "drafts" | "trash" | "archive" | "spam" | null;
  parent_id: string | null;
  color: string | null;
  is_system: boolean;
  sort_order: number;
}

export interface EmailAddress {
  name: string | null;
  address: string;
}

export interface Message {
  id: string;
  account_id: string;
  remote_id: string;
  message_id_header: string | null;
  in_reply_to: string | null;
  references_header: string | null;
  thread_id: string | null;
  subject: string;
  snippet: string;
  from_address: string;
  from_name: string;
  to_list: EmailAddress[];
  cc_list: EmailAddress[];
  bcc_list: EmailAddress[];
  body_text: string;
  body_html_raw: string;
  has_attachments: boolean;
  is_read: boolean;
  is_starred: boolean;
  is_draft: boolean;
  date: number;
  remote_version: string | null;
  is_deleted: boolean;
  deleted_at: number | null;
  created_at: number;
  updated_at: number;
}

export interface RenderedHtml {
  html: string;
  trackers_blocked: { domain: string; tracker_type: string }[];
  images_blocked: number;
}

export interface SearchHit {
  message_id: string;
  score: number;
  snippet: string;
}

export type PrivacyMode = "Strict" | { TrustSender: string } | "LoadOnce";

export interface AddAccountRequest {
  email: string;
  display_name: string;
  provider: string;
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  username: string;
  password: string;
  use_tls: boolean;
}

// ─── API Functions ────────────────────────────────────────────────────────────

export async function healthCheck(): Promise<string> {
  return invoke<string>("health_check");
}

export async function addAccount(request: AddAccountRequest): Promise<Account> {
  return invoke<Account>("add_account", { request });
}

export async function listAccounts(): Promise<Account[]> {
  return invoke<Account[]>("list_accounts");
}

export async function deleteAccount(accountId: string): Promise<void> {
  return invoke<void>("delete_account", { account_id: accountId });
}

export async function listFolders(accountId: string): Promise<Folder[]> {
  return invoke<Folder[]>("list_folders", { account_id: accountId });
}

export async function listMessages(
  folderId: string,
  limit: number,
  offset: number,
): Promise<Message[]> {
  return invoke<Message[]>("list_messages", {
    folder_id: folderId,
    limit,
    offset,
  });
}

export async function getMessage(messageId: string): Promise<Message | null> {
  return invoke<Message | null>("get_message", { message_id: messageId });
}

export async function getRenderedHtml(
  messageId: string,
  privacyMode: PrivacyMode,
): Promise<RenderedHtml> {
  return invoke<RenderedHtml>("get_rendered_html", {
    message_id: messageId,
    privacy_mode: privacyMode,
  });
}

export async function updateMessageFlags(
  messageId: string,
  isRead?: boolean,
  isStarred?: boolean,
): Promise<void> {
  return invoke<void>("update_message_flags", {
    message_id: messageId,
    is_read: isRead,
    is_starred: isStarred,
  });
}

export async function searchMessages(
  query: string,
  limit?: number,
): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search_messages", { query, limit });
}

export async function startSync(accountId: string): Promise<string> {
  return invoke<string>("start_sync", { account_id: accountId });
}

export async function stopSync(accountId: string): Promise<void> {
  return invoke<void>("stop_sync", { account_id: accountId });
}
