import { invoke } from "@tauri-apps/api/core";
import { retryQueue } from "./retry-queue";

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

/** Lightweight message data for list views (no body fields). */
export interface MessageSummary {
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

/** Full message including body content. */
export interface Message extends MessageSummary {
  body_text: string;
  body_html_raw: string;
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
  subject?: string;
  from_address?: string;
  date?: number;
}

export type PrivacyMode = "Strict" | { TrustSender: string } | "LoadOnce" | "Off";

export type ConnectionSecurity = "tls" | "starttls" | "plain";

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
  imap_security: ConnectionSecurity;
  smtp_security: ConnectionSecurity;
  proxy_host?: string;
  proxy_port?: number;
}

// ─── Retry Wrapper ───────────────────────────────────────────────────────────

export function withRetry(fn: () => Promise<void>): void {
  retryQueue.enqueue(fn);
}

// ─── API Functions ────────────────────────────────────────────────────────────

export async function healthCheck(): Promise<string> {
  return invoke<string>("health_check");
}

export async function completeOAuthFlow(
  provider: string,
  email: string,
  displayName: string,
): Promise<Account> {
  return invoke<Account>("complete_oauth_flow", { provider, email, displayName });
}

export async function addAccount(request: AddAccountRequest): Promise<Account> {
  return invoke<Account>("add_account", { request });
}

export async function testAccountConnection(accountId: string): Promise<string> {
  return invoke<string>("test_account_connection", { accountId });
}

export async function testImapConnection(
  imapHost: string,
  imapPort: number,
  imapSecurity: ConnectionSecurity,
  proxyHost?: string,
  proxyPort?: number,
  username?: string,
  password?: string,
): Promise<string> {
  return invoke<string>("test_imap_connection", {
    request: { imap_host: imapHost, imap_port: imapPort, imap_security: imapSecurity, proxy_host: proxyHost, proxy_port: proxyPort, username, password },
  });
}

export async function listAccounts(): Promise<Account[]> {
  return invoke<Account[]>("list_accounts");
}

export async function updateAccount(
  accountId: string,
  email: string,
  displayName: string,
  password?: string,
  imapHost?: string,
  imapPort?: number,
  smtpHost?: string,
  smtpPort?: number,
  imapSecurity?: ConnectionSecurity,
  smtpSecurity?: ConnectionSecurity,
  proxyHost?: string,
  proxyPort?: number,
): Promise<void> {
  return invoke<void>("update_account", {
    accountId, email, displayName, password,
    imapHost, imapPort, smtpHost, smtpPort, imapSecurity, smtpSecurity,
    proxyHost, proxyPort,
  });
}

export async function deleteAccount(accountId: string): Promise<void> {
  return invoke<void>("delete_account", { accountId });
}

export async function listFolders(accountId: string): Promise<Folder[]> {
  return invoke<Folder[]>("list_folders", { accountId });
}

export async function listMessages(
  folderId: string,
  limit: number,
  offset: number,
  folderIds?: string[],
): Promise<MessageSummary[]> {
  return invoke<MessageSummary[]>("list_messages", { folderId, folderIds, limit, offset });
}

export async function listStarredMessages(
  accountId: string,
  limit: number,
  offset: number,
): Promise<MessageSummary[]> {
  return invoke<MessageSummary[]>("list_starred_messages", { accountId, limit, offset });
}

export async function getMessage(messageId: string): Promise<Message | null> {
  return invoke<Message | null>("get_message", { messageId });
}

/** Batch-fetch multiple messages in a single IPC call. */
export async function getMessagesBatch(messageIds: string[]): Promise<Message[]> {
  return invoke<Message[]>("get_messages_batch", { messageIds });
}

export async function getRenderedHtml(
  messageId: string,
  privacyMode: PrivacyMode,
): Promise<RenderedHtml> {
  return invoke<RenderedHtml>("get_rendered_html", { messageId, privacyMode });
}

/** Single IPC call that returns both Message and RenderedHtml. */
export async function getMessageWithHtml(
  messageId: string,
  privacyMode: PrivacyMode,
): Promise<[Message, RenderedHtml] | null> {
  return invoke<[Message, RenderedHtml] | null>("get_message_with_html", { messageId, privacyMode });
}

export async function updateMessageFlags(
  messageId: string,
  isRead?: boolean,
  isStarred?: boolean,
): Promise<void> {
  return invoke<void>("update_message_flags", { messageId, isRead, isStarred });
}

const archivingIds = new Set<string>();

export async function archiveMessage(messageId: string): Promise<string> {
  if (archivingIds.has(messageId)) {
    return "skipped";
  }
  archivingIds.add(messageId);
  try {
    return await invoke<string>("archive_message", { messageId });
  } finally {
    archivingIds.delete(messageId);
  }
}

