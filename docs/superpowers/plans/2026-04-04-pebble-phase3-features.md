# Pebble Phase 3: Command Palette + Kanban + Snooze + Rules + Compose

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the five core productivity features that transform Pebble from a mail viewer into a productivity tool: command palette (Ctrl+K), Kanban board (drag cards), Snooze (timed hide/return), rule engine (auto-classify), and email compose (reply/forward/new).

**Architecture:** New Rust crate `pebble-rules`. New frontend features: command-palette, kanban, compose. New store CRUD for kanban_cards, snoozed_messages, rules. Snooze watcher runs as a background tokio task. dnd-kit for drag-and-drop, TipTap for rich text compose.

**Tech Stack:** dnd-kit (drag), @tiptap/react + @tiptap/starter-kit (compose), lettre 0.11 (SMTP send)

**Spec:** `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md`

**Depends on:** Phase 2 complete (mail engine, inbox UI, search, all IPC commands)

---

## File Structure (Phase 3 additions)

```
pebble/
├── crates/
│   ├── pebble-rules/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs              # RuleEngine: evaluate messages against rules
│   │       ├── types.rs            # Rule, RuleCondition, RuleAction, ConditionOp
│   │       └── matcher.rs          # Condition matching logic
│   │
│   ├── pebble-store/src/
│   │   ├── kanban.rs              # Kanban CRUD: upsert_card, list_cards, move_card, delete_card
│   │   ├── snooze.rs              # Snooze CRUD: snooze_message, list_snoozed, check_due, unsnooze
│   │   ├── rules.rs               # Rules CRUD: insert_rule, list_rules, update_rule, delete_rule
│   │   └── trusted_senders.rs     # trust_sender, is_trusted, list_trusted
│   │
│   └── pebble-mail/src/
│       └── smtp.rs                # SmtpSender: send email via lettre
│
├── src-tauri/src/
│   ├── commands/
│   │   ├── kanban.rs              # move_to_kanban, list_kanban, remove_from_kanban
│   │   ├── snooze.rs              # snooze_message, unsnooze_message, list_snoozed
│   │   ├── rules.rs               # create_rule, list_rules, update_rule, delete_rule
│   │   ├── compose.rs             # send_email
│   │   └── trusted_senders.rs     # trust_sender
│   └── snooze_watcher.rs          # Background task: check due snoozed messages every 30s
│
├── src/
│   ├── stores/
│   │   ├── command.store.ts       # CommandStore: open/close, query, commands, execute
│   │   └── kanban.store.ts        # KanbanStore: cards by column, optimistic updates
│   ├── features/
│   │   ├── command-palette/
│   │   │   ├── CommandPalette.tsx  # Modal overlay with fuzzy search
│   │   │   └── commands.ts        # Command registry (all available commands)
│   │   ├── kanban/
│   │   │   ├── KanbanView.tsx     # Three-column board with dnd-kit
│   │   │   ├── KanbanColumn.tsx   # Single column with droppable area
│   │   │   └── KanbanCard.tsx     # Draggable card showing message summary
│   │   ├── compose/
│   │   │   └── ComposeView.tsx    # Email compose with TipTap editor
│   │   └── inbox/
│   │       └── SnoozePopover.tsx  # Time picker for snooze
│   └── hooks/
│       └── useKeyboard.ts         # Global keyboard shortcut handler
```

---

### Task 1: Store CRUD — Kanban, Snooze, Rules, Trusted Senders

**Files:**
- Create: `crates/pebble-store/src/kanban.rs`
- Create: `crates/pebble-store/src/snooze.rs`
- Create: `crates/pebble-store/src/rules.rs`
- Create: `crates/pebble-store/src/trusted_senders.rs`
- Modify: `crates/pebble-store/src/lib.rs` (add modules)

**Context:** The database tables already exist (kanban_cards, snoozed_messages, rules, trusted_senders). We need CRUD operations for each. All methods follow the existing pattern: `self.with_conn(|conn| { ... })` returning `pebble_core::Result<T>`.

- [ ] **Step 1: Create kanban.rs**

