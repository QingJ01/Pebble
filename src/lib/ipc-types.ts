/**
 * IPC type contracts for Rust structs that cross the Tauri invoke boundary.
 *
 * Most types are generated from the Rust definitions (ts-rs derives) into
 * `./generated/ipc.ts`; a cargo test fails when that file is stale
 * (regenerate with `UPDATE_IPC_BINDINGS=1 cargo test -p pebble ipc_bindings`).
 * This module re-exports them under the names the frontend already uses.
 *
 * The few hand-written mirrors left below live in Rust files that cannot
 * carry the derive yet; each has a `@rust` tag pointing at its source.
 */

import type { Folder } from "./generated/ipc";

export type {
  Account,
  AccountProxyMode,
  AccountProxySetting,
  AddAccountRequest,
  AdvancedSearchQuery,
  AppLogSnapshot,
  Attachment,
  BackupPreview,
  BilingualSegment,
  ConnectionSecurity,
  Contact,
  ContactEmail,
  ContactEmailInput,
  ContactEmailLabel,
  ContactInput,
  ContactSuggestion,
  ContactSuggestionSource,
  EmailAddress,
  Folder,
  FolderRole,
  FolderType,
  HttpProxyConfig,
  ImportedBackgroundImage,
  KanbanCard,
  KnownContact,
  Label,
  Message,
  MessageSummary,
  NotificationStatus,
  PrivacyMode,
  ProviderType,
  RenderedHtml,
  Rule,
  SnoozedMessage,
  ThreadSummary,
  TrackerInfo,
  TranslateConfig,
  TranslateResult,
  TrustedSender,
  TrustType,
  VcardImportResult,
} from "./generated/ipc";

/** @rust pebble-core/src/types.rs → KanbanColumn */
export type { KanbanColumn as KanbanColumnType } from "./generated/ipc";

// ─── Hand-written mirrors ────────────────────────────────────────────────────

/** @rust src-tauri/src/commands/folders.rs → ImapSyncFolderSettings */
export interface ImapSyncFolderSettings {
  folders: Folder[];
  selected_remote_ids: string[];
}

/** @rust pebble-store/src/pending_ops.rs → PendingMailOpsSummary */
export interface PendingMailOpsSummary {
  pending_count: number;
  in_progress_count: number;
  failed_count: number;
  total_active_count: number;
  last_error: string | null;
  updated_at: number | null;
}

/** @rust pebble-store/src/pending_ops.rs → PendingMailOpStatus */
export type PendingMailOpStatus = "pending" | "in_progress" | "failed";

/** @rust pebble-store/src/pending_ops.rs → PendingMailOp */
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

/** @rust pebble-core/src/traits.rs → SearchHit */
export interface SearchHit {
  message_id: string;
  score: number;
  snippet: string;
  subject?: string;
  from_address?: string;
  date?: number;
}
