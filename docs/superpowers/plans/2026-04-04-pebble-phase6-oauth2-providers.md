# Pebble Phase 6: OAuth2 Providers + Provider Abstraction + IMAP Enhancements

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the multi-provider architecture prescribed by the spec (§ 4.2–4.3). Add OAuth2 PKCE authentication, Gmail API and Outlook Graph API providers, native IMAP IDLE push notifications, and CONDSTORE/MODSEQ incremental flag sync. Refactor `pebble-mail` from a monolithic IMAP module into a provider-dispatched design via `Box<dyn MailProvider>`.

**Architecture:** New `pebble-oauth` crate handles OAuth2 PKCE with localhost redirect. `pebble-mail` gains a `provider/` module with factory dispatch. `ImapProvider` wraps existing IMAP code behind `MailTransport`; `GmailProvider` and `OutlookProvider` are new HTTP-based providers using `reqwest`. Credentials flow through the existing encrypted `auth_data` path from Phase 5.

**Tech Stack:**
- `oauth2` 5 (RFC 7636 PKCE flow)
- `reqwest` 0.12 (HTTP client for Gmail/Outlook APIs)
- `tokio::net::TcpListener` (localhost OAuth redirect capture)
- `async-trait` 0.1 (trait object safety for async providers)

**Spec:** `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md` § 4.2, § 4.3, § 7.2

**Depends on:** Phase 5 complete (encrypted auth_data, MailTransport trait, ProviderType enum, sync engine)

---

## File Structure (Phase 6 additions)

```
pebble/
├── crates/
│   ├── pebble-oauth/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs          # OAuthManager facade
│   │       ├── pkce.rs         # PKCE challenge/verifier generation
│   │       ├── redirect.rs     # Localhost HTTP listener for redirect URI
│   │       └── tokens.rs       # TokenPair, refresh logic, expiry tracking
│   ├── pebble-core/src/
│   │   ├── traits.rs           # + CategoryProvider, DraftProvider, MailProvider supertrait
│   │   ├── types.rs            # + OAuthTokens, OAuthConfig, MODSEQ types
│   │   └── error.rs            # + OAuth error variants
│   └── pebble-mail/src/
│       ├── provider/
│       │   ├── mod.rs          # create_provider() factory + MailProvider re-export
│       │   ├── imap_provider.rs # ImapProvider implementing MailTransport + FolderProvider
│       │   ├── gmail.rs        # GmailProvider (history.list, labels, drafts)
│       │   └── outlook.rs      # OutlookProvider (delta query, categories, drafts)
│       ├── imap.rs             # + IDLE support (RFC 2177)
│       └── sync.rs             # Refactored to use Box<dyn MailProvider>
├── src-tauri/src/commands/
│   ├── oauth.rs                # start_oauth_flow, oauth_callback commands
│   └── accounts.rs             # Modified: provider-aware account creation
└── src/
    ├── features/settings/
    │   └── AccountsTab.tsx     # Modified: OAuth "Sign in with Google/Microsoft" buttons
    └── components/
        └── OAuthCallback.tsx   # OAuth redirect handler component
```

---

## Task 1: Expand Core Traits — CategoryProvider, DraftProvider, MailProvider

**Why:** The spec (§ 4.2) defines `CategoryProvider` (Outlook), `DraftProvider` (Gmail/Outlook), and a `MailProvider` super-trait combining all sub-traits. Currently only `MailTransport`, `FolderProvider`, `LabelProvider` exist.

### Steps

- [ ] **1.1** Add `async-trait` 0.1 to `pebble-core/Cargo.toml`
- [ ] **1.2** In `pebble-core/src/traits.rs`, add `CategoryProvider` trait:
  ```rust
  #[async_trait]
  pub trait CategoryProvider: Send + Sync {
      async fn list_categories(&self) -> Result<Vec<Category>>;
      async fn set_categories(&self, message_id: &str, categories: &[String]) -> Result<()>;
  }
  ```
