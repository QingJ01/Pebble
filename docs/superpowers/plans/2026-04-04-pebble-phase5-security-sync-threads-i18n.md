# Pebble Phase 5: Security Hardening + Sync Engine + Thread View + i18n

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden auth data security (AES-256-GCM encryption + OS credential store), make the IMAP sync engine robust (IDLE push, flag diff reconciliation, ChangeSet pipeline, tombstone cleanup), add a thread/conversation view to the frontend, and implement i18n with Chinese/English dual-language support via i18next.

**Architecture:** New Rust crate `pebble-crypto` for encryption primitives. Auth credentials encrypted at rest in the `accounts.auth_data` BLOB column, with the DEK stored in OS credential store via the `keyring` crate. SyncWorker upgraded to use flag diff reconciliation and IMAP IDLE for push notifications. New Tauri commands for thread queries plus frontend `ThreadView.tsx`. i18next with JSON translation files under `src/locales/`.

**Tech Stack:**
- `aes-gcm` 0.10 (AES-256-GCM encryption)
- `keyring` 3 (OS credential store — Windows Credential Manager / macOS Keychain / Linux Secret Service)
- `rand` 0.8 (nonce generation)
- `i18next` + `react-i18next` (frontend i18n)

**Spec:** `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md` § 4.3, § 6.1, § 7.2

**Depends on:** Phase 4 complete (translate, settings, dark theme)

**Note on Gmail/Outlook providers:** OAuth2 provider integration (Gmail API, Microsoft Graph) remains deferred to Phase 6. Phase 5 encryption infrastructure is a prerequisite — OAuth2 tokens will be stored through the same encrypted auth_data path.

---

## File Structure (Phase 5 additions)

```
pebble/
├── crates/
│   └── pebble-crypto/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs              # CryptoService: encrypt/decrypt auth data
│           ├── aes.rs              # AES-256-GCM encrypt/decrypt primitives
│           └── keystore.rs         # DEK management via OS credential store (keyring)
│
├── crates/pebble-store/src/
│   └── auth_data.rs                # Store/load/delete encrypted auth_data BLOB
│
├── crates/pebble-mail/src/
│   ├── idle.rs                     # IMAP IDLE listener (push notifications)
│   └── reconcile.rs                # Flag diff + delete/move detection logic
│
├── src-tauri/src/
│   ├── commands/
│   │   └── threads.rs              # list_thread_messages, list_threads commands
│   └── lib.rs                      # Register new commands, init CryptoService
│
├── src/
│   ├── locales/
│   │   ├── en.json                 # English translations
│   │   └── zh.json                 # Chinese translations
│   ├── lib/
│   │   └── i18n.ts                 # i18next initialization
│   ├── features/
│   │   └── inbox/
│   │       └── ThreadView.tsx      # Conversation/thread view
│   ├── components/
│   │   ├── ThreadItem.tsx          # Thread row in message list
│   │   └── ThreadMessageBubble.tsx # Single message within thread view
│   └── features/settings/
│       └── AppearanceTab.tsx       # Add language selector
```

---

## Dependency Graph

```
Task 1 (crypto crate) ──> Task 2 (store auth_data) ──> Task 3 (wire encryption)
Task 4 (flag diff / reconcile) ──> Task 5 (IDLE push)
Task 6 (tombstone cleanup) -- independent
Task 7 (thread backend) ──> Task 8 (thread frontend)
Task 9 (i18n) -- independent
Task 10 (cursor persistence + integration test) -- depends on Tasks 3, 4, 5
```

**Parallelization:**
- Agent A: Tasks 1 → 2 → 3
- Agent B: Tasks 4 → 5
- Agent C: Task 6 + Task 9
- Agent D: Task 7 → 8
- Final: Task 10 (after A, B complete)

---

### Task 1: New crate `pebble-crypto` — AES-256-GCM encryption + OS keystore

