# Pebble Phase 1: Scaffold + Core + Store + Privacy + Shell

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the foundational project structure, database layer, privacy module, and frontend shell — a launchable app with working layout and database.

**Architecture:** Tauri v2 with React (TypeScript) frontend and Rust backend organized as a Cargo workspace. SQLite via rusqlite for storage, Tailwind CSS for styling. Custom frameless window with drag region titlebar.

**Tech Stack:** Tauri v2, React 19, TypeScript, Rust (2021 edition), rusqlite 0.39, thiserror 2, serde 1, uuid 1, tracing 0.1, aes-gcm 0.10, Zustand, Tailwind CSS 4, React Router v7, Lucide React, Vite, Vitest

**Spec:** `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md`

**Phasing:** This is Phase 1 of 4. Subsequent phases:
- Phase 2: Mail engine (IMAP) + Inbox UI + Search
- Phase 3: Command palette + Kanban + Snooze + Rules + Compose
- Phase 4: Gmail/Outlook providers + Translate + Settings

---

## File Structure (Phase 1)

```
pebble/
├── Cargo.toml                          # workspace root
├── package.json                        # React + Vite + Tailwind
├── tsconfig.json
├── vite.config.ts
├── index.html
├── tailwind.config.ts
├── src-tauri/
│   ├── Cargo.toml                      # Tauri app crate
│   ├── build.rs
│   ├── tauri.conf.json
│   ├── capabilities/
│   │   └── default.json
│   └── src/
│       ├── main.rs                     # Desktop entry
│       ├── lib.rs                      # Tauri setup + command registration
│       ├── state.rs                    # AppState (holds Store, etc.)
│       ├── commands/
│       │   ├── mod.rs
│       │   └── health.rs              # Health check command (Phase 1)
│       └── events.rs                  # Event name constants
├── crates/
│   ├── pebble-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs              # PebbleError enum
│   │       ├── types.rs              # Account, Message, Folder, Label, etc.
│   │       └── traits.rs            # MailTransport, SearchEngine, etc.
│   ├── pebble-store/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── migrations.rs         # Schema creation SQL
│   │       ├── accounts.rs           # Account CRUD
│   │       ├── messages.rs           # Message CRUD
│   │       └── folders.rs            # Folder CRUD
│   └── pebble-privacy/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── sanitizer.rs          # HTML sanitization
│           └── tracker.rs            # Tracker pixel detection
├── src/
│   ├── main.tsx                      # React entry
│   ├── App.tsx                       # Root component + router
│   ├── app/
│   │   └── Layout.tsx                # Shell layout (titlebar + sidebar + main)
│   ├── components/
│   │   ├── TitleBar.tsx              # Custom titlebar with drag region
│   │   ├── Sidebar.tsx               # Navigation sidebar
│   │   └── StatusBar.tsx             # Bottom status bar
│   ├── features/
│   │   └── inbox/
│   │       └── InboxView.tsx         # Placeholder inbox view
│   ├── stores/
│   │   └── ui.store.ts              # UIStore (sidebar, theme, view)
│   ├── lib/
│   │   └── api.ts                   # Tauri IPC wrapper
│   └── styles/
│       └── index.css                # Tailwind imports + global styles
└── tests/                           # Frontend tests
    └── stores/
        └── ui.store.test.ts
```

---

### Task 1: Project Scaffold — Tauri v2 + React + TypeScript

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `package.json`
- Create: `index.html`
- Create: `vite.config.ts`
- Create: `tsconfig.json`
- Create: `tsconfig.node.json`
- Create: `tailwind.config.ts`
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/build.rs`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/capabilities/default.json`
- Create: `src-tauri/src/main.rs`
- Create: `src-tauri/src/lib.rs`
- Create: `src/main.tsx`
- Create: `src/App.tsx`
- Create: `src/styles/index.css`

- [ ] **Step 1: Create workspace root Cargo.toml**

```toml
# Cargo.toml (project root)
[workspace]
members = [
    "src-tauri",
    "crates/pebble-core",
    "crates/pebble-store",
    "crates/pebble-privacy",
]
resolver = "2"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
uuid = { version = "1", features = ["v4", "serde"] }
tracing = "0.1"
tokio = { version = "1", features = ["full"] }
rusqlite = { version = "0.39", features = ["bundled"] }
tauri = { version = "2", features = [] }
tauri-build = { version = "2", features = [] }
```

- [ ] **Step 2: Create src-tauri/Cargo.toml**

```toml
# src-tauri/Cargo.toml
[package]
name = "pebble"
version = "0.1.0"
edition = "2021"

[lib]
name = "pebble_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[dependencies]
tauri = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tokio = { workspace = true }
pebble-core = { path = "../crates/pebble-core" }
pebble-store = { path = "../crates/pebble-store" }
pebble-privacy = { path = "../crates/pebble-privacy" }

[build-dependencies]
tauri-build = { workspace = true }
```

- [ ] **Step 3: Create src-tauri/build.rs**

```rust
// src-tauri/build.rs
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 4: Create src-tauri/tauri.conf.json**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Pebble",
  "version": "0.1.0",
  "identifier": "com.pebble.app",
  "build": {
    "frontendDist": "../dist",
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "pnpm dev:frontend",
    "beforeBuildCommand": "pnpm build:frontend"
  },
  "app": {
    "windows": [
      {
        "label": "main",
        "title": "Pebble",
        "width": 1200,
        "height": 800,
        "minWidth": 800,
        "minHeight": 600,
        "decorations": false,
        "resizable": true,
        "center": true
      }
    ]
  }
}
```

- [ ] **Step 5: Create src-tauri/capabilities/default.json**

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main-capability",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "core:window:default",
    "core:window:allow-start-dragging",
    "core:window:allow-minimize",
    "core:window:allow-close",
    "core:window:allow-toggle-maximize",
    "core:window:allow-is-maximized"
  ]
}
```

- [ ] **Step 6: Create src-tauri/src/main.rs and src-tauri/src/lib.rs**

```rust
// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    pebble_lib::run();
}
```

```rust
// src-tauri/src/lib.rs
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! Pebble is running.", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pebble=debug,pebble_store=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 7: Initialize npm project with package.json**

```json
{
  "name": "pebble",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev:frontend": "vite",
    "build:frontend": "tsc && vite build",
    "preview": "vite preview",
    "dev": "tauri dev",
    "build": "tauri build",
    "test": "vitest run",
    "test:watch": "vitest"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "react-router": "^7.0.0",
    "zustand": "^5.0.0",
    "lucide-react": "^0.500.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^4.0.0",
    "tailwindcss": "^4.0.0",
    "@tailwindcss/vite": "^4.0.0",
    "typescript": "^5.7.0",
    "vite": "^6.0.0",
    "vitest": "^3.0.0",
    "@testing-library/react": "^16.0.0",
    "@testing-library/jest-dom": "^6.0.0",
    "jsdom": "^26.0.0"
  }
}
```

- [ ] **Step 8: Create vite.config.ts**

```typescript
// vite.config.ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
```

- [ ] **Step 9: Create tsconfig.json and tsconfig.node.json**

```json
// tsconfig.json
{
  "compilerOptions": {
    "target": "ES2021",
    "useDefineForClassFields": true,
    "lib": ["ES2021", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedSideEffectImports": true,
    "baseUrl": ".",
    "paths": {
      "@/*": ["src/*"]
    }
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

```json
// tsconfig.node.json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2023"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedSideEffectImports": true
  },
  "include": ["vite.config.ts"]
}
```

- [ ] **Step 10: Create index.html**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Pebble</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 11: Create src/styles/index.css**

```css
/* src/styles/index.css */
@import "tailwindcss";

