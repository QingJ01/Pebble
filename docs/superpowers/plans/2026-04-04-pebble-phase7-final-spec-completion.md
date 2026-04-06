# Pebble Phase 7: Final Spec Completion

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close all remaining spec gaps and bring the project to 100% spec completion. This is the FINAL development phase covering: attachment system, shortcut customization, TanStack Query integration, system notifications, offline mode, contact autocomplete, WebDAV cloud sync, advanced search, and cross-feature polish.

**Architecture:** Builds on top of the existing 9 Rust crates, 28 Tauri commands, 4 Zustand stores, and React 19 frontend. New Tauri commands for attachments, contacts, cloud sync, and advanced search. TanStack Query replaces raw `invoke()` calls for data fetching. New Zustand store for shortcut customization.

**Tech Stack (additions):**
- `@tanstack/react-query` 5 (data fetching, caching, deduplication)
- `tauri-plugin-notification` (OS-level notifications)
- WebDAV via `reqwest` (cloud settings backup)

**Spec:** `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md` — all remaining sections

**Depends on:** Phase 6 complete (OAuth2, multi-provider, IDLE, CONDSTORE)

---

## File Structure (Phase 7 additions)

```
pebble/
├── crates/
│   ├── pebble-store/src/
│   │   ├── attachments.rs      # Attachment CRUD methods
│   │   └── cloud_sync.rs       # Settings export/import + WebDAV client
│   └── pebble-mail/src/
│       ├── parser.rs            # Extended: extract attachment bytes
│       └── sync.rs              # Modified: save attachments, emit notifications
├── src-tauri/src/
│   ├── commands/
│   │   ├── attachments.rs      # list_attachments, get_attachment_path, download_attachment
│   │   ├── contacts.rs         # search_contacts
│   │   ├── cloud_sync.rs       # test_webdav, backup_to_webdav, restore_from_webdav
│   │   └── advanced_search.rs  # advanced_search (structured query)
│   ├── lib.rs                  # + notification plugin, new commands
│   └── state.rs                # + attachments_dir
├── src/
│   ├── lib/
│   │   ├── query-client.ts     # TanStack Query client configuration
│   │   └── retry-queue.ts      # Offline retry queue
│   ├── hooks/
│   │   ├── queries/            # TanStack Query hooks (7 files)
│   │   ├── mutations/          # TanStack mutation hooks (3 files)
│   │   ├── useNetworkStatus.ts # Online/offline detection
│   │   └── useKeyboard.ts      # Modified: dynamic shortcut bindings
│   ├── stores/
│   │   ├── shortcut.store.ts   # Editable keybinding store
│   │   ├── mail.store.ts       # Slimmed: UI state only
│   │   └── ui.store.ts         # + networkStatus, search view
│   ├── features/
│   │   ├── search/
│   │   │   ├── SearchView.tsx
│   │   │   ├── SearchFilters.tsx
│   │   │   └── SearchResultItem.tsx
│   │   └── settings/
│   │       ├── CloudSyncTab.tsx
│   │       └── ShortcutsTab.tsx  # Rewritten: editable
│   └── components/
│       ├── AttachmentList.tsx
│       ├── ContactAutocomplete.tsx
│       └── StatusBar.tsx         # Enhanced: offline, retry, notifications
└── tests/
    ├── components/
    ├── hooks/
    ├── features/
    └── integration/
```

---

## Task 1: Attachment Backend — Extraction, Storage, and Tauri Commands

**Why:** The `attachments` table exists in SQLite, the `Attachment` type exists in `pebble-core`, and the parser extracts `AttachmentMeta`. However, binary data is discarded during sync, no store methods exist for the table, and no Tauri commands expose them.

### Steps

- [ ] **1.1** Add `insert_attachment`, `list_attachments_by_message`, `get_attachment` methods to `pebble-store` in new file `crates/pebble-store/src/attachments.rs`
- [ ] **1.2** In `pebble-mail/src/parser.rs`, extend `ParsedMessage` to carry `Vec<AttachmentData>` including raw `Vec<u8>` bytes from `part.contents()`
- [ ] **1.3** In `pebble-mail/src/sync.rs` `sync_folder()`, after `insert_message`, save attachment files to `{app_data_dir}/attachments/{message_id}/{filename}` and call `store.insert_attachment()`
- [ ] **1.4** Create `src-tauri/src/commands/attachments.rs` with commands: `list_attachments`, `get_attachment_path`, `download_attachment` (copies file to user-chosen destination)
- [ ] **1.5** Register commands in `mod.rs` and `lib.rs`
- [ ] **1.6** Add `attachments_dir: PathBuf` to `AppState`, initialize in `lib.rs` setup, thread through to sync
- [ ] **1.7** Rust unit tests: store CRUD, parser binary extraction

