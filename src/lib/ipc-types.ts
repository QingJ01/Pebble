/**
 * IPC type contracts — the single source of TypeScript mirrors for Rust structs
 * that cross the Tauri invoke boundary.
 *
 * Each type has a `@rust` JSDoc tag pointing to the canonical Rust definition.
 * When updating a type here, check the Rust source first; when updating the
 * Rust struct, update the mirror here.
 */

// ─── Core domain types ─────────────────────────────────────────────────────────

/** @rust pebble-core/src/types.rs → Account */
export interface Account {
  id: string;
  email: string;
  display_name: string;
  /** ProviderType enum (rename_all = "lowercase") */
  provider: "imap" | "gmail" | "outlook";
  created_at: number;
  updated_at: number;
}

/** @rust pebble-core/src/types.rs → Folder */
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

/** @rust pebble-core/src/types.rs → EmailAddress */
export interface EmailAddress {
  name: string | null;
  address: string;
}

/**
 * Lightweight message data for list views (no body fields).
 * @rust pebble-core/src/types.rs → MessageSummary
 */
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

/**
 * Full message including body content.
 * @rust pebble-core/src/types.rs → Message
 */
export interface Message extends MessageSummary {
  body_text: string;
  body_html_raw: string;
}

export interface PendingMailOpsSummary {
  pending_count: number;
  in_progress_count: number;
  failed_count: number;
  total_active_count: number;
  last_error: string | null;
  updated_at: number | null;
}

export type PendingMailOpStatus = "pending" | "in_progress" | "failed";

export interface PendingMailOp {
  id: string;
  account_id: string;
  message_id: string;
  op_type: string;
  status: PendingMailOpStatus;
  attempts: number;
  last_error: string | null;
  created_at: number;
  updated_at: number;
  next_retry_at: number | null;
}

/** @rust src-tauri/src/commands/diagnostics.rs -> AppLogSnapshot */
export interface AppLogSnapshot {
  path: string;
  content: string;
  truncated: boolean;
}

/** @rust pebble-core/src/types.rs → RenderedHtml */
export interface RenderedHtml {
  html: string;
  trackers_blocked: { domain: string; tracker_type: string }[];
  images_blocked: number;
}

/** @rust pebble-core/src/traits.rs → SearchHit */
export interface SearchHit {
  message_id: string;
  score: number;
  snippet: string;
  subject?: string;
  from_address?: string;
  date?: number;
}

/**
 * Serde external tagging: unit variants serialize as strings, tuple variants
 * as `{ VariantName: value }`.
 * @rust pebble-core/src/types.rs → PrivacyMode
 */
export type PrivacyMode = "Strict" | { TrustSender: string } | "LoadOnce" | "Off";

// ─── Mail config types ──────────────────────────────────────────────────────────

/** @rust pebble-mail/src/imap.rs → ConnectionSecurity (rename_all = "lowercase") */
export type ConnectionSecurity = "tls" | "starttls" | "plain";

/** @rust src-tauri/src/commands/accounts.rs → AddAccountRequest */
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

// ─── Attachment types ───────────────────────────────────────────────────────────

/** @rust pebble-core/src/types.rs → Attachment */
export interface Attachment {
  id: string;
  message_id: string;
  filename: string;
  mime_type: string;
  size: number;
  local_path: string | null;
  content_id: string | null;
  is_inline: boolean;
}

// ─── Kanban types ───────────────────────────────────────────────────────────────

/** @rust pebble-core/src/types.rs → KanbanColumn (rename_all = "lowercase") */
export type KanbanColumnType = "todo" | "waiting" | "done";

/** @rust pebble-core/src/types.rs → KanbanCard */
export interface KanbanCard {
  message_id: string;
  column: KanbanColumnType;
  position: number;
  created_at: number;
  updated_at: number;
}

// ─── Snooze types ───────────────────────────────────────────────────────────────

/** @rust pebble-core/src/types.rs → SnoozedMessage */
export interface SnoozedMessage {
  message_id: string;
  snoozed_at: number;
  unsnoozed_at: number;
  return_to: string;
}

// ─── Trusted Sender types ───────────────────────────────────────────────────────

/** @rust pebble-core/src/types.rs → TrustedSender */
export interface TrustedSender {
  account_id: string;
  email: string;
  /** TrustType enum (rename_all = "lowercase") */
  trust_type: "images" | "all";
  created_at: number;
}

// ─── Rule types ─────────────────────────────────────────────────────────────────

/** @rust pebble-core/src/types.rs → Rule */
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

// ─── Search types ───────────────────────────────────────────────────────────────

/**
 * @rust src-tauri/src/commands/advanced_search.rs → AdvancedSearchQuery
 * (rename_all = "camelCase")
 */
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

// ─── Translate types ────────────────────────────────────────────────────────────

/** @rust pebble-core/src/types.rs → TranslateConfig */
export interface TranslateConfig {
  id: string;
  provider_type: string;
  config: string;
  is_enabled: boolean;
  created_at: number;
  updated_at: number;
}

/** @rust pebble-translate/src/types.rs → TranslateResult + BilingualSegment */
export interface TranslateResult {
  translated: string;
  segments: { source: string; target: string }[];
}

// ─── Thread types ───────────────────────────────────────────────────────────────

/** @rust pebble-core/src/types.rs → ThreadSummary */
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

// ─── Label types ────────────────────────────────────────────────────────────────

/** @rust pebble-store/src/labels.rs → Label */
export interface Label {
  id: string;
  name: string;
  color: string;
  is_system: boolean;
  rule_id: string | null;
}

// ─── Cloud Sync types ───────────────────────────────────────────────────────────

/** @rust pebble-store/src/cloud_sync.rs → BackupPreview */
export interface BackupPreview {
  version: number;
  exported_at: number;
  account_count: number;
  rule_count: number;
  kanban_card_count: number;
  kanban_note_count: number;
  has_translate_config: boolean;
  size_bytes: number;
}

// ─── Contacts types ─────────────────────────────────────────────────────────────

/** @rust pebble-core/src/types.rs → KnownContact */
export interface KnownContact {
  name: string | null;
  address: string;
}