**Files:**
- Create: `crates/pebble-crypto/Cargo.toml`
- Create: `crates/pebble-crypto/src/lib.rs`
- Create: `crates/pebble-crypto/src/aes.rs`
- Create: `crates/pebble-crypto/src/keystore.rs`
- Modify: `Cargo.toml` (workspace members + workspace.dependencies)

**Context:** Auth credentials (IMAP password, future OAuth2 tokens) must be encrypted at rest. The design uses a Data Encryption Key (DEK) stored in the OS credential store (Windows Credential Manager on this platform). The DEK encrypts/decrypts the `auth_data` BLOB via AES-256-GCM. This is a prerequisite for Phase 6 OAuth2 since tokens will be stored through this same path.

- [ ] **Step 1: Add workspace dependencies**

In root `Cargo.toml` under `[workspace.dependencies]`, add:
```toml
aes-gcm = "0.10"
rand = "0.8"
keyring = { version = "3", features = ["sync-secret-service", "windows-native", "apple-native"] }
```

Add `"crates/pebble-crypto"` to workspace members.

- [ ] **Step 2: Create `crates/pebble-crypto/Cargo.toml`**

```toml
[package]
name = "pebble-crypto"
version = "0.1.0"
edition = "2021"

[dependencies]
pebble-core = { path = "../pebble-core" }
aes-gcm = { workspace = true }
rand = { workspace = true }
keyring = { workspace = true }
tracing = { workspace = true }
```

- [ ] **Step 3: Create `crates/pebble-crypto/src/aes.rs`**

Two public functions:
- `encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>>` — generates random 12-byte nonce, encrypts with AES-256-GCM, returns `nonce || ciphertext || tag`.
- `decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>>` — splits first 12 bytes as nonce, decrypts the rest.

Use `aes_gcm::{Aes256Gcm, KeyInit, aead::Aead}` and `rand::RngCore`.

- [ ] **Step 4: Create `crates/pebble-crypto/src/keystore.rs`**

`KeyStore` struct:
- `const SERVICE_NAME: &str = "com.pebble.email";`
- `const KEY_ENTRY: &str = "master-dek";`
- `pub fn get_or_create_dek() -> Result<[u8; 32]>` — uses `keyring::Entry` to get/create DEK.
- `pub fn delete_dek() -> Result<()>` — deletes the entry.

- [ ] **Step 5: Create `crates/pebble-crypto/src/lib.rs`**

```rust
pub mod aes;
pub mod keystore;

pub struct CryptoService {
    dek: [u8; 32],
}

impl CryptoService {
    pub fn init() -> pebble_core::Result<Self> {
        let dek = keystore::KeyStore::get_or_create_dek()?;
        Ok(Self { dek })
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> pebble_core::Result<Vec<u8>> {
        aes::encrypt(&self.dek, plaintext)
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> pebble_core::Result<Vec<u8>> {
        aes::decrypt(&self.dek, ciphertext)
    }
}
```

Tests: encrypt/decrypt round-trip, wrong key fails, truncated data fails, `CryptoService::init()` succeeds (mark `#[ignore]` for CI).

Commit: `feat(crypto): add pebble-crypto crate with AES-256-GCM encryption and OS keystore`

---

### Task 2: Store — encrypted auth_data read/write

**Files:**
- Create: `crates/pebble-store/src/auth_data.rs`
- Modify: `crates/pebble-store/src/lib.rs` (add module)

**Context:** The `accounts` table already has an `auth_data BLOB` column but it's never read or written. This task adds store methods to persist and retrieve the encrypted BLOB. The encryption itself happens in the command layer using `CryptoService` — the store only handles raw bytes.

- [ ] **Step 1: Create `crates/pebble-store/src/auth_data.rs`**

Methods on `impl Store`:
- `pub fn set_auth_data(&self, account_id: &str, encrypted: &[u8]) -> Result<()>` — `UPDATE accounts SET auth_data = ?1, updated_at = ?2 WHERE id = ?3`
- `pub fn get_auth_data(&self, account_id: &str) -> Result<Option<Vec<u8>>>` — `SELECT auth_data FROM accounts WHERE id = ?1`
- `pub fn clear_auth_data(&self, account_id: &str) -> Result<()>` — `UPDATE accounts SET auth_data = NULL WHERE id = ?1`