- [ ] **1.3** Add `DraftProvider` trait:
  ```rust
  #[async_trait]
  pub trait DraftProvider: Send + Sync {
      async fn save_draft(&self, draft: &DraftMessage) -> Result<String>;
      async fn update_draft(&self, draft_id: &str, draft: &DraftMessage) -> Result<()>;
      async fn delete_draft(&self, draft_id: &str) -> Result<()>;
      async fn list_drafts(&self) -> Result<Vec<DraftMessage>>;
  }
  ```
- [ ] **1.4** Add `MailProvider` super-trait combining `MailTransport + FolderProvider + Send + Sync` with optional sub-traits accessible via `as_label_provider()`, `as_category_provider()`, `as_draft_provider()` downcasting methods that return `Option<&dyn T>`
- [ ] **1.5** Add `Category` and `DraftMessage` types to `pebble-core/src/types.rs`
- [ ] **1.6** Add `OAuthTokens` struct (access_token, refresh_token, expires_at, scopes) and `OAuthConfig` struct (client_id, auth_url, token_url, scopes) to `types.rs`
- [ ] **1.7** Unit tests: verify trait object safety (`Box<dyn MailProvider>` compiles)

**Acceptance:** `cargo check -p pebble-core` passes. Traits are object-safe and can be used as `Box<dyn MailProvider>`.

---

## Task 2: pebble-oauth Crate — OAuth2 PKCE Flow

**Why:** Gmail and Outlook require OAuth2 with PKCE. Centralizing OAuth logic in its own crate keeps it reusable and testable independent of any provider.

### Steps

- [ ] **2.1** Create `crates/pebble-oauth/Cargo.toml` with dependencies: `oauth2` 5, `tokio`, `reqwest`, `serde`, `serde_json`, `url`, `pebble-core` (for error types)
- [ ] **2.2** Add `pebble-oauth` to workspace `Cargo.toml` members
- [ ] **2.3** Implement `pkce.rs`: generate PKCE challenge + verifier using `oauth2::PkceCodeChallenge`
- [ ] **2.4** Implement `redirect.rs`: `start_redirect_listener(port: u16) -> oneshot::Receiver<String>` — binds `127.0.0.1:{port}`, serves a minimal HTML "You can close this tab" page, extracts `?code=` from the redirect
- [ ] **2.5** Implement `tokens.rs`: `TokenPair` struct, `exchange_code()` for initial token exchange, `refresh_if_needed()` for automatic refresh before expiry (5-minute buffer)
- [ ] **2.6** Implement `lib.rs`: `OAuthManager` facade with `start_auth(config: OAuthConfig) -> AuthUrl`, `complete_auth(code: String) -> TokenPair`, `refresh(token: &TokenPair) -> TokenPair`
- [ ] **2.7** Unit tests for PKCE generation, token expiry logic. Integration test (ignored) for redirect listener.

**Acceptance:** `cargo test -p pebble-oauth` passes. PKCE flow works end-to-end in unit tests with mocked HTTP.

---

## Task 3: Expand PebbleError + OAuth Types

**Why:** OAuth introduces new error classes (token expired, refresh failed, consent denied) and the provider factory needs error variants for unsupported provider types.

### Steps

- [ ] **3.1** In `pebble-core/src/error.rs`, add variants:
  - `OAuthError(String)` — general OAuth failures
  - `TokenExpired` — access token expired, refresh needed
  - `TokenRefreshFailed(String)` — refresh token invalid/revoked
  - `UnsupportedProvider(String)` — unknown provider type in factory
- [ ] **3.2** Implement `From<oauth2::RequestTokenError>` for `PebbleError`
- [ ] **3.3** Ensure all new variants serialize properly for Tauri command error responses
- [ ] **3.4** Tests: verify error Display messages are user-friendly

**Acceptance:** `cargo check -p pebble-core` passes with new error variants.

---

## Task 4: Provider Factory + SyncWorker Refactor

**Why:** The sync engine currently hardcodes `ImapProvider`. To support Gmail/Outlook, it must dispatch through `Box<dyn MailProvider>` created by a factory function.