```rust
use pebble_core::{KanbanCard, KanbanColumn, PebbleError, Result};
use rusqlite::params;
use crate::Store;

impl Store {
    /// Add or update a message on the kanban board
    pub fn upsert_kanban_card(&self, card: &KanbanCard) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO kanban_cards (message_id, column_name, position, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(message_id) DO UPDATE SET
                     column_name = excluded.column_name,
                     position = excluded.position,
                     updated_at = excluded.updated_at",
                params![
                    card.message_id,
                    column_to_str(&card.column),
                    card.position,
                    card.created_at,
                    card.updated_at,
                ],
            ).map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }

    /// List all kanban cards, optionally filtered by column
    pub fn list_kanban_cards(&self, column: Option<&KanbanColumn>) -> Result<Vec<KanbanCard>> {
        self.with_conn(|conn| {
            let (sql, param_val) = match column {
                Some(col) => (
                    "SELECT message_id, column_name, position, created_at, updated_at
                     FROM kanban_cards WHERE column_name = ?1 ORDER BY position ASC",
                    Some(column_to_str(col)),
                ),
                None => (
                    "SELECT message_id, column_name, position, created_at, updated_at
                     FROM kanban_cards ORDER BY column_name, position ASC",
                    None,
                ),
            };
            let mut stmt = conn.prepare(sql).map_err(|e| PebbleError::Storage(e.to_string()))?;
            let rows = if let Some(ref val) = param_val {
                stmt.query_map(params![val], row_to_kanban_card)
            } else {
                stmt.query_map([], row_to_kanban_card)
            }.map_err(|e| PebbleError::Storage(e.to_string()))?;
            let mut cards = Vec::new();
            for row in rows {
                cards.push(row.map_err(|e| PebbleError::Storage(e.to_string()))?);
            }
            Ok(cards)
        })
    }

    /// Move a card to a different column and position
    pub fn move_kanban_card(&self, message_id: &str, column: &KanbanColumn, position: i32) -> Result<()> {
        self.with_conn(|conn| {
            let now = pebble_core::now_timestamp();
            conn.execute(
                "UPDATE kanban_cards SET column_name = ?1, position = ?2, updated_at = ?3 WHERE message_id = ?4",
                params![column_to_str(column), position, now, message_id],
            ).map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }

    /// Remove a card from the kanban board
    pub fn delete_kanban_card(&self, message_id: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM kanban_cards WHERE message_id = ?1", params![message_id])
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }
}

fn column_to_str(col: &KanbanColumn) -> &'static str {
    match col { KanbanColumn::Todo => "todo", KanbanColumn::Waiting => "waiting", KanbanColumn::Done => "done" }
}

fn str_to_column(s: &str) -> KanbanColumn {
    match s { "waiting" => KanbanColumn::Waiting, "done" => KanbanColumn::Done, _ => KanbanColumn::Todo }
}

fn row_to_kanban_card(row: &rusqlite::Row) -> rusqlite::Result<KanbanCard> {
    let col_str: String = row.get(1)?;
    Ok(KanbanCard {
        message_id: row.get(0)?,
        column: str_to_column(&col_str),
        position: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}
```

Tests: `test_kanban_upsert_and_list`, `test_kanban_move`, `test_kanban_delete`

- [ ] **Step 2: Create snooze.rs**

```rust
impl Store {
    pub fn snooze_message(&self, snooze: &SnoozedMessage) -> Result<()> { ... }
    pub fn list_snoozed_messages(&self) -> Result<Vec<SnoozedMessage>> { ... }
    pub fn get_due_snoozed(&self, now: i64) -> Result<Vec<SnoozedMessage>> {
        // WHERE unsnoozed_at <= now
    }
    pub fn unsnooze_message(&self, message_id: &str) -> Result<()> {
        // DELETE FROM snoozed_messages WHERE message_id = ?
    }
}
```

Tests: `test_snooze_and_list`, `test_due_snoozed`, `test_unsnooze`

- [ ] **Step 3: Create rules.rs**

Need a `Rule` type in pebble-core. Add to `types.rs`:
```rust
pub struct Rule {
    pub id: String,
    pub name: String,
    pub priority: i32,
    pub conditions: String, // JSON
    pub actions: String,    // JSON
    pub is_enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}
```

Store methods:
```rust
impl Store {
    pub fn insert_rule(&self, rule: &Rule) -> Result<()> { ... }
    pub fn list_rules(&self) -> Result<Vec<Rule>> { ... }
    pub fn update_rule(&self, rule: &Rule) -> Result<()> { ... }
    pub fn delete_rule(&self, id: &str) -> Result<()> { ... }
}
```