- [ ] **Step 2: Register module**

Add `pub mod auth_data;` to `crates/pebble-store/src/lib.rs`.

Tests: set/get round-trip, get returns None for no data, clear sets back to None.

Commit: `feat(store): add auth_data BLOB read/write for encrypted credential storage`

---

### Task 3: Wire encryption into account add/sync flow

**Files:**
- Modify: `src-tauri/src/commands/accounts.rs` (encrypt credentials on add)
- Modify: `src-tauri/src/commands/sync_cmd.rs` (decrypt credentials on sync start)
- Modify: `src-tauri/src/state.rs` (add CryptoService to AppState)
- Modify: `src-tauri/src/lib.rs` (init CryptoService at startup)
- Modify: `src-tauri/Cargo.toml` (add pebble-crypto dependency)

**Context:** Currently `add_account` stores IMAP/SMTP config as plaintext JSON in `sync_state`. This task migrates to encrypted `auth_data` BLOB. Flow: `add_account` → serialize credentials → `CryptoService.encrypt()` → `store.set_auth_data()`. `start_sync` → `store.get_auth_data()` → `CryptoService.decrypt()` → deserialize → create `ImapProvider`.

- [ ] **Step 1: Add CryptoService to AppState**

In `state.rs`, add `pub crypto: Arc<CryptoService>`. In `lib.rs` setup, init `CryptoService::init()` before `app.manage()`.

- [ ] **Step 2: Modify `add_account` command**

After creating account: serialize IMAP/SMTP config to JSON bytes, encrypt via `state.crypto.encrypt()`, store via `state.store.set_auth_data()`. Write only non-secret metadata to `sync_state`.

- [ ] **Step 3: Modify `start_sync` command**

Read `state.store.get_auth_data()`, decrypt via `state.crypto.decrypt()`, parse config. Keep fallback: if `auth_data` is None, try legacy `sync_state` for backward compatibility.

- [ ] **Step 4: Update Cargo.toml**

Add `pebble-crypto = { path = "../crates/pebble-crypto" }` to `src-tauri/Cargo.toml`.

Commit: `feat(security): encrypt auth credentials at rest with AES-256-GCM via OS keystore`

---

### Task 4: Sync engine — ChangeSet pipeline + flag diff reconciliation

**Files:**
- Create: `crates/pebble-mail/src/reconcile.rs`
- Modify: `crates/pebble-mail/src/imap.rs` (add `fetch_flags`, `fetch_all_uids`)
- Modify: `crates/pebble-mail/src/sync.rs` (add reconcile_folder)
- Modify: `crates/pebble-mail/src/lib.rs` (add module)
- Modify: `crates/pebble-store/src/messages.rs` (add helper queries)

**Context:** Currently `poll_new_messages()` only fetches new UIDs. It never detects flag changes or server-side deletions. This task wires up flag diff reconciliation: fetch flags for known UIDs, compare to local, apply changes. Also detects deletions by comparing local vs server UID sets.

- [ ] **Step 1: Add store helper queries**

In `messages.rs`:
- `list_remote_ids_by_folder(account_id, folder_id) -> Vec<(String, String, bool, bool)>` — `(message_id, remote_id, is_read, is_starred)` for non-deleted messages.
- `bulk_update_flags(changes: &[(String, Option<bool>, Option<bool>)]) -> Result<()>` — batch update flags in a transaction.
- `bulk_soft_delete(message_ids: &[String]) -> Result<()>` — batch soft-delete.

- [ ] **Step 2: Add `fetch_flags` and `fetch_all_uids` to ImapProvider**

- `fetch_flags(mailbox, uids) -> Vec<(u32, bool, bool)>` — FETCH FLAGS for given UIDs.
- `fetch_all_uids(mailbox) -> Vec<u32>` — `UID SEARCH ALL`.