**Acceptance:** Syncing emails with attachments saves files + DB rows. `list_attachments` returns correct metadata. `download_attachment` copies file.

---

## Task 2: Attachment Frontend — UI Components, API Layer, i18n

**Why:** Even with backend support, users can't see or download attachments. `MessageDetail.tsx` has no attachment section.

### Steps

- [ ] **2.1** Add `Attachment` interface and `listAttachments`, `getAttachmentPath`, `downloadAttachment` to `src/lib/api.ts`
- [ ] **2.2** Create `src/components/AttachmentList.tsx` — file list with mime-type icons (lucide-react), size display, download button
- [ ] **2.3** Integrate into `MessageDetail.tsx` — render when `message.has_attachments` is true
- [ ] **2.4** Download via Tauri `dialog.save` → `downloadAttachment`
- [ ] **2.5** Show `Paperclip` icon on `MessageItem.tsx` and `ThreadItem.tsx` when `has_attachments`
- [ ] **2.6** i18n keys: `attachments.title`, `attachments.download`, `attachments.downloading`, `attachments.noAttachments`
- [ ] **2.7** Component test for `AttachmentList`

**Acceptance:** Attachment list visible in message detail. Download works via save dialog. Icon visible in list items.

---

## Task 3: Shortcut Customization — Editable Keybindings with Persistence

**Why:** `ShortcutsTab.tsx` is read-only. `useKeyboard.ts` has hardcoded key checks. Spec requires customizable shortcuts.

### Steps

- [ ] **3.1** Create `src/stores/shortcut.store.ts` — Zustand store with `{actionId: keyCombo}` map, localStorage persistence, `updateShortcut`, `resetToDefaults`, `detectConflicts`
- [ ] **3.2** Extract default shortcuts from current hardcoded values
- [ ] **3.3** Rewrite `src/hooks/useKeyboard.ts` to read bindings dynamically from `shortcut.store`
- [ ] **3.4** Rewrite `ShortcutsTab.tsx` — each row gets "record shortcut" button (capture next keydown), conflict warnings, "Reset to Defaults" button
- [ ] **3.5** i18n keys: `shortcuts.edit`, `shortcuts.recording`, `shortcuts.conflict`, `shortcuts.resetDefaults`
- [ ] **3.6** Store test: load defaults, update binding, conflict detection, reset, persistence

**Acceptance:** User can click to record new shortcut, conflicts warned, persists via localStorage, `useKeyboard` respects changes.

---

## Task 4: TanStack Query Integration

**Why:** All 28+ Tauri commands use raw `invoke()`. Spec calls for TanStack Query for caching, deduplication, background refresh.

### Steps

- [ ] **4.1** Add `@tanstack/react-query` to `package.json`
- [ ] **4.2** Create `src/lib/query-client.ts` with `QueryClient` config (staleTime: 30s, gcTime: 5min, retry: 2)
- [ ] **4.3** Wrap `<App />` in `<QueryClientProvider>` in `main.tsx`
- [ ] **4.4** Create query hooks in `src/hooks/queries/`: `useAccountsQuery`, `useFoldersQuery`, `useMessagesQuery`, `useThreadsQuery`, `useMessageQuery`, `useSearchQuery`, `useAttachmentsQuery`
- [ ] **4.5** Create mutation hooks in `src/hooks/mutations/`: `useSendEmailMutation`, `useUpdateFlagsMutation` (optimistic update), `useSyncMutation`
- [ ] **4.6** Slim `mail.store.ts` to UI-only state (selection, active IDs), remove data-fetching methods
- [ ] **4.7** Refactor consumers: `InboxView.tsx`, `MessageDetail.tsx`, `Sidebar.tsx`, `ComposeView.tsx` to use query hooks
- [ ] **4.8** Tests for query hooks

**Acceptance:** All data fetching via TanStack Query. Deduplication works. `mail.store.ts` only holds selection state.

---

## Task 5: System Notifications

**Why:** `snooze_watcher.rs` emits events but no OS notification. Spec requires "系统通知" for unsnoozed messages and new mail.

### Steps