Tests: `test_rule_crud`

- [ ] **Step 4: Create trusted_senders.rs**

```rust
impl Store {
    pub fn trust_sender(&self, sender: &TrustedSender) -> Result<()> {
        // INSERT OR REPLACE
    }
    pub fn is_trusted_sender(&self, account_id: &str, email: &str) -> Result<Option<TrustType>> { ... }
    pub fn list_trusted_senders(&self, account_id: &str) -> Result<Vec<TrustedSender>> { ... }
    pub fn remove_trusted_sender(&self, account_id: &str, email: &str) -> Result<()> { ... }
}
```

Tests: `test_trust_sender_crud`

- [ ] **Step 5: Add modules to lib.rs**

Add `pub mod kanban; pub mod snooze; pub mod rules; pub mod trusted_senders;` to `crates/pebble-store/src/lib.rs`.

- [ ] **Step 6: Add Rule type to pebble-core**

Add the `Rule` struct to `crates/pebble-core/src/types.rs` and re-export from `lib.rs`.

- [ ] **Step 7: Run tests**

Run: `cargo test -p pebble-store`
Expected: All existing + new tests pass.

- [ ] **Step 8: Commit**

```bash
git commit -m "feat(store): add CRUD for kanban cards, snoozed messages, rules, and trusted senders"
```

---

### Task 2: pebble-rules crate — Rule engine

**Files:**
- Create: `crates/pebble-rules/Cargo.toml`
- Create: `crates/pebble-rules/src/lib.rs`
- Create: `crates/pebble-rules/src/types.rs`
- Create: `crates/pebble-rules/src/matcher.rs`
- Modify: `Cargo.toml` (workspace members)

**Context:** The rule engine evaluates incoming messages against user-defined rules. Each rule has JSON conditions and JSON actions. The engine returns a list of actions to apply. No regex for MVP — use string operations (contains, equals, starts_with, ends_with).

- [ ] **Step 1: Create crate**

`crates/pebble-rules/Cargo.toml`:
```toml
[package]
name = "pebble-rules"
version = "0.1.0"
edition = "2021"

[dependencies]
pebble-core = { path = "../pebble-core" }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
```

Add to workspace members in root `Cargo.toml`.

- [ ] **Step 2: Create types.rs**

```rust
use serde::{Deserialize, Serialize};
use pebble_core::KanbanColumn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConditionSet {
    pub operator: LogicalOp,
    pub conditions: Vec<RuleCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogicalOp { And, Or }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleCondition {
    pub field: ConditionField,
    pub op: ConditionOp,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionField { From, To, Subject, Body, HasAttachment, Domain }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOp { Contains, NotContains, Equals, StartsWith, EndsWith }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum RuleAction {
    AddLabel(String),
    MoveToFolder(String),
    MarkRead,
    Archive,
    SetKanbanColumn(KanbanColumn),
}
```

- [ ] **Step 3: Create matcher.rs**

```rust
use pebble_core::Message;
use crate::types::*;

pub fn evaluate_condition(msg: &Message, condition: &RuleCondition) -> bool {
    let field_value = match condition.field {
        ConditionField::From => &msg.from_address,
        ConditionField::To => {
            // Check all to addresses
            let joined = msg.to_list.iter().map(|a| a.address.as_str()).collect::<Vec<_>>().join(" ");
            return match_op(&joined, &condition.op, &condition.value);
        }
        ConditionField::Subject => &msg.subject,
        ConditionField::Body => &msg.body_text,
        ConditionField::HasAttachment => {
            let has = msg.has_attachments.to_string();
            return match_op(&has, &condition.op, &condition.value);
        }
        ConditionField::Domain => {
            let domain = msg.from_address.split('@').nth(1).unwrap_or("");
            return match_op(domain, &condition.op, &condition.value);
        }
    };
    match_op(field_value, &condition.op, &condition.value)
}

fn match_op(field_value: &str, op: &ConditionOp, value: &str) -> bool {
    let fv = field_value.to_lowercase();
    let v = value.to_lowercase();
    match op {
        ConditionOp::Contains => fv.contains(&v),
        ConditionOp::NotContains => !fv.contains(&v),
        ConditionOp::Equals => fv == v,
        ConditionOp::StartsWith => fv.starts_with(&v),
        ConditionOp::EndsWith => fv.ends_with(&v),
    }
}

pub fn evaluate_conditions(msg: &Message, conditions: &RuleConditionSet) -> bool {
    match conditions.operator {
        LogicalOp::And => conditions.conditions.iter().all(|c| evaluate_condition(msg, c)),
        LogicalOp::Or => conditions.conditions.iter().any(|c| evaluate_condition(msg, c)),
    }
}
```