- [ ] **Step 3: Create `reconcile.rs`**

```rust
pub fn compute_flag_diff(
    local: &[(String, String, bool, bool)],
    remote: &[(u32, bool, bool)],
) -> Vec<(String, Option<bool>, Option<bool>)>

pub fn detect_deletions(
    local_remote_ids: &[(String, String)],
    server_uids: &[u32],
) -> Vec<String>
```

- [ ] **Step 4: Wire into SyncWorker**

Add `reconcile_folder()` method:
1. Get local state → fetch remote flags → compute diff → apply.
2. Fetch all server UIDs → detect deletions → soft-delete locally.
Replace `reconcile_ticker` to use `reconcile_folder` instead of `initial_sync`.

- [ ] **Step 5: Register module**

Tests: flag diff produces changes, matching flags produce nothing, deletions detected, all UIDs present gives empty result.

Commit: `feat(sync): wire flag diff reconciliation and server-side delete detection`

---

### Task 5: Sync engine — IMAP IDLE push notifications

**Files:**
- Create: `crates/pebble-mail/src/idle.rs`
- Modify: `crates/pebble-mail/src/imap.rs` (add `supports_idle`, IDLE handling)
- Modify: `crates/pebble-mail/src/sync.rs` (integrate IDLE into run loop)
- Modify: `crates/pebble-mail/src/lib.rs` (add module)

**Context:** IDLE allows the IMAP server to push notifications when new mail arrives, reducing latency from 60s to near-instant. RFC 2177 specifies a 29-minute timeout. Falls back to polling if server doesn't support IDLE.

- [ ] **Step 1: Add `supports_idle` to ImapProvider**

Issue `CAPABILITY`, check for `IDLE` in response. Cache in a field.

- [ ] **Step 2: Create `idle.rs`**

```rust
pub enum IdleEvent { NewMail, FlagsChanged, Expunge, Timeout, Error(String) }

pub async fn wait_for_changes(
    provider: &ImapProvider,
    mailbox: &str,
    timeout: Duration,
) -> Result<IdleEvent>
```

Uses `async_imap`'s `idle()` with `tokio::time::timeout(29 min)`.

- [ ] **Step 3: Modify SyncWorker run loop**

After initial sync, check `supports_idle()`:
- If IDLE supported: replace poll_ticker with IDLE-based loop (NewMail → poll, FlagsChanged/Expunge ��� reconcile, Timeout → re-issue).
- Keep 15min reconcile_ticker running independently.
- If IDLE not supported: keep existing polling unchanged.

Tests: IdleEvent enum variants test, integration test (mark `#[ignore]`).

Commit: `feat(sync): add IMAP IDLE push notification support with polling fallback`

---

### Task 6: Tombstone cleanup — 30-day physical deletion

**Files:**
- Modify: `crates/pebble-store/src/messages.rs` (add cleanup query)
- Modify: `src-tauri/src/snooze_watcher.rs` (add hourly tombstone cleanup)

**Context:** Soft-deleted messages should be physically removed after 30 days. Piggyback on existing snooze_watcher's 30s loop, running cleanup once per hour.

- [ ] **Step 1: Add cleanup query**

```rust
pub fn purge_old_tombstones(&self, older_than_secs: i64) -> Result<u32>
```
`DELETE FROM messages WHERE is_deleted = 1 AND deleted_at < ?1`. Returns row count.

- [ ] **Step 2: Add periodic cleanup to snooze_watcher**

Track `last_purge: Instant`. Every iteration, check if 1 hour elapsed. If yes, call `purge_old_tombstones(30 * 86400)`. Log count at info level.

Tests: insert + soft-delete 31 days ago → purged. 1 day ago → not purged. Non-deleted → never purged.

Commit: `feat(store): add 30-day tombstone physical cleanup for soft-deleted messages`

---

### Task 7: Thread/conversation view — backend commands