:root {
  --color-sidebar-bg: #f8f7f5;
  --color-sidebar-hover: #eeedeb;
  --color-sidebar-active: #e5e4e2;
  --color-titlebar-bg: #f8f7f5;
  --color-main-bg: #ffffff;
  --color-statusbar-bg: #f8f7f5;
  --color-border: #e5e4e2;
  --color-text-primary: #1a1a1a;
  --color-text-secondary: #6b6b6b;
  --color-accent: #d4714e;
}

html, body, #root {
  height: 100%;
  margin: 0;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  color: var(--color-text-primary);
  background: var(--color-main-bg);
}
```

- [ ] **Step 12: Create src/main.tsx and src/App.tsx (minimal)**

```tsx
// src/main.tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles/index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
```

```tsx
// src/App.tsx
export default function App() {
  return <div className="h-full flex items-center justify-center">Pebble</div>;
}
```

- [ ] **Step 13: Install dependencies and verify build**

Run:
```bash
pnpm install
```
Expected: Dependencies installed successfully.

Run:
```bash
cd src-tauri && cargo check
```
Expected: Compiles with no errors (warnings OK for unused code at this stage).

- [ ] **Step 14: Commit scaffold**

```bash
git add -A
git commit -m "feat: scaffold Tauri v2 + React + TypeScript project

Workspace with pebble-core, pebble-store, pebble-privacy crates.
Custom frameless window, Tailwind CSS 4, Vite dev server."
```

---

### Task 2: pebble-core — Types, Traits, and Errors

**Files:**
- Create: `crates/pebble-core/Cargo.toml`
- Create: `crates/pebble-core/src/lib.rs`
- Create: `crates/pebble-core/src/error.rs`
- Create: `crates/pebble-core/src/types.rs`
- Create: `crates/pebble-core/src/traits.rs`

- [ ] **Step 1: Create crates/pebble-core/Cargo.toml**

```toml
# crates/pebble-core/Cargo.toml
[package]
name = "pebble-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true }
```

- [ ] **Step 2: Create error.rs with PebbleError**

```rust
// crates/pebble-core/src/error.rs
use serde::Serialize;

#[derive(thiserror::Error, Debug, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum PebbleError {
    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Sync error: {0}")]
    Sync(String),

    #[error("Rule error: {0}")]
    Rule(String),

    #[error("Translate error: {0}")]
    Translate(String),

    #[error("Privacy error: {0}")]
    Privacy(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, PebbleError>;
```

- [ ] **Step 3: Create types.rs with core data types**

```rust
// crates/pebble-core/src/types.rs
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// === Account ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub provider: ProviderType,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    Imap,
    Gmail,
    Outlook,
}

// === Folder ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub name: String,
    pub folder_type: FolderType,
    pub role: Option<FolderRole>,
    pub parent_id: Option<String>,
    pub color: Option<String>,
    pub is_system: bool,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FolderType {
    Folder,
    Label,
    Category,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Archive,
    Spam,
}

// === Message ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_header: Option<String>,
    pub thread_id: Option<String>,
    pub subject: String,
    pub snippet: String,
    pub from_address: String,
    pub from_name: String,
    pub to_list: Vec<EmailAddress>,
    pub cc_list: Vec<EmailAddress>,
    pub bcc_list: Vec<EmailAddress>,
    pub body_text: String,
    pub body_html_raw: String,
    pub has_attachments: bool,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_draft: bool,
    pub date: i64,
    pub remote_version: Option<String>,
    pub is_deleted: bool,
    pub deleted_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: String,
}

// === Attachment ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub message_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub local_path: Option<String>,
}

// === User Label (Pebble-local, distinct from provider folders) ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserLabel {
    pub id: String,
    pub name: String,
    pub color: String,
    pub is_system: bool,
    pub rule_id: Option<String>,
}