- [ ] **5.1** Add `tauri-plugin-notification` to `src-tauri/Cargo.toml`
- [ ] **5.2** Register plugin in `lib.rs` builder
- [ ] **5.3** Add notification permission to `tauri.conf.json`
- [ ] **5.4** In `snooze_watcher.rs`, fire OS notification after `mail:unsnoozed` event
- [ ] **5.5** In sync worker, fire OS notification when new messages arrive
- [ ] **5.6** Add notification toggle to `AppearanceTab.tsx` with localStorage persistence
- [ ] **5.7** Listen for `mail:new` events in Layout and show badge/toast
- [ ] **5.8** i18n keys: `notifications.newMail`, `notifications.unsnoozed`, `settings.enableNotifications`

**Acceptance:** OS notification on snooze wake and new mail. Toggle in settings. Works on Windows 10.

---

## Task 6: Offline Mode and Error Handling

**Why:** No network status detection, no retry queue, no offline indicator. Spec 7.5: "网络错误显示离线状态 + 自动重试".

### Steps

- [ ] **6.1** Extend `ui.store.ts` with `networkStatus: 'online' | 'offline'`
- [ ] **6.2** Create `src/hooks/useNetworkStatus.ts` — browser events + periodic `health_check` ping
- [ ] **6.3** Update `StatusBar.tsx` — offline indicator with reconnect icon
- [ ] **6.4** Create `src/lib/retry-queue.ts` — exponential backoff (1s→30s), pause when offline, resume on reconnect
- [ ] **6.5** Integrate retry into mutation hooks / `api.ts` wrappers
- [ ] **6.6** Emit structured `mail:error` events from Rust sync worker
- [ ] **6.7** Listen for `mail:error` in frontend, surface in StatusBar
- [ ] **6.8** i18n keys: `status.offline`, `status.reconnecting`, `status.retrying`, `status.pendingActions`
- [ ] **6.9** Tests: retry-queue unit test, useNetworkStatus hook test

**Acceptance:** StatusBar shows "Offline". Failed ops retry with backoff. Pauses offline, resumes online.

---

## Task 7: Contact Autocomplete for Compose

**Why:** `ComposeView.tsx` uses plain `<input>` for To/Cc. Spec defines "收件人补全".

### Steps

- [ ] **7.1** Add `list_known_contacts(account_id, query, limit)` to `pebble-store/src/messages.rs` — DISTINCT addresses from messages table with LIKE prefix matching
- [ ] **7.2** Create `src-tauri/src/commands/contacts.rs` with `search_contacts` command
- [ ] **7.3** Register command, add `searchContacts` to `api.ts`
- [ ] **7.4** Create `src/components/ContactAutocomplete.tsx` — debounced input (200ms), dropdown, comma-separated multi-select, keyboard navigation, highlight matching
- [ ] **7.5** Replace plain `<input>` in `ComposeView.tsx` To/Cc with `ContactAutocomplete`
- [ ] **7.6** i18n keys: `compose.contactSuggestions`, `compose.noContactsFound`
- [ ] **7.7** Tests: store query test, component test

**Acceptance:** Typing in To/Cc shows autocomplete from known contacts. Keyboard navigable. Multi-select works.

---

## Task 8: Advanced Search Feature Module

**Why:** `SearchBar.tsx` is a simple text input. Spec defines search as full feature module with structured queries. `StructuredQuery` exists in `pebble-core` but is unused in frontend.

### Steps

- [ ] **8.1** Create `src-tauri/src/commands/advanced_search.rs` with `advanced_search(query: StructuredQuery, limit)` building Tantivy query from structured fields
- [ ] **8.2** Add `StructuredQuery` type and `advancedSearch()` to `api.ts`
- [ ] **8.3** Create `src/features/search/` module:
  - `SearchView.tsx` — full search page with results
  - `SearchFilters.tsx` — collapsible filter panel (from, to, subject, date range, has_attachment)
  - `SearchResultItem.tsx` — highlighted match with metadata
- [ ] **8.4** Upgrade `SearchBar.tsx` — add filter toggle icon, expand to advanced mode
- [ ] **8.5** Add `"search"` to `ActiveView` type in `ui.store.ts`, render `SearchView` in Layout
- [ ] **8.6** i18n keys: `search.title`, `search.filters`, `search.from`, `search.to`, `search.subject`, `search.dateFrom`, `search.dateTo`, `search.hasAttachment`, `search.noResults`
- [ ] **8.7** Tests: structured query parsing, SearchFilters component

