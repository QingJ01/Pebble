import { invoke } from "@tauri-apps/api/core";

// Re-export all IPC types so existing `import { Foo } from "@/lib/api"` keeps working.
export type {
  Account,
  AccountProxyMode,
  AccountProxySetting,
  AddAccountRequest,
  AdvancedSearchQuery,
  AppLogSnapshot,
  Attachment,
  BackupPreview,
  ConnectionSecurity,
  EmailAddress,
  Folder,
  HttpProxyConfig,
  KanbanCard,
  KanbanColumnType,
  KnownContact,
  Label,
  Message,
  MessageSummary,
  NotificationStatus,
  PendingMailOp,
  PendingMailOpsSummary,
  PrivacyMode,
  RenderedHtml,
  Rule,
  SearchHit,
  SnoozedMessage,
  ThreadSummary,
  TranslateConfig,
  TranslateResult,
  TrustedSender,
} from "./ipc-types";

import type {
  Account,
  AccountProxyMode,
  AccountProxySetting,
  AddAccountRequest,
  AdvancedSearchQuery,
  AppLogSnapshot,
  Attachment,
  BackupPreview,
  ConnectionSecurity,
  Folder,
  HttpProxyConfig,
  KanbanCard,
  KanbanColumnType,
  KnownContact,
  Label,
  Message,
  MessageSummary,
  NotificationStatus,
  PendingMailOp,
  PendingMailOpsSummary,
  PrivacyMode,
  RenderedHtml,
  Rule,
  SearchHit,
  SnoozedMessage,
  ThreadSummary,
  TranslateConfig,
  TranslateResult,
  TrustedSender,
} from "./ipc-types";

// ─── Account API ─────────────────────────────────────────────────────────────

export async function healthCheck(): Promise<string> {
  return invoke<string>("health_check");
}

export async function readAppLog(maxBytes: number): Promise<AppLogSnapshot> {
  return invoke<AppLogSnapshot>("read_app_log", { maxBytes });
}

export async function getGlobalProxy(): Promise<HttpProxyConfig | null> {
  return invoke<HttpProxyConfig | null>("get_global_proxy");
}

export async function getAccountProxy(accountId: string): Promise<HttpProxyConfig | null> {
  return invoke<HttpProxyConfig | null>("get_account_proxy", { accountId });
}

export async function getAccountProxySetting(accountId: string): Promise<AccountProxySetting> {
  return invoke<AccountProxySetting>("get_account_proxy_setting", { accountId });
}

export async function updateAccountProxy(
  accountId: string,
  proxyHost?: string,
  proxyPort?: number,
): Promise<void> {
  return invoke<void>("update_account_proxy", { accountId, proxyHost, proxyPort });
}

export async function updateAccountProxySetting(
  accountId: string,
  mode: AccountProxyMode,
  proxyHost?: string,
  proxyPort?: number,
): Promise<void> {
  return invoke<void>("update_account_proxy_setting", { accountId, mode, proxyHost, proxyPort });
}

export async function updateGlobalProxy(
  proxyHost?: string,
  proxyPort?: number,
): Promise<void> {
  return invoke<void>("update_global_proxy", { proxyHost, proxyPort });
}

export async function completeOAuthFlow(
  provider: string,
  email: string,
  displayName: string,
  proxyHost?: string,
  proxyPort?: number,
): Promise<Account> {
  return invoke<Account>("complete_oauth_flow", { provider, email, displayName, proxyHost, proxyPort });
}

export async function getOAuthAccountProxy(accountId: string): Promise<HttpProxyConfig | null> {
  return invoke<HttpProxyConfig | null>("get_oauth_account_proxy", { accountId });
}

export async function getOAuthAccountProxySetting(accountId: string): Promise<AccountProxySetting> {
  return invoke<AccountProxySetting>("get_oauth_account_proxy_setting", { accountId });
}

export async function updateOAuthAccountProxy(
  accountId: string,
  proxyHost?: string,
  proxyPort?: number,
): Promise<void> {
  return invoke<void>("update_oauth_account_proxy", { accountId, proxyHost, proxyPort });
}

export async function updateOAuthAccountProxySetting(
  accountId: string,
  mode: AccountProxyMode,
  proxyHost?: string,
  proxyPort?: number,
): Promise<void> {
  return invoke<void>("update_oauth_account_proxy_setting", { accountId, mode, proxyHost, proxyPort });
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
  accountColor?: string,
): Promise<void> {
  return invoke<void>("update_account", {
    accountId, email, displayName, password,
    imapHost, imapPort, smtpHost, smtpPort, imapSecurity, smtpSecurity,
    proxyHost, proxyPort, accountColor,
  });
}