// === Kanban ===

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KanbanColumn {
    Todo,
    Waiting,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanbanCard {
    pub message_id: String,
    pub column: KanbanColumn,
    pub position: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

// === Snooze ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnoozedMessage {
    pub message_id: String,
    pub snoozed_at: i64,
    pub unsnoozed_at: i64,
    pub return_to: String,
}

// === Trusted Sender ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedSender {
    pub account_id: String,
    pub email: String,
    pub trust_type: TrustType,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TrustType {
    Images,
    All,
}

// === Privacy ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrivacyMode {
    Strict,
    TrustSender(String),
    LoadOnce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedHtml {
    pub html: String,
    pub trackers_blocked: Vec<TrackerInfo>,
    pub images_blocked: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerInfo {
    pub domain: String,
    pub tracker_type: String,
}

// === Provider Capabilities ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub has_labels: bool,
    pub has_folders: bool,
    pub has_categories: bool,
    pub has_push: bool,
    pub has_threads: bool,
}

// === Helpers ===

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn now_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
```

- [ ] **Step 4: Create traits.rs with provider and search traits**

```rust
// crates/pebble-core/src/traits.rs
use crate::error::Result;
use crate::types::*;

// === Sync types ===

pub struct FetchQuery {
    pub folder_id: String,
    pub limit: Option<u32>,
}

pub struct FetchResult {
    pub messages: Vec<Message>,
    pub cursor: SyncCursor,
}

#[derive(Debug, Clone)]
pub struct SyncCursor {
    pub value: String,
}

pub struct ChangeSet {
    pub new_messages: Vec<Message>,
    pub flag_changes: Vec<FlagChange>,
    pub moved: Vec<MoveChange>,
    pub deleted: Vec<String>,
    pub cursor: SyncCursor,
}

pub struct FlagChange {
    pub remote_id: String,
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

pub struct MoveChange {
    pub remote_id: String,
    pub from_folder: String,
    pub to_folder: String,
}

// === Mail Transport (required for all providers) ===

pub trait MailTransport: Send + Sync {
    fn authenticate(
        &mut self,
        credentials: &AuthCredentials,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn fetch_messages(
        &self,
        query: &FetchQuery,
    ) -> impl std::future::Future<Output = Result<FetchResult>> + Send;

    fn send_message(
        &self,
        message: &OutgoingMessage,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn sync_changes(
        &self,
        since: &SyncCursor,
    ) -> impl std::future::Future<Output = Result<ChangeSet>> + Send;

    fn capabilities(&self) -> ProviderCapabilities;
}

pub struct AuthCredentials {
    pub provider: ProviderType,
    pub data: serde_json::Value,
}

pub struct OutgoingMessage {
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
}

// === Capability traits (optional per provider) ===

pub trait FolderProvider: Send + Sync {
    fn list_folders(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<Folder>>> + Send;

    fn move_message(
        &self,
        remote_id: &str,
        to_folder_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}

pub trait LabelProvider: Send + Sync {
    fn list_labels(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<Folder>>> + Send;

    fn modify_labels(
        &self,
        remote_id: &str,
        add: &[String],
        remove: &[String],
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}

// === Search Engine ===

pub struct StructuredQuery {
    pub text: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub has_attachment: Option<bool>,
    pub folder_id: Option<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
}

pub enum SearchQuery {
    Structured(StructuredQuery),
}

pub struct SearchHit {
    pub message_id: String,
    pub score: f32,
    pub snippet: String,
}

pub trait SearchEngine: Send + Sync {
    fn index_message(
        &self,
        message: &Message,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn search(
        &self,
        query: &SearchQuery,
    ) -> impl std::future::Future<Output = Result<Vec<SearchHit>>> + Send;

    fn rebuild_index(&self) -> impl std::future::Future<Output = Result<()>> + Send;
}
```

- [ ] **Step 5: Create lib.rs re-exports**

```rust
// crates/pebble-core/src/lib.rs
pub mod error;
pub mod traits;
pub mod types;

pub use error::{PebbleError, Result};
pub use types::*;
```

- [ ] **Step 6: Run cargo check**

Run:
```bash
cargo check -p pebble-core
```
Expected: Compiles with no errors.

- [ ] **Step 7: Commit**

```bash
git add crates/pebble-core/
git commit -m "feat(core): add types, traits, and error definitions

PebbleError enum, core data types (Account, Message, Folder, etc.),
MailTransport/FolderProvider/LabelProvider/SearchEngine traits,
ProviderCapabilities for runtime capability queries."
```

---

### Task 3: pebble-store — SQLite Schema and CRUD

**Files:**
- Create: `crates/pebble-store/Cargo.toml`
- Create: `crates/pebble-store/src/lib.rs`
- Create: `crates/pebble-store/src/migrations.rs`
- Create: `crates/pebble-store/src/accounts.rs`
- Create: `crates/pebble-store/src/messages.rs`
- Create: `crates/pebble-store/src/folders.rs`

- [ ] **Step 1: Create crates/pebble-store/Cargo.toml**

```toml
# crates/pebble-store/Cargo.toml
[package]
name = "pebble-store"
version = "0.1.0"
edition = "2021"

[dependencies]
pebble-core = { path = "../pebble-core" }
rusqlite = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Write the failing test for database initialization**

```rust
// crates/pebble-store/src/lib.rs
pub mod migrations;
pub mod accounts;
pub mod messages;
pub mod folders;

use pebble_core::{PebbleError, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
        let store = Self { conn: Mutex::new(conn) };
        store.initialize()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
        let store = Self { conn: Mutex::new(conn) };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        migrations::run_migrations(&conn)
    }

    pub(crate) fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let store = Store::open_in_memory();
        assert!(store.is_ok(), "Failed to open in-memory store: {:?}", store.err());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run:
```bash
cargo test -p pebble-store -- test_open_in_memory
```
Expected: FAIL — `migrations` module doesn't exist yet or `run_migrations` not defined.

- [ ] **Step 4: Create migrations.rs with schema SQL**

```rust
// crates/pebble-store/src/migrations.rs
use pebble_core::{PebbleError, Result};
use rusqlite::Connection;

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| PebbleError::Storage(e.to_string()))?;

    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| PebbleError::Storage(e.to_string()))?;

    conn.execute_batch(SCHEMA_V1)
        .map_err(|e| PebbleError::Storage(format!("Migration failed: {e}")))?;

    Ok(())
}

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL CHECK(provider IN ('imap', 'gmail', 'outlook')),
    auth_data BLOB,
    sync_state TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS folders (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL,
    name TEXT NOT NULL,
    folder_type TEXT NOT NULL CHECK(folder_type IN ('folder', 'label', 'category')),
    role TEXT CHECK(role IN ('inbox', 'sent', 'drafts', 'trash', 'archive', 'spam')),
    parent_id TEXT,
    color TEXT,
    is_system INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_folders_account ON folders(account_id);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL,
    message_id_header TEXT,
    in_reply_to TEXT,
    references_header TEXT,
    thread_id TEXT,
    subject TEXT NOT NULL DEFAULT '',
    snippet TEXT NOT NULL DEFAULT '',
    from_address TEXT NOT NULL DEFAULT '',
    from_name TEXT NOT NULL DEFAULT '',
    to_list TEXT NOT NULL DEFAULT '[]',
    cc_list TEXT NOT NULL DEFAULT '[]',
    bcc_list TEXT NOT NULL DEFAULT '[]',
    body_text TEXT NOT NULL DEFAULT '',
    body_html_raw TEXT NOT NULL DEFAULT '',
    has_attachments INTEGER NOT NULL DEFAULT 0,
    is_read INTEGER NOT NULL DEFAULT 0,
    is_starred INTEGER NOT NULL DEFAULT 0,
    is_draft INTEGER NOT NULL DEFAULT 0,
    date INTEGER NOT NULL,
    raw_headers TEXT,
    remote_version TEXT,
    is_deleted INTEGER NOT NULL DEFAULT 0,
    deleted_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_messages_account ON messages(account_id);
CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id);
CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(date);
CREATE INDEX IF NOT EXISTS idx_messages_message_id_header ON messages(message_id_header);

CREATE TABLE IF NOT EXISTS message_folders (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, folder_id)
);

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename TEXT NOT NULL DEFAULT '',
    mime_type TEXT NOT NULL DEFAULT '',
    size INTEGER NOT NULL DEFAULT 0,
    local_path TEXT
);

CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments(message_id);

CREATE TABLE IF NOT EXISTS labels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '#808080',
    is_system INTEGER NOT NULL DEFAULT 0,
    rule_id TEXT
);

CREATE TABLE IF NOT EXISTS message_labels (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, label_id)
);

CREATE TABLE IF NOT EXISTS kanban_cards (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    column_name TEXT NOT NULL CHECK(column_name IN ('todo', 'waiting', 'done')),
    position INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS snoozed_messages (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    snoozed_at INTEGER NOT NULL,
    unsnoozed_at INTEGER NOT NULL,
    return_to TEXT NOT NULL DEFAULT 'inbox'
);

CREATE TABLE IF NOT EXISTS trusted_senders (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    trust_type TEXT NOT NULL CHECK(trust_type IN ('images', 'all')),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, email)
);

CREATE TABLE IF NOT EXISTS rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    conditions TEXT NOT NULL DEFAULT '{}',
    actions TEXT NOT NULL DEFAULT '[]',
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
"#;
```

- [ ] **Step 5: Run test to verify it passes**

Run:
```bash
cargo test -p pebble-store -- test_open_in_memory
```
Expected: PASS

- [ ] **Step 6: Write failing test for account CRUD**

```rust
// Add to crates/pebble-store/src/lib.rs tests module:

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::ProviderType;

    #[test]
    fn test_open_in_memory() {
        let store = Store::open_in_memory();
        assert!(store.is_ok());
    }

    #[test]
    fn test_account_crud() {
        let store = Store::open_in_memory().unwrap();

        // Insert
        let account = pebble_core::Account {
            id: pebble_core::new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test User".to_string(),
            provider: ProviderType::Imap,
            created_at: pebble_core::now_timestamp(),
            updated_at: pebble_core::now_timestamp(),
        };
        store.insert_account(&account).unwrap();

        // Read
        let fetched = store.get_account(&account.id).unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.email, "test@example.com");
        assert_eq!(fetched.provider, ProviderType::Imap);

        // List
        let accounts = store.list_accounts().unwrap();
        assert_eq!(accounts.len(), 1);

        // Delete
        store.delete_account(&account.id).unwrap();
        let accounts = store.list_accounts().unwrap();
        assert_eq!(accounts.len(), 0);
    }
}
```

- [ ] **Step 7: Run test to verify it fails**

Run:
```bash
cargo test -p pebble-store -- test_account_crud
```
Expected: FAIL — `insert_account`, `get_account`, etc. not defined.

- [ ] **Step 8: Implement accounts.rs**

```rust
// crates/pebble-store/src/accounts.rs
use crate::Store;
use pebble_core::{Account, PebbleError, ProviderType, Result};

impl Store {
    pub fn insert_account(&self, account: &Account) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, provider, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    account.id,
                    account.email,
                    account.display_name,
                    provider_to_str(&account.provider),
                    account.created_at,
                    account.updated_at,
                ],
            )
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, email, display_name, provider, created_at, updated_at
                     FROM accounts WHERE id = ?1",
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            let result = stmt
                .query_row(rusqlite::params![id], |row| {
                    Ok(Account {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        provider: str_to_provider(&row.get::<_, String>(3)?),
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                })
                .optional()
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            Ok(result)
        })
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, email, display_name, provider, created_at, updated_at
                     FROM accounts ORDER BY created_at",
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            let accounts = stmt
                .query_map([], |row| {
                    Ok(Account {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        provider: str_to_provider(&row.get::<_, String>(3)?),
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                })
                .map_err(|e| PebbleError::Storage(e.to_string()))?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            Ok(accounts)
        })
    }

    pub fn delete_account(&self, id: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM accounts WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }
}