export async function deleteMessage(messageId: string): Promise<void> {
  return invoke<void>("delete_message", { messageId });
}

export async function restoreMessage(messageId: string): Promise<void> {
  return invoke<void>("restore_message", { messageId });
}

export async function emptyTrash(accountId: string): Promise<number> {
  return invoke<number>("empty_trash", { accountId });
}

// ─── Trusted Senders ────────────────────────────────────────────────────────

export interface TrustedSender {
  account_id: string;
  email: string;
  trust_type: "images" | "all";
  created_at: number;
}

export async function listTrustedSenders(accountId: string): Promise<TrustedSender[]> {
  return invoke<TrustedSender[]>("list_trusted_senders", { accountId });
}

export async function removeTrustedSender(accountId: string, email: string): Promise<void> {
  return invoke<void>("remove_trusted_sender", { accountId, email });
}

export async function searchMessages(
  query: string,
  limit?: number,
): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search_messages", { query, limit });
}

// ─── Advanced Search ─────────────────────────────────────────────────────────

export interface AdvancedSearchQuery {
  text?: string;
  from?: string;
  to?: string;
  subject?: string;
  dateFrom?: number;
  dateTo?: number;
  hasAttachment?: boolean;
  folderId?: string;
}

export async function advancedSearch(
  query: AdvancedSearchQuery,
  limit?: number,
): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("advanced_search", { query, limit });
}

export async function startSync(accountId: string, pollIntervalSecs?: number): Promise<string> {
  return invoke<string>("start_sync", { accountId, pollIntervalSecs: pollIntervalSecs ?? null });
}

export async function stopSync(accountId: string): Promise<void> {
  return invoke<void>("stop_sync", { accountId });
}

// ─── Attachment Types ─────────────────────────────────────────────────────────

export interface Attachment {
  id: string;
  message_id: string;
  filename: string;
  mime_type: string;
  size: number;
  content_id: string | null;
  is_inline: boolean;
}

// ─── Attachment API ──────────────────────────────────────────────────────────

export async function listAttachments(messageId: string): Promise<Attachment[]> {
  return invoke<Attachment[]>("list_attachments", { messageId });
}

export async function getAttachmentPath(attachmentId: string): Promise<string | null> {
  return invoke<string | null>("get_attachment_path", { attachmentId });
}

export async function downloadAttachment(attachmentId: string, saveTo: string): Promise<void> {
  return invoke<void>("download_attachment", { attachmentId, saveTo });
}

// ─── Kanban Types ─────────────────────────────────────────────────────────────

export type KanbanColumnType = "todo" | "waiting" | "done";

export interface KanbanCard {
  message_id: string;
  column: KanbanColumnType;
  position: number;
  created_at: number;
  updated_at: number;
}

// ─── Snooze Types ─────────────────────────────────────────────────────────────

export interface SnoozedMessage {
  message_id: string;
  snoozed_at: number;
  unsnoozed_at: number;
  return_to: string;
}

// ─── Rule Types ───────────────────────────────────────────────────────────────

export interface Rule {
  id: string;
  name: string;
  priority: number;
  conditions: string;
  actions: string;
  is_enabled: boolean;
  created_at: number;
  updated_at: number;
}

// ─── Kanban API ───────────────────────────────────────────────────────────────

export async function moveToKanban(messageId: string, column: KanbanColumnType, position?: number): Promise<void> {
  return invoke<void>("move_to_kanban", { messageId, column, position });
}

export async function listKanbanCards(column?: KanbanColumnType): Promise<KanbanCard[]> {
  return invoke<KanbanCard[]>("list_kanban_cards", { column });
}

export async function removeFromKanban(messageId: string): Promise<void> {
  return invoke<void>("remove_from_kanban", { messageId });
}

// ─── Snooze API ───────────────────────────────────────────────────────────────

export async function snoozeMessage(messageId: string, until: number, returnTo: string): Promise<void> {
  return invoke<void>("snooze_message", { messageId, until, returnTo });
}

export async function unsnoozeMessage(messageId: string): Promise<void> {
  return invoke<void>("unsnooze_message", { messageId });
}

export async function listSnoozed(): Promise<SnoozedMessage[]> {
  return invoke<SnoozedMessage[]>("list_snoozed");
}

// ─── Rules API ────────────────────────────────────────────────────────────────

export async function createRule(name: string, priority: number, conditions: string, actions: string): Promise<Rule> {
  return invoke<Rule>("create_rule", { name, priority, conditions, actions });
}

export async function listRules(): Promise<Rule[]> {
  return invoke<Rule[]>("list_rules");
}

export async function updateRule(rule: Rule): Promise<void> {
  return invoke<void>("update_rule", { rule });
}