**Files:**
- Create: `src-tauri/src/commands/threads.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `crates/pebble-store/src/messages.rs` (add thread queries)
- Modify: `crates/pebble-core/src/types.rs` (add ThreadSummary)

**Context:** `thread.rs` already computes `thread_id` during sync. Need a command to return all messages in a thread and a query to list threads grouped by folder.

- [ ] **Step 1: Add ThreadSummary to pebble-core types**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub thread_id: String,
    pub subject: String,
    pub snippet: String,
    pub last_date: i64,
    pub message_count: u32,
    pub unread_count: u32,
    pub is_starred: bool,
    pub participants: Vec<String>,
    pub has_attachments: bool,
}
```

- [ ] **Step 2: Add thread queries to store**

- `list_messages_by_thread(thread_id) -> Vec<Message>` — `WHERE thread_id = ?1 AND is_deleted = 0 ORDER BY date ASC`
- `list_threads_by_folder(folder_id, limit, offset) -> Vec<ThreadSummary>` — GROUP BY thread_id, aggregate counts, participants, starred.

- [ ] **Step 3: Create `commands/threads.rs`**

```rust
#[tauri::command]
pub async fn list_thread_messages(state, thread_id) -> Result<Vec<Message>>

#[tauri::command]
pub async fn list_threads(state, folder_id, limit, offset) -> Result<Vec<ThreadSummary>>
```

- [ ] **Step 4: Register commands**

Tests: list_messages_by_thread chronological order, list_threads_by_folder correct grouping and counts.

Commit: `feat(threads): add thread query commands for conversation view`

---

### Task 8: Thread/conversation view — frontend UI

**Files:**
- Create: `src/components/ThreadItem.tsx`
- Create: `src/components/ThreadMessageBubble.tsx`
- Create: `src/features/inbox/ThreadView.tsx`
- Modify: `src/lib/api.ts` (add thread API types and functions)
- Modify: `src/stores/mail.store.ts` (add thread state)
- Modify: `src/features/inbox/InboxView.tsx` (toggle thread/message view)
- Modify: `src/components/MessageList.tsx` (support thread mode)

**Context:** Add "threaded" view mode: message list shows ThreadSummary items, clicking one opens ThreadView with collapsible message bubbles.

- [ ] **Step 1: Add thread types and API to `api.ts`**

```typescript
export interface ThreadSummary { ... }
export async function listThreads(folderId, limit, offset): Promise<ThreadSummary[]>
export async function listThreadMessages(threadId): Promise<Message[]>
```

- [ ] **Step 2: Add thread state to `mail.store.ts`**

Fields: `threadView`, `threads`, `selectedThreadId`, `threadMessages`, `loadingThreadMessages`. Actions: `toggleThreadView`, `fetchThreads`, `selectThread`.

- [ ] **Step 3: Create `ThreadItem.tsx`**

Row showing: participants (up to 3), subject, snippet, message count badge, unread indicator, star, attachment icon, date. 76px height for virtualizer.

- [ ] **Step 4: Create `ThreadMessageBubble.tsx`**

Collapsed: sender, date, snippet (click to expand). Expanded: full headers + rendered HTML body. Most recent starts expanded.

- [ ] **Step 5: Create `ThreadView.tsx`**

Header: back button, subject, count. Body: scrollable list of bubbles (oldest first). Reply button at bottom.

- [ ] **Step 6: Modify `InboxView.tsx`**

Toggle button in header. `threadView=false` → MessageList with messages. `threadView=true` → MessageList with ThreadItem rows → ThreadView on select.

- [ ] **Step 7: Modify `MessageList.tsx`**

Support `mode: "messages" | "threads"` prop for rendering either MessageItem or ThreadItem.

Tests: Vitest for ThreadItem participant rendering, ThreadMessageBubble collapse/expand.

Commit: `feat(ui): add thread/conversation view with collapsible message bubbles`

---

### Task 9: i18n — i18next with Chinese/English dual-language