fn provider_to_str(p: &ProviderType) -> &'static str {
    match p {
        ProviderType::Imap => "imap",
        ProviderType::Gmail => "gmail",
        ProviderType::Outlook => "outlook",
    }
}

fn str_to_provider(s: &str) -> ProviderType {
    match s {
        "gmail" => ProviderType::Gmail,
        "outlook" => ProviderType::Outlook,
        _ => ProviderType::Imap,
    }
}
```

We need to add the `optional()` import. Add this to the top of `accounts.rs`:

```rust
use rusqlite::OptionalExtension;
```

- [ ] **Step 9: Run test to verify it passes**

Run:
```bash
cargo test -p pebble-store -- test_account_crud
```
Expected: PASS

- [ ] **Step 10: Write failing test for folder CRUD**

Add to `crates/pebble-store/src/lib.rs` tests:

```rust
    #[test]
    fn test_folder_crud() {
        let store = Store::open_in_memory().unwrap();

        let account = pebble_core::Account {
            id: pebble_core::new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            provider: ProviderType::Imap,
            created_at: pebble_core::now_timestamp(),
            updated_at: pebble_core::now_timestamp(),
        };
        store.insert_account(&account).unwrap();

        let folder = pebble_core::Folder {
            id: pebble_core::new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: pebble_core::FolderType::Folder,
            role: Some(pebble_core::FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();

        let folders = store.list_folders(&account.id).unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "Inbox");
        assert_eq!(folders[0].role, Some(pebble_core::FolderRole::Inbox));
    }
```

- [ ] **Step 11: Implement folders.rs**

```rust
// crates/pebble-store/src/folders.rs
use crate::Store;
use pebble_core::{Folder, FolderRole, FolderType, PebbleError, Result};

impl Store {
    pub fn insert_folder(&self, folder: &Folder) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO folders (id, account_id, remote_id, name, folder_type, role, parent_id, color, is_system, sort_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    folder.id,
                    folder.account_id,
                    folder.remote_id,
                    folder.name,
                    folder_type_to_str(&folder.folder_type),
                    folder.role.as_ref().map(folder_role_to_str),
                    folder.parent_id,
                    folder.color,
                    folder.is_system as i32,
                    folder.sort_order,
                ],
            )
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }

    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, remote_id, name, folder_type, role, parent_id, color, is_system, sort_order
                     FROM folders WHERE account_id = ?1 ORDER BY sort_order",
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            let folders = stmt
                .query_map(rusqlite::params![account_id], |row| {
                    Ok(Folder {
                        id: row.get(0)?,
                        account_id: row.get(1)?,
                        remote_id: row.get(2)?,
                        name: row.get(3)?,
                        folder_type: str_to_folder_type(&row.get::<_, String>(4)?),
                        role: row.get::<_, Option<String>>(5)?.map(|s| str_to_folder_role(&s)),
                        parent_id: row.get(6)?,
                        color: row.get(7)?,
                        is_system: row.get::<_, i32>(8)? != 0,
                        sort_order: row.get(9)?,
                    })
                })
                .map_err(|e| PebbleError::Storage(e.to_string()))?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            Ok(folders)
        })
    }
}

fn folder_type_to_str(ft: &FolderType) -> &'static str {
    match ft {
        FolderType::Folder => "folder",
        FolderType::Label => "label",
        FolderType::Category => "category",
    }
}

fn str_to_folder_type(s: &str) -> FolderType {
    match s {
        "label" => FolderType::Label,
        "category" => FolderType::Category,
        _ => FolderType::Folder,
    }
}

fn folder_role_to_str(r: &FolderRole) -> &'static str {
    match r {
        FolderRole::Inbox => "inbox",
        FolderRole::Sent => "sent",
        FolderRole::Drafts => "drafts",
        FolderRole::Trash => "trash",
        FolderRole::Archive => "archive",
        FolderRole::Spam => "spam",
    }
}