Tests: `test_contains_match`, `test_not_contains`, `test_domain_match`, `test_and_conditions`, `test_or_conditions`

- [ ] **Step 4: Create lib.rs**

```rust
pub mod types;
pub mod matcher;

use pebble_core::{Message, Rule};
use types::{RuleAction, RuleConditionSet};
use matcher::evaluate_conditions;

pub struct RuleEngine {
    rules: Vec<(RuleConditionSet, Vec<RuleAction>)>,
}

impl RuleEngine {
    /// Load rules from parsed Rule structs (sorted by priority)
    pub fn new(rules: &[Rule]) -> Self {
        let mut parsed: Vec<(i32, RuleConditionSet, Vec<RuleAction>)> = rules.iter()
            .filter(|r| r.is_enabled)
            .filter_map(|r| {
                let conditions: RuleConditionSet = serde_json::from_str(&r.conditions).ok()?;
                let actions: Vec<RuleAction> = serde_json::from_str(&r.actions).ok()?;
                Some((r.priority, conditions, actions))
            })
            .collect();
        parsed.sort_by_key(|(p, _, _)| *p);
        Self {
            rules: parsed.into_iter().map(|(_, c, a)| (c, a)).collect(),
        }
    }

    /// Evaluate a message against all rules, return all matching actions
    pub fn evaluate(&self, message: &Message) -> Vec<RuleAction> {
        let mut actions = Vec::new();
        for (conditions, rule_actions) in &self.rules {
            if evaluate_conditions(message, conditions) {
                actions.extend(rule_actions.iter().cloned());
            }
        }
        actions
    }
}
```

Tests: `test_rule_engine_evaluate`, `test_disabled_rules_skipped`

- [ ] **Step 5: Run tests and commit**

Run: `cargo test -p pebble-rules`
Commit: `feat(rules): add pebble-rules crate with condition matching and rule engine`

---

### Task 3: SMTP send + Snooze watcher + IPC commands

**Files:**
- Create: `crates/pebble-mail/src/smtp.rs`
- Create: `src-tauri/src/snooze_watcher.rs`
- Create: `src-tauri/src/commands/kanban.rs`
- Create: `src-tauri/src/commands/snooze.rs`
- Create: `src-tauri/src/commands/rules.rs`
- Create: `src-tauri/src/commands/compose.rs`
- Create: `src-tauri/src/commands/trusted_senders.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs` (register commands, start snooze watcher)
- Modify: `src-tauri/src/state.rs` (add SmtpSender handle)
- Modify: `src-tauri/Cargo.toml` (add pebble-rules, lettre)
- Modify: `crates/pebble-mail/Cargo.toml` (add lettre)

**Context:** This task wires up all the new backend features to the frontend via Tauri IPC.

- [ ] **Step 1: Create smtp.rs in pebble-mail**

```rust
use pebble_core::{PebbleError, Result};
use lettre::{Message as LettreMessage, SmtpTransport, Transport};
use lettre::transport::smtp::authentication::Credentials;

pub struct SmtpSender {
    host: String,
    port: u16,
    credentials: Credentials,
    use_tls: bool,
}

impl SmtpSender {
    pub fn new(host: String, port: u16, username: String, password: String, use_tls: bool) -> Self { ... }
    pub fn send(&self, from: &str, to: &[String], cc: &[String], subject: &str, body_text: &str, body_html: Option<&str>, in_reply_to: Option<&str>) -> Result<()> { ... }
}
```

Add `lettre` to pebble-mail/Cargo.toml dependencies. Add lettre workspace dep to root Cargo.toml:
```toml
lettre = { version = "0.11", default-features = false, features = ["tokio1-rustls-tls", "smtp-transport", "builder"] }
```