export async function deleteAccount(accountId: string): Promise<void> {
  return invoke<void>("delete_account", { accountId });
}

// ─── Folder API ──────────────────────────────────────────────────────────────

export async function listFolders(accountId: string): Promise<Folder[]> {
  return invoke<Folder[]>("list_folders", { accountId });
}

// ─── Message API ─────────────────────────────────────────────────────────────

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

// Rapid-toggle guard: archive_message is toggle-based (archive ⇄ unarchive),
// so a double-click would flip the state back. This Set coalesces concurrent
// calls per-message; it is NOT idempotency — a second click *after* the first
// resolves is intentionally allowed to unarchive.
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

export async function moveToFolder(messageId: string, targetFolderId: string): Promise<void> {
  return invoke<void>("move_to_folder", { messageId, targetFolderId });
}

export async function emptyTrash(accountId: string): Promise<number> {
  return invoke<number>("empty_trash", { accountId });
}

export async function getPendingMailOpsSummary(
  accountId: string | null,
): Promise<PendingMailOpsSummary> {
  return invoke<PendingMailOpsSummary>("get_pending_mail_ops_summary", { accountId });
}

export async function listPendingMailOps(
  accountId: string | null,
  limit = 100,
): Promise<PendingMailOp[]> {
  return invoke<PendingMailOp[]>("list_pending_mail_ops", { accountId, limit });
}

// ─── Trusted Senders API ────────────────────────────────────────────────────

export async function listTrustedSenders(accountId: string): Promise<TrustedSender[]> {
  return invoke<TrustedSender[]>("list_trusted_senders", { accountId });
}

export async function removeTrustedSender(accountId: string, email: string): Promise<void> {
  return invoke<void>("remove_trusted_sender", { accountId, email });
}

export async function trustSender(accountId: string, email: string, trustType: "images" | "all"): Promise<void> {
  return invoke<void>("trust_sender", { accountId, email, trustType });
}

export async function isTrustedSender(accountId: string, email: string): Promise<boolean> {
  return invoke<boolean>("is_trusted_sender", { accountId, email });
}

// ─── Search API ──────────────────────────────────────────────────────────────

export async function searchMessages(
  query: string,
  limit?: number,
): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search_messages", { query, limit });
}

export async function advancedSearch(
  query: AdvancedSearchQuery,
  limit?: number,
): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("advanced_search", { query, limit });
}

// ─── Sync API ────────────────────────────────────────────────────────────────

export async function startSync(accountId: string, pollIntervalSecs?: number): Promise<string> {
  return invoke<string>("start_sync", { accountId, pollIntervalSecs: pollIntervalSecs ?? null });
}

export async function triggerSync(accountId: string, reason: string): Promise<void> {
  return invoke<void>("trigger_sync", { accountId, reason });
}

export type RealtimePreference = "realtime" | "balanced" | "battery" | "manual";

export async function setRealtimePreference(mode: RealtimePreference): Promise<void> {
  return invoke<void>("set_realtime_preference", { mode });
}

export async function setNotificationsEnabled(enabled: boolean): Promise<void> {
  return invoke<void>("set_notifications_enabled", { enabled });
}

export async function getNotificationStatus(): Promise<NotificationStatus> {
  return invoke<NotificationStatus>("get_notification_status");
}

export async function showTestNotification(): Promise<void> {
  return invoke<void>("show_test_notification");
}

export async function clearNotificationAttention(): Promise<void> {
  return invoke<void>("clear_notification_attention");
}

export async function setTrayMenuLabels(showLabel: string, hideLabel: string, quitLabel: string): Promise<void> {
  return invoke<void>("set_tray_menu_labels", { showLabel, hideLabel, quitLabel });
}

export async function stopSync(accountId: string): Promise<void> {
  return invoke<void>("stop_sync", { accountId });
}

// ─── Attachment API ──────────────────────────────────────────────────────────

export async function listAttachments(messageId: string): Promise<Attachment[]> {
  return invoke<Attachment[]>("list_attachments", { messageId });
}

export async function getAttachmentPath(attachmentId: string): Promise<string | null> {
  return invoke<string | null>("get_attachment_path", { attachmentId });
}

export async function downloadAttachment(attachmentId: string, saveTo: string): Promise<string> {
  return invoke<string>("download_attachment", { attachmentId, saveTo });
}

// ─── Kanban API ──────────────────────────────────────────────────────────────

export async function moveToKanban(messageId: string, column: KanbanColumnType, position?: number): Promise<void> {
  return invoke<void>("move_to_kanban", { messageId, column, position });
}