### Steps

- [ ] **4.1** Create `crates/pebble-mail/src/provider/mod.rs` with:
  ```rust
  pub fn create_provider(
      provider_type: ProviderType,
      credentials: &AuthCredentials,
  ) -> Result<Box<dyn MailProvider>>
  ```
- [ ] **4.2** Create `crates/pebble-mail/src/provider/imap_provider.rs`: wrap existing `ImapProvider` to implement `MailTransport` + `FolderProvider` traits. Delegate to existing `imap.rs` methods.
- [ ] **4.3** Stub `provider/gmail.rs` and `provider/outlook.rs` with `todo!()` bodies that return `UnsupportedProvider` error for now
- [ ] **4.4** Refactor `sync.rs`: replace direct `ImapProvider` usage with `create_provider()` → `Box<dyn MailProvider>`. The poll loop calls trait methods instead of IMAP-specific methods.
- [ ] **4.5** Ensure `SyncWorker` stores `Box<dyn MailProvider>` and all sync operations (poll, reconcile, fetch) work through the trait interface
- [ ] **4.6** Integration tests: existing IMAP sync flow still works through the new dispatch layer

**Acceptance:** `cargo test -p pebble-mail` passes. Sync still works for IMAP accounts with no behavioral changes. Factory returns `UnsupportedProvider` for Gmail/Outlook.

---

## Task 5: Gmail API Provider

**Why:** Spec § 4.3 requires Gmail integration via REST API using `history.list(startHistoryId)` for incremental sync, label management, and draft CRUD.

### Steps

- [ ] **5.1** Add `reqwest` 0.12 to `pebble-mail/Cargo.toml`
- [ ] **5.2** Implement `provider/gmail.rs` `GmailProvider` struct holding `reqwest::Client` + `TokenPair`
- [ ] **5.3** Implement `MailTransport` for Gmail:
  - `authenticate()` — validate token, refresh if needed
  - `fetch_messages()` — `GET /gmail/v1/users/me/messages` with pagination
  - `send_message()` — `POST /gmail/v1/users/me/messages/send` (RFC 2822 raw)
  - `sync_changes()` — `GET /gmail/v1/users/me/history` with `startHistoryId`
  - `capabilities()` — labels: true, categories: false, drafts: true, idle: false
- [ ] **5.4** Implement `LabelProvider` for Gmail:
  - `list_labels()` — `GET /gmail/v1/users/me/labels`
  - `modify_labels()` — `POST /gmail/v1/users/me/messages/{id}/modify`
- [ ] **5.5** Implement `DraftProvider` for Gmail:
  - `save_draft()` — `POST /gmail/v1/users/me/drafts`
  - `update_draft()` — `PUT /gmail/v1/users/me/drafts/{id}`
  - `delete_draft()` — `DELETE /gmail/v1/users/me/drafts/{id}`
  - `list_drafts()` — `GET /gmail/v1/users/me/drafts`
- [ ] **5.6** Update factory `create_provider()` to return `GmailProvider` for `ProviderType::Gmail`
- [ ] **5.7** Unit tests with mocked HTTP responses (use `mockito` or similar)

**Acceptance:** `cargo test -p pebble-mail` passes including Gmail provider tests with mocked API.

---

## Task 6: Outlook API Provider

**Why:** Spec § 4.3 requires Outlook/Microsoft 365 integration via Microsoft Graph API using delta queries for incremental sync.

### Steps

- [ ] **6.1** Implement `provider/outlook.rs` `OutlookProvider` struct holding `reqwest::Client` + `TokenPair`
- [ ] **6.2** Implement `MailTransport` for Outlook:
  - `authenticate()` — validate token, refresh if needed
  - `fetch_messages()` — `GET /me/mailFolders/{id}/messages` with `$top/$skip`
  - `send_message()` — `POST /me/sendMail`
  - `sync_changes()` — `GET /me/mailFolders/{id}/messages/delta` with `deltaLink`
  - `capabilities()` — labels: false, categories: true, drafts: true, idle: false