**Files:**
- Create: `src/locales/en.json`
- Create: `src/locales/zh.json`
- Create: `src/lib/i18n.ts`
- Modify: `src/main.tsx` (import i18n init)
- Modify: `src/stores/ui.store.ts` (add language state)
- Modify: `src/features/settings/AppearanceTab.tsx` (add language selector)
- Modify: all UI components with hardcoded strings

**Context:** Chinese/English dual-language via i18next. Language preference stored in Zustand + localStorage.

- [ ] **Step 1: Install packages**

```bash
pnpm add i18next react-i18next
```

- [ ] **Step 2: Create `src/locales/en.json`**

All user-visible strings: common (loading, save, cancel, delete, close), sidebar (inbox, sent, drafts, trash, archive, spam, kanban, settings), inbox, compose, settings, thread.

- [ ] **Step 3: Create `src/locales/zh.json`**

Chinese translations of all keys.

- [ ] **Step 4: Create `src/lib/i18n.ts`**

Init i18next with resources, saved language from localStorage, fallback "en".

- [ ] **Step 5: Import in `main.tsx`**

Add `import "@/lib/i18n"` before App import.

- [ ] **Step 6: Add language to UI store**

`language: "en" | "zh"`, `setLanguage()` that calls `i18n.changeLanguage()` and saves to localStorage.

- [ ] **Step 7: Add language selector to AppearanceTab**

Below theme selector, add Language section with English/中文 options.

- [ ] **Step 8: Replace hardcoded strings across components**

Use `useTranslation()` hook. Replace strings in MessageList, MessageDetail, Sidebar, SearchBar, ComposeView, SettingsView tabs, and all other components.

Tests: switching language changes text, fallback works.

Commit: `feat(i18n): add Chinese/English dual-language support with i18next`

---

### Task 10: Sync cursor persistence + integration verification

**Files:**
- Modify: `crates/pebble-store/src/accounts.rs` (sync cursor helpers)
- Modify: `crates/pebble-mail/src/sync.rs` (persist cursor, UIDVALIDITY check)

**Context:** SyncWorker doesn't persist the sync cursor — on restart it re-fetches from scratch. Add cursor persistence to `sync_state` JSON so incremental sync survives restarts. Also add UIDVALIDITY check to detect mailbox rebuilds.

- [ ] **Step 1: Add cursor helpers to store**

```rust
pub fn get_sync_cursor(&self, account_id: &str) -> Result<Option<String>>
pub fn set_sync_cursor(&self, account_id: &str, cursor: &str) -> Result<()>
```

Parse/update JSON in `sync_state`, only touching `last_sync_cursor` field.

- [ ] **Step 2: Use cursor in SyncWorker**

On startup: read cursor, use as `since_uid` if present. After each sync: persist new highest UID.

- [ ] **Step 3: Add UIDVALIDITY check**

Store UIDVALIDITY in sync metadata. On each mailbox SELECT, compare with stored value. If changed, discard cursor and full re-sync.

- [ ] **Step 4: Full verification**

```bash
cargo test -p pebble-core -p pebble-store -p pebble-search -p pebble-privacy -p pebble-mail -p pebble-rules -p pebble-translate -p pebble-crypto
cargo clippy --workspace -- -D warnings
npx tsc --noEmit
npx vitest run
```

Tests: set/get cursor round-trip, cursor doesn't overwrite other fields.

Commit: `feat(sync): persist sync cursor and UIDVALIDITY for incremental sync across restarts`

---

## Summary

| Task | Description | Complexity |
|------|-------------|------------|
| 1 | pebble-crypto crate (AES-256-GCM + OS keystore) | Medium |
| 2 | Store auth_data BLOB read/write | Low |
| 3 | Wire encryption into account/sync flow | Medium |
| 4 | Sync flag diff reconciliation + delete detection | High |
| 5 | IMAP IDLE push notifications | High |
| 6 | 30-day tombstone physical cleanup | Low |
| 7 | Thread backend commands | Medium |
| 8 | Thread frontend UI | High |
| 9 | i18n (Chinese/English) | Medium |
| 10 | Sync cursor persistence + verification | Medium |

**Total:** 10 tasks, ~40 steps
