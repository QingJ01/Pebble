/**
 * IPC type contracts for Rust structs that cross the Tauri invoke boundary.
 *
 * Most types are generated from the Rust definitions (ts-rs derives) into
 * `./generated/ipc.ts`; a cargo test fails when that file is stale
 * (regenerate with `UPDATE_IPC_BINDINGS=1 cargo test -p pebble ipc_bindings`).
 * This module re-exports them under the names the frontend already uses.
 */

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

export type {
  BatchMarkReadResult,
  ImapSyncFolderSettings,
  PendingMailOp,
  PendingMailOpsSummary,
  SearchHit,
} from "./generated/ipc";

import type { PendingMailOp } from "./generated/ipc";

export type PendingMailOpStatus = PendingMailOp["status"];