fn str_to_folder_role(s: &str) -> FolderRole {
    match s {
        "sent" => FolderRole::Sent,
        "drafts" => FolderRole::Drafts,
        "trash" => FolderRole::Trash,
        "archive" => FolderRole::Archive,
        "spam" => FolderRole::Spam,
        _ => FolderRole::Inbox,
    }
}
```

- [ ] **Step 12: Run test to verify it passes**

Run:
```bash
cargo test -p pebble-store -- test_folder_crud
```
Expected: PASS

- [ ] **Step 13: Write failing test for message insert + query by folder**

Add to tests:

```rust
    #[test]
    fn test_message_insert_and_query() {
        let store = Store::open_in_memory().unwrap();
        let now = pebble_core::now_timestamp();

        let account = pebble_core::Account {
            id: pebble_core::new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();

        let folder = pebble_core::Folder {
            id: pebble_core::new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: pebble_core::FolderType::Folder,
            role: Some(pebble_core::FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();

        let msg = pebble_core::Message {
            id: pebble_core::new_id(),
            account_id: account.id.clone(),
            remote_id: "12345".to_string(),
            message_id_header: Some("<abc@example.com>".to_string()),
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Hello World".to_string(),
            snippet: "This is a test...".to_string(),
            from_address: "sender@example.com".to_string(),
            from_name: "Sender".to_string(),
            to_list: vec![pebble_core::EmailAddress {
                name: Some("Test".to_string()),
                address: "test@example.com".to_string(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "This is a test email.".to_string(),
            body_html_raw: "<p>This is a test email.</p>".to_string(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };
        store.insert_message(&msg, &[folder.id.clone()]).unwrap();

        // Query by folder
        let messages = store.list_messages_by_folder(&folder.id, 50, 0).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].subject, "Hello World");
        assert_eq!(messages[0].from_address, "sender@example.com");
    }
```

- [ ] **Step 14: Implement messages.rs**

```rust
// crates/pebble-store/src/messages.rs
use crate::Store;
use pebble_core::{EmailAddress, Message, PebbleError, Result};

impl Store {
    pub fn insert_message(&self, msg: &Message, folder_ids: &[String]) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO messages (
                    id, account_id, remote_id, message_id_header, in_reply_to,
                    references_header, thread_id, subject, snippet,
                    from_address, from_name, to_list, cc_list, bcc_list,
                    body_text, body_html_raw, has_attachments,
                    is_read, is_starred, is_draft, date, raw_headers,
                    remote_version, is_deleted, deleted_at, created_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27
                )",
                rusqlite::params![
                    msg.id,
                    msg.account_id,
                    msg.remote_id,
                    msg.message_id_header,
                    msg.in_reply_to,
                    msg.references_header,
                    msg.thread_id,
                    msg.subject,
                    msg.snippet,
                    msg.from_address,
                    msg.from_name,
                    serde_json::to_string(&msg.to_list).unwrap_or_default(),
                    serde_json::to_string(&msg.cc_list).unwrap_or_default(),
                    serde_json::to_string(&msg.bcc_list).unwrap_or_default(),
                    msg.body_text,
                    msg.body_html_raw,
                    msg.has_attachments as i32,
                    msg.is_read as i32,
                    msg.is_starred as i32,
                    msg.is_draft as i32,
                    msg.date,
                    Option::<String>::None,
                    msg.remote_version,
                    msg.is_deleted as i32,
                    msg.deleted_at,
                    msg.created_at,
                    msg.updated_at,
                ],
            )
            .map_err(|e| PebbleError::Storage(e.to_string()))?;

            for folder_id in folder_ids {
                conn.execute(
                    "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, ?2)",
                    rusqlite::params![msg.id, folder_id],
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            }

            Ok(())
        })
    }

    pub fn list_messages_by_folder(
        &self,
        folder_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Message>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT m.id, m.account_id, m.remote_id, m.message_id_header,
                            m.in_reply_to, m.references_header, m.thread_id,
                            m.subject, m.snippet, m.from_address, m.from_name,
                            m.to_list, m.cc_list, m.bcc_list,
                            m.body_text, m.body_html_raw, m.has_attachments,
                            m.is_read, m.is_starred, m.is_draft, m.date,
                            m.remote_version, m.is_deleted, m.deleted_at,
                            m.created_at, m.updated_at
                     FROM messages m
                     JOIN message_folders mf ON m.id = mf.message_id
                     WHERE mf.folder_id = ?1 AND m.is_deleted = 0
                     ORDER BY m.date DESC
                     LIMIT ?2 OFFSET ?3",
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            let messages = stmt
                .query_map(rusqlite::params![folder_id, limit, offset], |row| {
                    Ok(row_to_message(row))
                })
                .map_err(|e| PebbleError::Storage(e.to_string()))?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            Ok(messages)
        })
    }

    pub fn get_message(&self, id: &str) -> Result<Option<Message>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, remote_id, message_id_header,
                            in_reply_to, references_header, thread_id,
                            subject, snippet, from_address, from_name,
                            to_list, cc_list, bcc_list,
                            body_text, body_html_raw, has_attachments,
                            is_read, is_starred, is_draft, date,
                            remote_version, is_deleted, deleted_at,
                            created_at, updated_at
                     FROM messages WHERE id = ?1",
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            use rusqlite::OptionalExtension;
            let result = stmt
                .query_row(rusqlite::params![id], |row| Ok(row_to_message(row)))
                .optional()
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            Ok(result)
        })
    }

    pub fn update_message_flags(
        &self,
        id: &str,
        is_read: Option<bool>,
        is_starred: Option<bool>,
    ) -> Result<()> {
        self.with_conn(|conn| {
            if let Some(read) = is_read {
                conn.execute(
                    "UPDATE messages SET is_read = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![read as i32, pebble_core::now_timestamp(), id],
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            }
            if let Some(starred) = is_starred {
                conn.execute(
                    "UPDATE messages SET is_starred = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![starred as i32, pebble_core::now_timestamp(), id],
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            }
            Ok(())
        })
    }

    pub fn soft_delete_message(&self, id: &str) -> Result<()> {
        let now = pebble_core::now_timestamp();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE messages SET is_deleted = 1, deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }
}

fn row_to_message(row: &rusqlite::Row<'_>) -> Message {
    fn parse_addresses(json: &str) -> Vec<EmailAddress> {
        serde_json::from_str(json).unwrap_or_default()
    }

    Message {
        id: row.get(0).unwrap(),
        account_id: row.get(1).unwrap(),
        remote_id: row.get(2).unwrap(),
        message_id_header: row.get(3).unwrap(),
        in_reply_to: row.get(4).unwrap(),
        references_header: row.get(5).unwrap(),
        thread_id: row.get(6).unwrap(),
        subject: row.get(7).unwrap(),
        snippet: row.get(8).unwrap(),
        from_address: row.get(9).unwrap(),
        from_name: row.get(10).unwrap(),
        to_list: parse_addresses(&row.get::<_, String>(11).unwrap()),
        cc_list: parse_addresses(&row.get::<_, String>(12).unwrap()),
        bcc_list: parse_addresses(&row.get::<_, String>(13).unwrap()),
        body_text: row.get(14).unwrap(),
        body_html_raw: row.get(15).unwrap(),
        has_attachments: row.get::<_, i32>(16).unwrap() != 0,
        is_read: row.get::<_, i32>(17).unwrap() != 0,
        is_starred: row.get::<_, i32>(18).unwrap() != 0,
        is_draft: row.get::<_, i32>(19).unwrap() != 0,
        date: row.get(20).unwrap(),
        remote_version: row.get(21).unwrap(),
        is_deleted: row.get::<_, i32>(22).unwrap() != 0,
        deleted_at: row.get(23).unwrap(),
        created_at: row.get(24).unwrap(),
        updated_at: row.get(25).unwrap(),
    }
}
```

- [ ] **Step 15: Run all store tests**

Run:
```bash
cargo test -p pebble-store
```
Expected: All 3 tests pass (test_open_in_memory, test_account_crud, test_folder_crud, test_message_insert_and_query).

- [ ] **Step 16: Commit**

```bash
git add crates/pebble-store/
git commit -m "feat(store): SQLite schema, migrations, account/folder/message CRUD

Full schema with all tables from spec. Account, folder, and message
CRUD with multi-folder message association (message_folders join table).
Soft delete with tombstone support."
```

---

### Task 4: pebble-privacy — HTML Sanitizer and Tracker Detection

**Files:**
- Create: `crates/pebble-privacy/Cargo.toml`
- Create: `crates/pebble-privacy/src/lib.rs`
- Create: `crates/pebble-privacy/src/sanitizer.rs`
- Create: `crates/pebble-privacy/src/tracker.rs`

- [ ] **Step 1: Create crates/pebble-privacy/Cargo.toml**

```toml
# crates/pebble-privacy/Cargo.toml
[package]
name = "pebble-privacy"
version = "0.1.0"
edition = "2021"

[dependencies]
pebble-core = { path = "../pebble-core" }
```

- [ ] **Step 2: Write failing test for tracker detection**

```rust
// crates/pebble-privacy/src/lib.rs
pub mod sanitizer;
pub mod tracker;

pub use sanitizer::PrivacyGuard;
```

```rust
// crates/pebble-privacy/src/tracker.rs

/// Known email tracking domains
const KNOWN_TRACKERS: &[&str] = &[
    "mailchimp.com",
    "list-manage.com",
    "hubspot.com",
    "sendgrid.net",
    "mailgun.org",
    "constantcontact.com",
    "campaign-archive.com",
    "exacttarget.com",
    "sailthru.com",
    "returnpath.net",
    "litmus.com",
    "bananatag.com",
    "yesware.com",
    "mailtrack.io",
    "getnotify.com",
    "streak.com",
    "cirrusinsight.com",
    "boomeranggmail.com",
    "mixmax.com",
    "superhuman.com",
    "facebook.com/tr",
    "google-analytics.com",
    "doubleclick.net",
    "pixel.wp.com",
    "open.convertkit.com",
    "cmail19.com",
    "cmail20.com",
    "createsend.com",
    "intercom.io",
    "drip.com",
    "mandrillapp.com",
];

pub fn is_known_tracker(domain: &str) -> bool {
    let domain_lower = domain.to_lowercase();
    KNOWN_TRACKERS.iter().any(|t| domain_lower.contains(t))
}

/// Detect if an img tag is likely a tracking pixel based on attributes
pub fn is_tracking_pixel(width: Option<&str>, height: Option<&str>) -> bool {
    let w = width.and_then(|v| v.parse::<u32>().ok()).unwrap_or(u32::MAX);
    let h = height.and_then(|v| v.parse::<u32>().ok()).unwrap_or(u32::MAX);
    w <= 1 && h <= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_tracker_domains() {
        assert!(is_known_tracker("open.mailchimp.com"));
        assert!(is_known_tracker("track.hubspot.com"));
        assert!(is_known_tracker("subdomain.sendgrid.net"));
        assert!(!is_known_tracker("example.com"));
        assert!(!is_known_tracker("google.com"));
    }

    #[test]
    fn test_tracking_pixel_detection() {
        assert!(is_tracking_pixel(Some("1"), Some("1")));
        assert!(is_tracking_pixel(Some("0"), Some("0")));
        assert!(!is_tracking_pixel(Some("100"), Some("50")));
        assert!(!is_tracking_pixel(None, None));
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run:
```bash
cargo test -p pebble-privacy
```
Expected: PASS (2 tests)

- [ ] **Step 4: Write failing test for HTML sanitizer**

```rust
// crates/pebble-privacy/src/sanitizer.rs
use pebble_core::{PrivacyMode, RenderedHtml, TrackerInfo};

pub struct PrivacyGuard {
    custom_tracker_domains: Vec<String>,
}

impl PrivacyGuard {
    pub fn new() -> Self {
        Self {
            custom_tracker_domains: Vec::new(),
        }
    }

    pub fn add_custom_tracker(&mut self, domain: String) {
        self.custom_tracker_domains.push(domain);
    }

    pub fn render_safe_html(&self, raw_html: &str, mode: &PrivacyMode) -> RenderedHtml {
        let mut trackers_blocked = Vec::new();
        let mut images_blocked: u32 = 0;
        let mut result = String::with_capacity(raw_html.len());

        let allow_images = matches!(mode, PrivacyMode::LoadOnce | PrivacyMode::TrustSender(_));

        // Simple tag-based processing
        let mut chars = raw_html.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '<' {
                let mut tag = String::from('<');
                for tc in chars.by_ref() {
                    tag.push(tc);
                    if tc == '>' {
                        break;
                    }
                }
                let tag_lower = tag.to_lowercase();

                // Remove dangerous tags completely
                if tag_lower.starts_with("<script")
                    || tag_lower.starts_with("<iframe")
                    || tag_lower.starts_with("<form")
                    || tag_lower.starts_with("<object")
                    || tag_lower.starts_with("<embed")
                {
                    // Skip until closing tag
                    let close_tag = if tag_lower.starts_with("<script") {
                        "</script>"
                    } else if tag_lower.starts_with("<iframe") {
                        "</iframe>"
                    } else if tag_lower.starts_with("<form") {
                        "</form>"
                    } else if tag_lower.starts_with("<object") {
                        "</object>"
                    } else {
                        "</embed>"
                    };
                    let mut buf = String::new();
                    for tc in chars.by_ref() {
                        buf.push(tc);
                        if buf.to_lowercase().ends_with(close_tag) {
                            break;
                        }
                    }
                    continue;
                }

                // Process img tags
                if tag_lower.starts_with("<img") {
                    let src = extract_attr(&tag, "src");
                    let width = extract_attr(&tag, "width");
                    let height = extract_attr(&tag, "height");

                    if let Some(ref src_val) = src {
                        let domain = extract_domain(src_val);

                        // Check tracking pixel
                        if crate::tracker::is_tracking_pixel(
                            width.as_deref(),
                            height.as_deref(),
                        ) {
                            trackers_blocked.push(TrackerInfo {
                                domain: domain.clone(),
                                tracker_type: "pixel".to_string(),
                            });
                            continue;
                        }

                        // Check known tracker domain
                        let is_custom_tracker = self
                            .custom_tracker_domains
                            .iter()
                            .any(|d| domain.contains(d));

                        if crate::tracker::is_known_tracker(&domain) || is_custom_tracker {
                            trackers_blocked.push(TrackerInfo {
                                domain,
                                tracker_type: "domain".to_string(),
                            });
                            continue;
                        }

                        // External image
                        if src_val.starts_with("http://") || src_val.starts_with("https://") {
                            if !allow_images {
                                images_blocked += 1;
                                result.push_str(&format!(
                                    r#"<div class="blocked-image" data-src="{}">[Image blocked]</div>"#,
                                    html_escape(src_val)
                                ));
                                continue;
                            }
                        }
                    }

                    result.push_str(&tag);
                    continue;
                }

                result.push_str(&tag);
            } else {
                result.push(c);
            }
        }

        RenderedHtml {
            html: result,
            trackers_blocked,
            images_blocked,
        }
    }
}

fn extract_attr(tag: &str, name: &str) -> Option<String> {
    let tag_lower = tag.to_lowercase();
    let search = format!("{}=", name);

    if let Some(pos) = tag_lower.find(&search) {
        let after = &tag[pos + search.len()..];
        let after = after.trim_start();
        if after.starts_with('"') {
            let end = after[1..].find('"')?;
            Some(after[1..1 + end].to_string())
        } else if after.starts_with('\'') {
            let end = after[1..].find('\'')?;
            Some(after[1..1 + end].to_string())
        } else {
            let end = after.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(after.len());
            Some(after[..end].to_string())
        }
    } else {
        None
    }
}

fn extract_domain(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_removes_script_tags() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><script>alert("xss")</script><p>World</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("script"));
        assert!(result.html.contains("Hello"));
        assert!(result.html.contains("World"));
    }

    #[test]
    fn test_blocks_tracking_pixel() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><img src="https://track.example.com/open.gif" width="1" height="1">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("track.example.com"));
        assert_eq!(result.trackers_blocked.len(), 1);
        assert_eq!(result.trackers_blocked[0].tracker_type, "pixel");
    }

    #[test]
    fn test_blocks_known_tracker_domain() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://open.mailchimp.com/track/abc123" width="100" height="50">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("mailchimp"));
        assert_eq!(result.trackers_blocked.len(), 1);
        assert_eq!(result.trackers_blocked[0].tracker_type, "domain");
    }

    #[test]
    fn test_blocks_external_images_in_strict_mode() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://example.com/photo.jpg" width="200" height="150">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(result.html.contains("blocked-image"));
        assert_eq!(result.images_blocked, 1);
    }

    #[test]
    fn test_allows_images_in_load_once_mode() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://example.com/photo.jpg" width="200" height="150">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);
        assert!(result.html.contains("example.com/photo.jpg"));
        assert_eq!(result.images_blocked, 0);
    }

    #[test]
    fn test_still_blocks_trackers_in_load_once_mode() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://open.mailchimp.com/track" width="100" height="50">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);
        assert!(!result.html.contains("mailchimp"));
        assert_eq!(result.trackers_blocked.len(), 1);
    }

    #[test]
    fn test_removes_iframe_tags() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Before</p><iframe src="https://evil.com"></iframe><p>After</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("iframe"));
        assert!(result.html.contains("Before"));
        assert!(result.html.contains("After"));
    }
}
```

- [ ] **Step 5: Run all privacy tests**

Run:
```bash
cargo test -p pebble-privacy
```
Expected: All 9 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/pebble-privacy/
git commit -m "feat(privacy): HTML sanitizer with tracker detection

Strips tracking pixels (1x1), known tracker domains (30+),
and dangerous tags (script/iframe/form/object/embed).
Three privacy modes: Strict, LoadOnce, TrustSender.
External images blocked by default with placeholder."
```

---

### Task 5: Frontend Shell — Layout, Sidebar, TitleBar, Routing

**Files:**
- Create: `src/app/Layout.tsx`
- Create: `src/components/TitleBar.tsx`
- Create: `src/components/Sidebar.tsx`
- Create: `src/components/StatusBar.tsx`
- Create: `src/features/inbox/InboxView.tsx`
- Create: `src/stores/ui.store.ts`
- Create: `src/lib/api.ts`
- Modify: `src/App.tsx`

- [ ] **Step 1: Create src/stores/ui.store.ts**

```typescript
// src/stores/ui.store.ts
import { create } from "zustand";

export type ActiveView = "inbox" | "kanban" | "settings";
export type Theme = "light" | "dark" | "system";

interface UIState {
  sidebarCollapsed: boolean;
  activeView: ActiveView;
  theme: Theme;
  syncStatus: "idle" | "syncing" | "error";
  toggleSidebar: () => void;
  setActiveView: (view: ActiveView) => void;
  setTheme: (theme: Theme) => void;
  setSyncStatus: (status: "idle" | "syncing" | "error") => void;
}

export const useUIStore = create<UIState>((set) => ({
  sidebarCollapsed: false,
  activeView: "inbox",
  theme: "light",
  syncStatus: "idle",
  toggleSidebar: () =>
    set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),
  setActiveView: (view) => set({ activeView: view }),
  setTheme: (theme) => set({ theme }),
  setSyncStatus: (status) => set({ syncStatus: status }),
}));
```

- [ ] **Step 2: Create src/lib/api.ts**

```typescript
// src/lib/api.ts
import { invoke } from "@tauri-apps/api/core";

export async function greet(name: string): Promise<string> {
  return invoke<string>("greet", { name });
}

// Phase 1: minimal API surface. More commands added in later phases.
```

- [ ] **Step 3: Create src/components/TitleBar.tsx**

```tsx
// src/components/TitleBar.tsx
import { getCurrentWindow } from "@tauri-apps/api/window";

export default function TitleBar() {
  const appWindow = getCurrentWindow();

  return (
    <div
      data-tauri-drag-region
      className="flex items-center justify-between h-9 select-none"
      style={{ backgroundColor: "var(--color-titlebar-bg)" }}
    >
      <div data-tauri-drag-region className="flex items-center gap-2 px-3">
        <span className="text-sm font-semibold" style={{ color: "var(--color-text-primary)" }}>
          Pebble
        </span>
      </div>
      <div className="flex items-center">
        <button
          onClick={() => appWindow.minimize()}
          className="h-9 w-11 inline-flex items-center justify-center hover:bg-black/5"
          aria-label="Minimize"
        >
          <svg width="10" height="1" viewBox="0 0 10 1">
            <rect width="10" height="1" fill="currentColor" />
          </svg>
        </button>
        <button
          onClick={async () => {
            const maximized = await appWindow.isMaximized();
            if (maximized) {
              await appWindow.unmaximize();
            } else {
              await appWindow.maximize();
            }
          }}
          className="h-9 w-11 inline-flex items-center justify-center hover:bg-black/5"
          aria-label="Maximize"
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <rect
              width="9"
              height="9"
              x="0.5"
              y="0.5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1"
            />
          </svg>
        </button>
        <button
          onClick={() => appWindow.close()}
          className="h-9 w-11 inline-flex items-center justify-center hover:bg-red-500 hover:text-white"
          aria-label="Close"
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <path d="M1,1 L9,9 M9,1 L1,9" stroke="currentColor" strokeWidth="1.2" />
          </svg>
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Create src/components/Sidebar.tsx**

```tsx
// src/components/Sidebar.tsx
import { Inbox, LayoutGrid, Settings } from "lucide-react";
import { useUIStore, type ActiveView } from "../stores/ui.store";

interface NavItem {
  id: ActiveView;
  label: string;
  icon: React.ReactNode;
}

const NAV_ITEMS: NavItem[] = [
  { id: "inbox", label: "Inbox", icon: <Inbox size={18} /> },
  { id: "kanban", label: "Kanban", icon: <LayoutGrid size={18} /> },
  { id: "settings", label: "Settings", icon: <Settings size={18} /> },
];

export default function Sidebar() {
  const { activeView, setActiveView, sidebarCollapsed } = useUIStore();

  return (
    <aside
      className="flex flex-col border-r h-full"
      style={{
        width: sidebarCollapsed ? "48px" : "200px",
        backgroundColor: "var(--color-sidebar-bg)",
        borderColor: "var(--color-border)",
        transition: "width 150ms ease",
      }}
    >
      <nav className="flex flex-col gap-0.5 p-2 mt-1">
        {NAV_ITEMS.map((item) => (
          <button
            key={item.id}
            onClick={() => setActiveView(item.id)}
            className="flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm"
            style={{
              backgroundColor:
                activeView === item.id
                  ? "var(--color-sidebar-active)"
                  : "transparent",
              color: "var(--color-text-primary)",
            }}
            onMouseEnter={(e) => {
              if (activeView !== item.id) {
                e.currentTarget.style.backgroundColor =
                  "var(--color-sidebar-hover)";
              }
            }}
            onMouseLeave={(e) => {
              if (activeView !== item.id) {
                e.currentTarget.style.backgroundColor = "transparent";
              }
            }}
          >
            {item.icon}
            {!sidebarCollapsed && <span>{item.label}</span>}
          </button>
        ))}
      </nav>
    </aside>
  );
}
```

- [ ] **Step 5: Create src/components/StatusBar.tsx**

```tsx
// src/components/StatusBar.tsx
import { useUIStore } from "../stores/ui.store";

export default function StatusBar() {
  const { syncStatus } = useUIStore();

  const statusText = {
    idle: "Ready",
    syncing: "Syncing...",
    error: "Sync error",
  }[syncStatus];

  return (
    <footer
      className="flex items-center px-3 h-6 text-xs border-t"
      style={{
        backgroundColor: "var(--color-statusbar-bg)",
        borderColor: "var(--color-border)",
        color: "var(--color-text-secondary)",
      }}
    >
      <span>{statusText}</span>
    </footer>
  );
}
```

- [ ] **Step 6: Create src/features/inbox/InboxView.tsx (placeholder)**

```tsx
// src/features/inbox/InboxView.tsx
export default function InboxView() {
  return (
    <div className="flex items-center justify-center h-full">
      <p style={{ color: "var(--color-text-secondary)" }}>
        No emails yet. Configure an account in Settings.
      </p>
    </div>
  );
}
```

- [ ] **Step 7: Create src/app/Layout.tsx**

```tsx
// src/app/Layout.tsx
import TitleBar from "../components/TitleBar";
import Sidebar from "../components/Sidebar";
import StatusBar from "../components/StatusBar";
import InboxView from "../features/inbox/InboxView";
import { useUIStore } from "../stores/ui.store";

export default function Layout() {
  const { activeView } = useUIStore();

  return (
    <div className="flex flex-col h-screen overflow-hidden">
      <TitleBar />
      <div className="flex flex-1 min-h-0">
        <Sidebar />
        <main className="flex-1 min-w-0 overflow-auto">
          {activeView === "inbox" && <InboxView />}
          {activeView === "kanban" && (
            <div className="flex items-center justify-center h-full">
              <p style={{ color: "var(--color-text-secondary)" }}>
                Kanban — coming in Phase 3
              </p>
            </div>
          )}
          {activeView === "settings" && (
            <div className="flex items-center justify-center h-full">
              <p style={{ color: "var(--color-text-secondary)" }}>
                Settings — coming in Phase 4
              </p>
            </div>
          )}
        </main>
      </div>
      <StatusBar />
    </div>
  );
}
```

- [ ] **Step 8: Update src/App.tsx**

```tsx
// src/App.tsx
import Layout from "./app/Layout";

export default function App() {
  return <Layout />;
}
```

- [ ] **Step 9: Run frontend build check**

Run:
```bash
pnpm build:frontend
```
Expected: TypeScript compiles and Vite builds with no errors.

- [ ] **Step 10: Commit**

```bash
git add src/ index.html
git commit -m "feat(ui): frontend shell with titlebar, sidebar, status bar

Custom frameless titlebar with window controls, collapsible sidebar
with navigation (Inbox/Kanban/Settings), status bar showing sync state.
Zustand UI store, Tailwind CSS styling matching Doru-inspired design."
```

---

### Task 6: Tauri Backend Integration — AppState + IPC Wiring

**Files:**
- Create: `src-tauri/src/state.rs`
- Create: `src-tauri/src/commands/mod.rs`
- Create: `src-tauri/src/commands/health.rs`
- Create: `src-tauri/src/events.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Create src-tauri/src/events.rs**

```rust
// src-tauri/src/events.rs
/// Event name constants for Tauri event system
pub const MAIL_SYNC_PROGRESS: &str = "mail:sync-progress";
pub const MAIL_SYNC_COMPLETE: &str = "mail:sync-complete";
pub const MAIL_ERROR: &str = "mail:error";
pub const MAIL_NEW: &str = "mail:new";
pub const MAIL_UNSNOOZED: &str = "mail:unsnoozed";
```

- [ ] **Step 2: Create src-tauri/src/state.rs**

```rust
// src-tauri/src/state.rs
use pebble_store::Store;
use std::sync::Arc;

pub struct AppState {
    pub store: Arc<Store>,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(store),
        }
    }
}
```

- [ ] **Step 3: Create src-tauri/src/commands/health.rs**

```rust
// src-tauri/src/commands/health.rs
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn health_check(state: State<'_, AppState>) -> Result<String, String> {
    // Verify store is accessible by listing accounts
    match state.store.list_accounts() {
        Ok(accounts) => Ok(format!(
            "Pebble is healthy. {} account(s) configured.",
            accounts.len()
        )),
        Err(e) => Err(format!("Health check failed: {}", e)),
    }
}
```

- [ ] **Step 4: Create src-tauri/src/commands/mod.rs**

```rust
// src-tauri/src/commands/mod.rs
pub mod health;

pub use health::health_check;
```

- [ ] **Step 5: Update src-tauri/src/lib.rs to wire everything together**

```rust
// src-tauri/src/lib.rs
mod commands;
mod events;
mod state;

use state::AppState;
use std::path::PathBuf;

fn get_db_path(app: &tauri::App) -> PathBuf {
    let app_data = app
        .path()
        .app_data_dir()
        .expect("Failed to get app data directory");
    std::fs::create_dir_all(&app_data).expect("Failed to create app data directory");
    let db_dir = app_data.join("db");
    std::fs::create_dir_all(&db_dir).expect("Failed to create db directory");
    db_dir.join("pebble.db")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pebble=debug,pebble_store=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let db_path = get_db_path(app);
            tracing::info!("Database path: {}", db_path.display());

            let store = pebble_store::Store::open(&db_path)
                .expect("Failed to open database");
            tracing::info!("Database initialized successfully");

            app.manage(AppState::new(store));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::health_check,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 6: Update src/lib/api.ts with health check**

```typescript
// src/lib/api.ts
import { invoke } from "@tauri-apps/api/core";

export async function healthCheck(): Promise<string> {
  return invoke<string>("health_check");
}
```

- [ ] **Step 7: Update InboxView to call health check on mount**

```tsx
// src/features/inbox/InboxView.tsx
import { useEffect, useState } from "react";
import { healthCheck } from "../../lib/api";

export default function InboxView() {
  const [status, setStatus] = useState<string>("Connecting...");

  useEffect(() => {
    healthCheck()
      .then(setStatus)
      .catch((err) => setStatus(`Error: ${err}`));
  }, []);

  return (
    <div className="flex items-center justify-center h-full">
      <p style={{ color: "var(--color-text-secondary)" }}>{status}</p>
    </div>
  );
}
```

- [ ] **Step 8: Verify full stack compiles**

Run:
```bash
cargo check -p pebble
```
Expected: Compiles with no errors.

Run:
```bash
pnpm build:frontend
```
Expected: Vite builds with no errors.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/ src/
git commit -m "feat: wire Tauri backend with AppState and health check IPC

AppState holds Store, initialized at app startup with SQLite DB
in app data directory. Health check command verifies store accessibility.
Frontend calls health_check on mount to confirm backend connection."
```

---

### Task 7: Frontend Tests — UI Store

**Files:**
- Create: `vitest.config.ts`
- Create: `tests/stores/ui.store.test.ts`

- [ ] **Step 1: Create vitest.config.ts**

```typescript
// vitest.config.ts
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: [],
  },
  resolve: {
    alias: {
      "@": "/src",
    },
  },
});
```

- [ ] **Step 2: Write UI store tests**

```typescript
// tests/stores/ui.store.test.ts
import { describe, it, expect, beforeEach } from "vitest";
import { useUIStore } from "../../src/stores/ui.store";