export async function listKanbanCards(column?: KanbanColumnType): Promise<KanbanCard[]> {
  return invoke<KanbanCard[]>("list_kanban_cards", { column });
}

export async function removeFromKanban(messageId: string): Promise<void> {
  return invoke<void>("remove_from_kanban", { messageId });
}

export async function listKanbanContextNotes(): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("list_kanban_context_notes");
}

export async function setKanbanContextNote(
  messageId: string,
  note: string,
): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("set_kanban_context_note", { messageId, note });
}

export async function mergeKanbanContextNotes(
  notes: Record<string, string>,
): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("merge_kanban_context_notes", { notes });
}

// ─── Snooze API ──────────────────────────────────────────────────────────────

export async function snoozeMessage(messageId: string, until: number, returnTo: string): Promise<void> {
  return invoke<void>("snooze_message", { messageId, until, returnTo });
}

export async function unsnoozeMessage(messageId: string): Promise<void> {
  return invoke<void>("unsnooze_message", { messageId });
}

export async function listSnoozed(): Promise<SnoozedMessage[]> {
  return invoke<SnoozedMessage[]>("list_snoozed");
}

// ─── Rules API ───────────────────────────────────────────────────────────────

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

// ─── Compose API ─────────────────────────────────────────────────────────────

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

export async function stageComposeAttachment(filename: string, bytes: number[]): Promise<string> {
  return invoke<string>("stage_compose_attachment", { filename, bytes });
}

// ─── Batch Operations ───────────────────────────────────────────────────────

export async function batchArchive(messageIds: string[]): Promise<number> {
  return invoke<number>("batch_archive", { messageIds });
}

export async function batchDelete(messageIds: string[]): Promise<number> {
  return invoke<number>("batch_delete", { messageIds });
}

export async function batchMarkRead(messageIds: string[], isRead: boolean): Promise<number> {
  return invoke<number>("batch_mark_read", { messageIds, isRead });
}

export async function batchStar(messageIds: string[], starred: boolean): Promise<number> {
  return invoke<number>("batch_star", { messageIds, starred });
}

// ─── Translate API ───────────────────────────────────────────────────────────

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

// ─── Thread API ──────────────────────────────────────────────────────────────

export async function listThreads(
  folderId: string,
  limit: number,
  offset: number,
  folderIds?: string[],
): Promise<ThreadSummary[]> {
  return invoke<ThreadSummary[]>("list_threads", { folderId, folderIds, limit, offset });
}

export async function listThreadMessages(threadId: string): Promise<Message[]> {
  return invoke<Message[]>("list_thread_messages", { threadId });
}

// ─── Labels API ──────────────────────────────────────────────────────────────

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

// ─── Cloud Sync API ─────────────────────────────────────────────────────────

export async function testWebdavConnection(url: string, username: string, password: string): Promise<string> {
  return invoke<string>("test_webdav_connection", { url, username, password });
}

export async function backupToWebdav(url: string, username: string, password: string): Promise<string> {
  return invoke<string>("backup_to_webdav", { url, username, password });
}

export async function previewWebdavBackup(url: string, username: string, password: string): Promise<BackupPreview> {
  return invoke<BackupPreview>("preview_webdav_backup", { url, username, password });
}

export async function restoreFromWebdav(url: string, username: string, password: string): Promise<string> {
  return invoke<string>("restore_from_webdav", { url, username, password });
}

// ─── Contacts API ────────────────────────────────────────────────────────────

export async function searchContacts(
  accountId: string,
  query: string,
  limit?: number,
): Promise<KnownContact[]> {
  return invoke<KnownContact[]>("search_contacts", { accountId, query, limit });
}

// ─── Drafts API ──────────────────────────────────────────────────────────────

export async function saveDraft(args: {
  accountId: string;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  bodyText: string;
  bodyHtml?: string;
  inReplyTo?: string;
  existingDraftId?: string;
  attachmentPaths?: string[];
}): Promise<string> {
  return invoke("save_draft", {
    accountId: args.accountId,
    to: args.to,
    cc: args.cc,
    bcc: args.bcc,
    subject: args.subject,
    bodyText: args.bodyText,
    bodyHtml: args.bodyHtml ?? null,
    inReplyTo: args.inReplyTo ?? null,
    existingDraftId: args.existingDraftId ?? null,
    attachmentPaths: args.attachmentPaths ?? null,
  });
}

export async function deleteDraft(accountId: string, draftId: string): Promise<void> {
  return invoke("delete_draft", { accountId, draftId });
}

// ─── Folder Counts API ───────────────────────────────────────────────────────

export async function getFolderUnreadCounts(accountId: string): Promise<Record<string, number>> {
  return invoke("get_folder_unread_counts", { accountId });
}