- [ ] **6.3** Implement `FolderProvider` for Outlook:
  - `list_folders()` — `GET /me/mailFolders`
  - `move_message()` — `POST /me/messages/{id}/move`
- [ ] **6.4** Implement `CategoryProvider` for Outlook:
  - `list_categories()` — `GET /me/outlook/masterCategories`
  - `set_categories()` — `PATCH /me/messages/{id}` with categories field
- [ ] **6.5** Implement `DraftProvider` for Outlook:
  - `save_draft()` — `POST /me/messages` (isDraft: true)
  - `update_draft()` — `PATCH /me/messages/{id}`
  - `delete_draft()` — `DELETE /me/messages/{id}`
  - `list_drafts()` — `GET /me/mailFolders/Drafts/messages`
- [ ] **6.6** Update factory `create_provider()` to return `OutlookProvider` for `ProviderType::Outlook`
- [ ] **6.7** Unit tests with mocked HTTP responses

**Acceptance:** `cargo test -p pebble-mail` passes including Outlook provider tests with mocked API.

---

## Task 7: Native IMAP IDLE (RFC 2177)

**Why:** The current IDLE implementation is a UID-count polling fallback. True IMAP IDLE (RFC 2177) keeps a persistent connection open and receives server pushes, reducing latency from 60s to near-instant.

### Steps

- [ ] **7.1** In `imap.rs`, implement `idle_wait()` method:
  ```rust
  pub async fn idle_wait(&mut self, mailbox: &str, timeout: Duration) -> Result<IdleEvent>
  ```
  Uses `async-imap`'s `Session::idle()` with a timeout (29 minutes per RFC recommendation)
- [ ] **7.2** Update `supports_idle()` to check server CAPABILITY response for IDLE
- [ ] **7.3** In `idle.rs`, replace UID-count polling with a branch:
  - If `supports_idle()` → use `idle_wait()` (true push)
  - Else → fall back to existing UID-count comparison (poll)
- [ ] **7.4** Update `sync.rs` poll loop: when IDLE is available, replace `tokio::time::sleep(60s)` with `idle_wait()`. On `IdleEvent::NewMail`, trigger immediate sync. On timeout, do periodic reconcile check.
- [ ] **7.5** Handle IDLE connection drops gracefully: reconnect and re-enter IDLE
- [ ] **7.6** Tests: unit test IdleEvent parsing, integration test (ignored) with real IMAP server

**Acceptance:** `cargo test -p pebble-mail` passes. IMAP IDLE is activated when server supports it; fallback polling still works for servers without IDLE.

---

## Task 8: CONDSTORE / MODSEQ Incremental Flag Sync

**Why:** Currently flag reconciliation fetches ALL flags for ALL UIDs. CONDSTORE (RFC 4551) allows fetching only messages whose flags changed since a known MODSEQ value, dramatically reducing bandwidth.

### Steps

- [ ] **8.1** Add `highest_modseq: Option<u64>` to sync cursor (stored in `sync_state` JSON via `set_sync_cursor`)
- [ ] **8.2** In `imap.rs`, implement `fetch_changed_flags(mailbox, since_modseq)`:
  ```rust
  pub async fn fetch_changed_flags(&mut self, mailbox: &str, since_modseq: u64) -> Result<Vec<FlagChange>>
  ```
  Uses `UID FETCH 1:* (FLAGS) (CHANGEDSINCE {modseq})`
- [ ] **8.3** Check server CAPABILITY for CONDSTORE; store support flag
- [ ] **8.4** In `reconcile.rs`, add `reconcile_with_modseq()`: uses `fetch_changed_flags` instead of fetching all flags. Falls back to full reconcile if CONDSTORE not supported.
- [ ] **8.5** Update `sync.rs` reconcile step: prefer MODSEQ-based reconcile when available, update `highest_modseq` in cursor after each reconcile
- [ ] **8.6** Tests: unit tests for MODSEQ parsing, flag change detection. Integration test (ignored) with CONDSTORE-capable server.