describe("UIStore", () => {
  beforeEach(() => {
    // Reset store to initial state
    useUIStore.setState({
      sidebarCollapsed: false,
      activeView: "inbox",
      theme: "light",
      syncStatus: "idle",
    });
  });

  it("should have correct initial state", () => {
    const state = useUIStore.getState();
    expect(state.sidebarCollapsed).toBe(false);
    expect(state.activeView).toBe("inbox");
    expect(state.theme).toBe("light");
    expect(state.syncStatus).toBe("idle");
  });

  it("should toggle sidebar", () => {
    useUIStore.getState().toggleSidebar();
    expect(useUIStore.getState().sidebarCollapsed).toBe(true);

    useUIStore.getState().toggleSidebar();
    expect(useUIStore.getState().sidebarCollapsed).toBe(false);
  });

  it("should set active view", () => {
    useUIStore.getState().setActiveView("kanban");
    expect(useUIStore.getState().activeView).toBe("kanban");

    useUIStore.getState().setActiveView("settings");
    expect(useUIStore.getState().activeView).toBe("settings");
  });

  it("should set theme", () => {
    useUIStore.getState().setTheme("dark");
    expect(useUIStore.getState().theme).toBe("dark");
  });

  it("should set sync status", () => {
    useUIStore.getState().setSyncStatus("syncing");
    expect(useUIStore.getState().syncStatus).toBe("syncing");

    useUIStore.getState().setSyncStatus("error");
    expect(useUIStore.getState().syncStatus).toBe("error");
  });
});
```

- [ ] **Step 3: Run tests**

Run:
```bash
pnpm test
```
Expected: 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add vitest.config.ts tests/
git commit -m "test(ui): add UI store unit tests

Tests for initial state, sidebar toggle, view switching,
theme setting, and sync status updates."
```