- [ ] **Step 2: Create snooze_watcher.rs**

```rust
/// Background task that checks for due snoozed messages every 30 seconds.
/// When a message is due, it removes the snooze record.
/// Emits a Tauri event "mail:unsnoozed" so the frontend can refresh.
pub async fn run_snooze_watcher(store: Arc<Store>, app_handle: tauri::AppHandle, stop_rx: watch::Receiver<bool>) {
    let interval = tokio::time::Duration::from_secs(30);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                let now = pebble_core::now_timestamp();
                if let Ok(due) = store.get_due_snoozed(now) {
                    for snoozed in due {
                        let _ = store.unsnooze_message(&snoozed.message_id);
                        let _ = app_handle.emit("mail:unsnoozed", &snoozed.message_id);
                    }
                }
            }
            _ = stop_rx.changed() => { if *stop_rx.borrow() { break; } }
        }
    }
}
```

- [ ] **Step 3: Create kanban IPC commands**

```rust
#[tauri::command]
pub async fn move_to_kanban(state, message_id, column: KanbanColumn) -> Result<()>
#[tauri::command]
pub async fn list_kanban_cards(state, column: Option<KanbanColumn>) -> Result<Vec<KanbanCard>>
#[tauri::command]
pub async fn remove_from_kanban(state, message_id) -> Result<()>
```

- [ ] **Step 4: Create snooze IPC commands**

```rust
#[tauri::command]
pub async fn snooze_message(state, message_id, until: i64, return_to: String) -> Result<()>
#[tauri::command]
pub async fn unsnooze_message(state, message_id) -> Result<()>
#[tauri::command]
pub async fn list_snoozed(state) -> Result<Vec<SnoozedMessage>>
```

- [ ] **Step 5: Create rules IPC commands**

```rust
#[tauri::command]
pub async fn create_rule(state, name, priority, conditions: String, actions: String) -> Result<Rule>
#[tauri::command]
pub async fn list_rules(state) -> Result<Vec<Rule>>
#[tauri::command]
pub async fn update_rule(state, rule: Rule) -> Result<()>
#[tauri::command]
pub async fn delete_rule(state, rule_id) -> Result<()>
```

- [ ] **Step 6: Create compose IPC command**

```rust
#[tauri::command]
pub async fn send_email(state, account_id, to: Vec<String>, cc: Vec<String>, subject, body_text, body_html: Option<String>, in_reply_to: Option<String>) -> Result<()>
```

This needs the account's SMTP config. Load it from sync_state (same JSON that has IMAP config — extend to include SMTP fields).

- [ ] **Step 7: Create trusted_senders IPC command**

```rust
#[tauri::command]
pub async fn trust_sender(state, account_id, email, trust_type: String) -> Result<()>
```

- [ ] **Step 8: Register all commands and start snooze watcher in lib.rs**

Add all new commands to `generate_handler![]`. Start snooze watcher in `setup()`.

- [ ] **Step 9: Run clippy + tests, commit**

Run: `cargo clippy --workspace -- -D warnings && cargo test -p pebble-store -p pebble-rules -p pebble-mail`
Commit: `feat(ipc): add commands for kanban, snooze, rules, compose, and trusted senders`

---

### Task 4: Command palette — frontend

**Files:**
- Create: `src/stores/command.store.ts`
- Create: `src/features/command-palette/commands.ts`
- Create: `src/features/command-palette/CommandPalette.tsx`
- Create: `src/hooks/useKeyboard.ts`
- Modify: `src/app/Layout.tsx` (mount command palette + keyboard handler)
- Modify: `src/lib/api.ts` (add new IPC wrappers)

**Context:** Command palette is the central productivity feature. Ctrl+K opens it, fuzzy-matching commands. Commands can navigate views, operate on selected messages, etc.

- [ ] **Step 1: Add new API wrappers to api.ts**