**Acceptance:** `cargo test -p pebble-mail` passes. CONDSTORE-capable servers use incremental flag sync; others fall back to full reconcile.

---

## Task 9: Tauri OAuth Commands + Frontend OAuth UI

**Why:** Users need a way to sign in with Google/Microsoft from the Settings UI. This connects the `pebble-oauth` crate to Tauri commands and provides frontend buttons.

### Steps

- [ ] **9.1** Create `src-tauri/src/commands/oauth.rs`:
  - `start_oauth_flow(provider: String) -> String` — returns auth URL for the system browser
  - `oauth_callback(code: String, provider: String) -> Account` — exchanges code, encrypts tokens, creates account
- [ ] **9.2** Register OAuth commands in `lib.rs`
- [ ] **9.3** Add OAuth config constants (client IDs, auth URLs, token URLs, scopes) for Gmail and Outlook. Use placeholder client IDs with TODO comments for production registration.
- [ ] **9.4** In `src/lib/api.ts`, add `startOAuthFlow(provider: string): Promise<string>` and `oauthCallback(code: string, provider: string): Promise<Account>`
- [ ] **9.5** Modify `AccountsTab.tsx`: add "Sign in with Google" and "Sign in with Microsoft" buttons below the existing "Add Account" button. Clicking opens system browser via `shell.open(authUrl)`.
- [ ] **9.6** Create `src/components/OAuthCallback.tsx`: listens for deep link / redirect callback, calls `oauthCallback`, shows success/error state
- [ ] **9.7** Add i18n keys for OAuth UI strings in `en.json` and `zh.json`
- [ ] **9.8** Test: Tauri command unit tests for OAuth flow (mocked HTTP)

**Acceptance:** OAuth buttons render in Settings. Clicking them opens system browser with correct auth URL. Token exchange path works end-to-end (with mocked HTTP in tests).

---

## Task 10: Integration Testing + Final Wiring

**Why:** Ensure all new providers, IDLE, CONDSTORE, and OAuth work together. Verify no regressions in the existing IMAP-only path.

### Steps

- [ ] **10.1** `cargo test --workspace` — all tests pass (target: ≥90 tests)
- [ ] **10.2** `cargo clippy --workspace -- -D warnings` — no warnings
- [ ] **10.3** `npx tsc --noEmit` — TypeScript clean
- [ ] **10.4** Verify IMAP sync still works end-to-end through the provider factory
- [ ] **10.5** Verify OAuth flow end-to-end (with placeholder client IDs): auth URL generation → token exchange → encrypted storage → account creation
- [ ] **10.6** Verify provider factory returns correct provider type for each `ProviderType` variant
- [ ] **10.7** Run frontend dev server, verify Settings → Accounts shows OAuth buttons, existing account CRUD still works
- [ ] **10.8** Document any TODO items for Phase 7 (production OAuth client registration, WebDAV sync, shortcut customization)

**Acceptance:** All tests pass, no clippy/TypeScript warnings, all three provider paths (IMAP/Gmail/Outlook) are wired through the factory.

---

## Execution Order & Parallelism

```
Task 1 (Core Traits) ──┬──→ Task 4 (Factory + SyncWorker) ──→ Task 10 (Integration)
Task 2 (pebble-oauth) ─┤                                        ↑
Task 3 (Error types) ──┘                                        │
                        Task 5 (Gmail) ─────────────────────────┤
                        Task 6 (Outlook) ───────────────────────┤
                        Task 7 (IMAP IDLE) ─────────────────────┤
                        Task 8 (CONDSTORE) ─────────────────────┤
                        Task 9 (OAuth UI) ──────────────────────┘
```

- **Parallel batch 1:** Tasks 1, 2, 3 (no dependencies between them)
- **Sequential:** Task 4 depends on Tasks 1–3
- **Parallel batch 2:** Tasks 5, 6, 7, 8 (all depend on Task 4 but not each other)
- **Sequential:** Task 9 depends on Tasks 2 + 4
- **Final:** Task 10 depends on all previous tasks