export async function deleteRule(ruleId: string): Promise<void> {
  return invoke<void>("delete_rule", { ruleId });
}

// ─── Compose API ──────────────────────────────────────────────────────────────

export async function sendEmail(
  accountId: string,
  to: string[],
  cc: string[],
  bcc: string[],
  subject: string,
  bodyText: string,
  bodyHtml?: string,
  inReplyTo?: string,
  attachmentPaths?: string[],
): Promise<void> {
  return invoke<void>("send_email", {
    accountId, to, cc, bcc, subject, bodyText, bodyHtml, inReplyTo, attachmentPaths,
  });
}

// ─── Batch Operations ────────────────────────────────────────────────────────

export async function batchArchive(messageIds: string[]): Promise<number> {
  return invoke<number>("batch_archive", { messageIds });
}

export async function batchDelete(messageIds: string[]): Promise<number> {
  return invoke<number>("batch_delete", { messageIds });
}

export async function batchMarkRead(messageIds: string[], isRead: boolean): Promise<number> {
  return invoke<number>("batch_mark_read", { messageIds, isRead });
}

// ─── Trusted Senders API ─────────────────────────────────────────────────────

export async function trustSender(accountId: string, email: string, trustType: "images" | "all"): Promise<void> {
  return invoke<void>("trust_sender", { accountId, email, trustType });
}

export async function isTrustedSender(accountId: string, email: string): Promise<boolean> {
  return invoke<boolean>("is_trusted_sender", { accountId, email });
}

// ─── Translate API ───────────────────────────────────────────────────────────

export interface TranslateConfig {
  id: string;
  provider_type: string;
  config: string;
  is_enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface TranslateResult {
  translated: string;
  segments: { source: string; target: string }[];
}

export async function translateText(text: string, fromLang: string, toLang: string): Promise<TranslateResult> {
  return invoke<TranslateResult>("translate_text", { text, fromLang, toLang });
}

export async function getTranslateConfig(): Promise<TranslateConfig | null> {
  return invoke<TranslateConfig | null>("get_translate_config");
}

export async function saveTranslateConfig(providerType: string, config: string, isEnabled: boolean): Promise<void> {
  return invoke<void>("save_translate_config", { providerType, config, isEnabled });
}

export async function testTranslateConnection(config: string): Promise<string> {
  return invoke<string>("test_translate_connection", { config });
}

// ─── Thread API ─────────────────────────────────────────────────────────────

export interface ThreadSummary {
  thread_id: string;
  subject: string;
  snippet: string;
  last_date: number;
  message_count: number;
  unread_count: number;
  is_starred: boolean;
  participants: string[];
  has_attachments: boolean;
}

export async function listThreads(folderId: string, limit: number, offset: number): Promise<ThreadSummary[]> {
  return invoke<ThreadSummary[]>("list_threads", { folderId, limit, offset });
}

export async function listThreadMessages(threadId: string): Promise<Message[]> {
  return invoke<Message[]>("list_thread_messages", { threadId });
}

// ─── Labels API ─────────────────────────────────────────────────────────────

export interface Label {
  id: string;
  name: string;
  color: string;
  is_system: boolean;
  rule_id: string | null;
}

export async function getMessageLabels(messageId: string): Promise<Label[]> {
  return invoke<Label[]>("get_message_labels", { messageId });
}

export async function getMessageLabelsBatch(messageIds: string[]): Promise<Record<string, Label[]>> {
  return invoke<Record<string, Label[]>>("get_message_labels_batch", { messageIds });
}

export async function addMessageLabel(messageId: string, labelName: string): Promise<void> {
  return invoke<void>("add_message_label", { messageId, labelName });
}

export async function removeMessageLabel(messageId: string, labelName: string): Promise<void> {
  return invoke<void>("remove_message_label", { messageId, labelName });
}

export async function listLabels(): Promise<Label[]> {
  return invoke<Label[]>("list_labels");
}

// ─── Cloud Sync API ────────────────────────────────────────────────────────

export async function testWebdavConnection(url: string, username: string, password: string): Promise<string> {
  return invoke<string>("test_webdav_connection", { url, username, password });
}

export async function backupToWebdav(url: string, username: string, password: string): Promise<string> {
  return invoke<string>("backup_to_webdav", { url, username, password });
}

export async function restoreFromWebdav(url: string, username: string, password: string): Promise<string> {
  return invoke<string>("restore_from_webdav", { url, username, password });
}

// ─── Contacts API ──────────────────────────────────────────────────────────

export interface KnownContact {
  name: string | null;
  address: string;
}

export async function searchContacts(
  accountId: string,
  query: string,
  limit?: number,
): Promise<KnownContact[]> {
  return invoke<KnownContact[]>("search_contacts", { accountId, query, limit });
}