Add TypeScript types and functions for all new commands:
```typescript
// Kanban
moveToKanban(message_id, column): Promise<void>
listKanbanCards(column?: string): Promise<KanbanCard[]>
removeFromKanban(message_id): Promise<void>

// Snooze
snoozeMessage(message_id, until, return_to): Promise<void>
unsnoozeMessage(message_id): Promise<void>
listSnoozed(): Promise<SnoozedMessage[]>

// Rules
createRule(name, priority, conditions, actions): Promise<Rule>
listRules(): Promise<Rule[]>
updateRule(rule): Promise<void>
deleteRule(rule_id): Promise<void>

// Compose
sendEmail(account_id, to, cc, subject, body_text, body_html?, in_reply_to?): Promise<void>

// Trusted senders
trustSender(account_id, email, trust_type): Promise<void>
```

- [ ] **Step 2: Create command.store.ts**

```typescript
interface Command {
  id: string;
  name: string;
  shortcut?: string;
  category: string;
  execute: () => void | Promise<void>;
}

interface CommandState {
  isOpen: boolean;
  query: string;
  commands: Command[];
  filteredCommands: Command[];
  open: () => void;
  close: () => void;
  setQuery: (q: string) => void;
  execute: (commandId: string) => Promise<void>;
  registerCommands: (cmds: Command[]) => void;
}
```

Filtering: case-insensitive substring match on name.

- [ ] **Step 3: Create commands.ts registry**

Register all available commands:
- Navigation: Go to Inbox, Go to Kanban, Go to Settings
- Mail actions: Archive, Mark Read/Unread, Star/Unstar, Delete
- Kanban: Move to Todo, Move to Waiting, Move to Done
- Snooze: Snooze 1h, Snooze Tonight, Snooze Tomorrow, Snooze Next Week
- Compose: New Email, Reply, Forward
- View: Toggle Sidebar

Each command gets an `execute` function that calls the appropriate store action or API.

- [ ] **Step 4: Create CommandPalette.tsx**

Modal overlay:
- Dark backdrop
- White card with search input at top
- Filtered command list below
- Arrow key navigation + Enter to execute
- Escape to close
- Shows shortcut hints on right side of each command

- [ ] **Step 5: Create useKeyboard.ts**

Global keyboard handler registered in Layout:
```typescript
const DEFAULT_KEYBINDINGS = {
  "mod+k": "command-palette:open",
  "mod+n": "compose:new",
  "e": "mail:archive",
  "s": "mail:star",
  "j": "mail:next",
  "k": "mail:previous",
  "mod+shift+k": "view:kanban",
  "mod+shift+i": "view:inbox",
  "/": "search:focus",
  "h": "snooze:open",
  "escape": "modal:close",
};
```

- [ ] **Step 6: Mount in Layout.tsx**

Add `<CommandPalette />` overlay and `useKeyboard()` hook to Layout.

- [ ] **Step 7: Run tests and commit**

Run: `npx vitest run`
Commit: `feat(ui): add command palette with keyboard shortcuts`

---

### Task 5: Kanban view — frontend

**Files:**
- Create: `src/stores/kanban.store.ts`
- Create: `src/features/kanban/KanbanView.tsx`
- Create: `src/features/kanban/KanbanColumn.tsx`
- Create: `src/features/kanban/KanbanCard.tsx`
- Modify: `src/app/Layout.tsx` (render KanbanView)

**Context:** Three-column kanban board (Todo / Waiting / Done). Cards show message subject, from, date. Drag-and-drop with dnd-kit. Optimistic updates with rollback on error.

- [ ] **Step 1: Install dnd-kit**

```bash
pnpm add @dnd-kit/core @dnd-kit/sortable @dnd-kit/utilities
```

- [ ] **Step 2: Create kanban.store.ts**

```typescript
interface KanbanState {
  cards: KanbanCard[];
  loading: boolean;
  fetchCards: () => Promise<void>;
  moveCard: (messageId: string, column: string, position: number) => Promise<void>;
  removeCard: (messageId: string) => Promise<void>;
}
```

`moveCard` does optimistic update: update local state immediately, call API, rollback on error.

- [ ] **Step 3: Create KanbanCard.tsx**

Draggable card component showing: subject (truncated), from name, date. Uses `useSortable` from dnd-kit.

- [ ] **Step 4: Create KanbanColumn.tsx**

Single column with header (title + count), droppable area containing sorted cards. Uses `useDroppable`.

- [ ] **Step 5: Create KanbanView.tsx**

Three columns side-by-side using `DndContext` + `SortableContext`. Handles `onDragEnd` to move cards between columns.