**Acceptance:** Free-text and structured searches work. Filters for from/to/subject/date/attachment. Full search view.

---

## Task 9: WebDAV Cloud Sync for Settings Backup

**Why:** Spec section 7.4 defines cloud backup. No implementation exists.

### Steps

- [ ] **9.1** Create `crates/pebble-store/src/cloud_sync.rs` — WebDAV client (PUT, GET, PROPFIND via reqwest), `export_settings(store) -> Vec<u8>`, `import_settings(store, data)`
- [ ] **9.2** Create `src-tauri/src/commands/cloud_sync.rs` with commands: `test_webdav_connection`, `backup_to_webdav`, `restore_from_webdav`
- [ ] **9.3** Register commands, add API wrappers to `api.ts`
- [ ] **9.4** Create `src/features/settings/CloudSyncTab.tsx` — URL/username/password fields, test connection, backup/restore buttons, last backup timestamp
- [ ] **9.5** Register tab in `SettingsView.tsx`
- [ ] **9.6** i18n keys: `settings.cloudSync`, `cloudSync.title`, `cloudSync.webdavUrl`, `cloudSync.username`, `cloudSync.password`, `cloudSync.testConnection`, `cloudSync.backup`, `cloudSync.restore`, `cloudSync.lastBackup`
- [ ] **9.7** Rust test: export/import round-trip

**Acceptance:** WebDAV config in settings. Test connection validates. Backup/restore round-trip lossless.

---

## Task 10: Command Registration and Cross-Feature Polish

**Why:** New features need command palette integration, keyboard shortcuts, and StatusBar wiring.

### Steps

- [ ] **10.1** Update `src/features/command-palette/commands.ts` — add commands: "Open Search", "Download Attachment", "Backup to Cloud", "Restore from Cloud", "Toggle Notifications"
- [ ] **10.2** Add default shortcuts for new commands in `shortcut.store.ts`
- [ ] **10.3** Enhance `StatusBar.tsx` — network status, pending retry count, notification indicator
- [ ] **10.4** Verify all new Tauri commands registered (target: ~35 total)
- [ ] **10.5** Fill any missing i18n keys across both locale files
- [ ] **10.6** Test: command palette includes all expected commands

**Acceptance:** All features accessible from command palette. StatusBar comprehensive. No missing i18n keys.

---

## Task 11: Integration Testing and Final Verification

**Why:** 10 parallel tasks touching shared surfaces need integration verification.

### Steps

- [ ] **11.1** `cargo test --workspace` — all Rust tests pass (target: ≥160 tests)
- [ ] **11.2** `cargo clippy --workspace -- -D warnings` — no warnings
- [ ] **11.3** `npx tsc --noEmit` — TypeScript clean
- [ ] **11.4** `pnpm test` — all Vitest tests pass
- [ ] **11.5** Verify `invoke_handler` has all ~35 commands
- [ ] **11.6** Verify locale files have identical key structures (en.json ≡ zh.json)
- [ ] **11.7** Verify all 9 spec gaps are closed
- [ ] **11.8** Document any remaining polish items

**Acceptance:** All tests pass, no warnings, 100% spec coverage.

---

## Execution Order & Parallelism

```
Wave 1 (6 parallel):
  Task 1 (Attachment Backend) ──→ Task 2 (Attachment Frontend)
  Task 3 (Shortcuts)            
  Task 5 (Notifications)        
  Task 6 (Offline/Retry)        ──→ Task 10 (Polish) ──→ Task 11 (Integration)
  Task 7 (Contact Autocomplete) 
  Task 9 (WebDAV Cloud Sync)    

Wave 2 (3 parallel, after Wave 1):
  Task 2 (Attachment Frontend)  [depends on Task 1]
  Task 4 (TanStack Query)       [large refactor, safer after APIs stabilize]
  Task 8 (Search Module)        [independent]

Wave 3 (sequential):
  Task 10 (Cross-Feature Polish) [depends on all above]

Wave 4 (sequential):
  Task 11 (Integration Testing)  [depends on Task 10]
```

- **Wave 1:** Tasks 1, 3, 5, 6, 7, 9 — touch entirely different files, no conflicts
- **Wave 2:** Tasks 2, 4, 8 — Task 2 depends on Task 1; Tasks 4 and 8 independent but benefit from stable APIs
- **Wave 3:** Task 10 wires everything together
- **Wave 4:** Task 11 final verification