---

### Task 8: Run Full Cargo Test Suite

- [ ] **Step 1: Run all Rust tests**

Run:
```bash
cargo test --workspace
```
Expected: All tests pass across pebble-core, pebble-store, and pebble-privacy.

- [ ] **Step 2: Run clippy**

Run:
```bash
cargo clippy --workspace -- -D warnings
```
Expected: No warnings (fix any that appear).

- [ ] **Step 3: Run all frontend tests**

Run:
```bash
pnpm test
```
Expected: All tests pass.

- [ ] **Step 4: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "chore: fix clippy warnings and test issues"
```

(Skip this commit if no fixes were needed.)

---

## Summary

After completing Phase 1, you will have:

- **Tauri v2 + React + TypeScript** project with Rust workspace
- **3 Rust crates**: pebble-core (types/traits/errors), pebble-store (SQLite with full schema), pebble-privacy (HTML sanitizer)
- **Frontend shell**: custom titlebar, sidebar navigation, status bar, Zustand state management
- **IPC bridge**: health check confirming frontend-backend connectivity
- **Tests**: Rust unit tests for store CRUD + privacy sanitization, frontend tests for UI store
- **Working app** that launches, shows the shell layout, and confirms database connection

**Next:** Phase 2 adds the IMAP mail engine, inbox UI with message list/detail, and Tantivy search.