- [ ] **Step 6: Update Layout.tsx**

Replace kanban placeholder with `<KanbanView />`.

- [ ] **Step 7: Run tests and commit**

Commit: `feat(ui): add kanban board with drag-and-drop cards`

---

### Task 6: Snooze popover — frontend

**Files:**
- Create: `src/features/inbox/SnoozePopover.tsx`
- Modify: `src/components/MessageDetail.tsx` (add snooze button)
- Modify: `src/lib/api.ts` (ensure snooze APIs exist)

**Context:** Snooze lets users temporarily hide a message and have it reappear at a set time. The popover shows preset times (1h, Tonight 8pm, Tomorrow 9am, Next Monday 9am, custom) and a return_to selector.

- [ ] **Step 1: Create SnoozePopover.tsx**

```tsx
interface Props {
  messageId: string;
  onClose: () => void;
  onSnoozed: () => void;
}
```

Preset options:
- "1 hour" → now + 3600
- "Tonight (8 PM)" → today 20:00
- "Tomorrow (9 AM)" → tomorrow 09:00
- "Next Monday (9 AM)" → next Monday 09:00

Each option calls `snoozeMessage(messageId, timestamp, "inbox")` then `onSnoozed()`.

- [ ] **Step 2: Add snooze button to MessageDetail**

Add a clock icon button in the message detail header that opens the SnoozePopover.

- [ ] **Step 3: Run tests and commit**

Commit: `feat(ui): add snooze popover with preset time options`

---

### Task 7: Compose view — frontend

**Files:**
- Create: `src/features/compose/ComposeView.tsx`
- Modify: `src/app/Layout.tsx` (add compose modal/view)
- Modify: `src/stores/ui.store.ts` (add compose state)

**Context:** Email compose with TipTap rich text editor. Supports new message, reply, and forward. Simple for MVP — no attachment support yet.

- [ ] **Step 1: Install TipTap**

```bash
pnpm add @tiptap/react @tiptap/starter-kit @tiptap/extension-placeholder
```

- [ ] **Step 2: Add compose state to UIStore**

Add to UIStore:
```typescript
composeOpen: boolean;
composeMode: "new" | "reply" | "forward" | null;
composeReplyTo: Message | null;
openCompose: (mode, replyTo?) => void;
closeCompose: () => void;
```

- [ ] **Step 3: Create ComposeView.tsx**

Modal overlay with:
- To field (text input, comma-separated)
- CC field (collapsible)
- Subject field (pre-filled for reply/forward)
- TipTap editor for body
- Send button (calls sendEmail API)
- Cancel button

For reply: pre-fill to with original sender, subject with "Re: ...", quote original in body.
For forward: subject with "Fwd: ...", include original body.

- [ ] **Step 4: Mount in Layout**

Render `<ComposeView />` when composeOpen is true.

- [ ] **Step 5: Run tests and commit**

Commit: `feat(ui): add email compose with TipTap rich text editor`

---

### Task 8: Integration tests + full verification

**Files:**
- Create: `tests/stores/kanban.store.test.ts`
- Create: `tests/stores/command.store.test.ts`

- [ ] **Step 1: Write kanban store tests**

Test initial state, fetchCards mock, moveCard optimistic update.

- [ ] **Step 2: Write command store tests**

Test open/close, query filtering, command execution.

- [ ] **Step 3: Run full test suite**

```bash
npx vitest run
cargo test -p pebble-core -p pebble-store -p pebble-search -p pebble-privacy -p pebble-mail -p pebble-rules
cargo clippy --workspace -- -D warnings
```

- [ ] **Step 4: Commit**

```bash
git commit -m "test: add kanban and command store unit tests"
```

---

## Summary

| Task | Description | Complexity |
|------|-------------|------------|
| 1 | Store CRUD for kanban, snooze, rules, trusted senders | Medium |
| 2 | pebble-rules crate: condition matching + rule engine | Medium |
| 3 | SMTP send + snooze watcher + all new IPC commands | High |
| 4 | Command palette with keyboard shortcuts | High |
| 5 | Kanban board with drag-and-drop | High |
| 6 | Snooze popover | Low |
| 7 | Compose view with TipTap | Medium |
| 8 | Integration tests | Low |

**Total:** 8 tasks, ~55 steps
