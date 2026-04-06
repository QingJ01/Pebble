# Pebble Phase 2: Mail Engine (IMAP) + Inbox UI + Search

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect Pebble to real IMAP/SMTP email accounts — fetch folders, sync messages, parse MIME, render emails safely, full-text search, and display everything in a production-quality Inbox UI.

**Architecture:** New Rust crates `pebble-mail` (IMAP/SMTP + MIME + sync) and `pebble-search` (Tantivy). Frontend gets real Inbox list, message detail with privacy banner, folder navigation, and search. All wired through Tauri IPC commands.

**Tech Stack:** async-imap 0.10, lettre 0.11, mail-parser 0.9, tantivy 0.22, tokio, ammonia 4 (replacing hand-rolled sanitizer), @tanstack/react-virtual, @tanstack/react-query + Tauri invoke

**Spec:** `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md`

**Depends on:** Phase 1 complete (scaffold, core types, store CRUD, privacy guard, frontend shell)

---

## File Structure (Phase 2 additions)

```
pebble/
├── Cargo.toml                          # Add pebble-mail, pebble-search to workspace
├── crates/
│   ├── pebble-mail/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                  # Public API: create_provider, ImapProvider
│   │       ├── imap.rs                 # ImapProvider: MailTransport + FolderProvider impl
│   │       ├── smtp.rs                 # SmtpSender: send_message via lettre
│   │       ├── parser.rs              # MIME parsing: raw bytes → Message + Attachment metadata
│   │       ├── sync.rs                 # SyncWorker: per-account sync loop (poll + reconcile)
│   │       └── thread.rs              # Thread aggregation: compute thread_id from headers
│   │
│   ├── pebble-search/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                  # TantivySearch: SearchEngine impl
│   │       └── schema.rs              # Tantivy index schema definition
│   │
│   ├── pebble-privacy/
│   │   └── src/
│   │       └── sanitizer.rs           # REPLACE hand-rolled parser with ammonia
│   │
│   └── pebble-core/
│       └── src/
│           └── types.rs               # Add AccountConfig, SyncState types
│
├── src-tauri/
│   └── src/
│       ├── lib.rs                      # Register new commands, start sync workers
│       ├── state.rs                    # Add search index + sync handles to AppState
│       └── commands/
│           ├── mod.rs
│           ├── health.rs
│           ├── accounts.rs            # add_account, list_accounts, delete_account
│           ├── messages.rs            # list_messages, get_message, get_rendered_html, update_flags
│           ├── folders.rs             # list_folders
│           ├── search.rs              # search_messages
│           └── sync.rs               # start_sync, stop_sync
│
├── src/
│   ├── lib/
│   │   └── api.ts                     # All new IPC wrappers (typed)
│   ├── stores/
│   │   ├── ui.store.ts
│   │   └── mail.store.ts             # MailStore: accounts, folders, messages, selectedMessage
│   ├── components/
│   │   ├── Sidebar.tsx                # Add folder list under nav items
│   │   ├── MessageList.tsx            # Virtual-scrolled message list
│   │   ├── MessageItem.tsx            # Single message row (from, subject, date, flags)
│   │   ├── MessageDetail.tsx          # Full message view with HTML rendering
│   │   ├── PrivacyBanner.tsx          # "N trackers blocked" banner with load/trust buttons
│   │   ├── SearchBar.tsx              # Search input in toolbar area
│   │   └── AccountSetup.tsx           # Add-account dialog (IMAP server, credentials)
│   ├── features/
│   │   ├── inbox/
│   │   │   └── InboxView.tsx          # Rewrite: message list + detail split pane
│   │   └── settings/
│   │       └── SettingsView.tsx       # Account management section
│   └── hooks/
│       ├── useMessages.ts            # Fetch + paginate messages for a folder
│       ├── useSearch.ts              # Search query + results
│       └── useFolders.ts             # Fetch folders for account
```

---

### Task 1: pebble-mail crate — IMAP provider

**Files:**
- Create: `crates/pebble-mail/Cargo.toml`
- Create: `crates/pebble-mail/src/lib.rs`
- Create: `crates/pebble-mail/src/imap.rs`
- Create: `crates/pebble-mail/src/parser.rs`
- Create: `crates/pebble-mail/src/thread.rs`
- Modify: `Cargo.toml` (workspace members + dependencies)

**Context:** This crate implements the `MailTransport` and `FolderProvider` traits from pebble-core for IMAP accounts. Uses `async-imap` for IMAP protocol, `mail-parser` for MIME parsing. The `MailTransport` trait uses `impl Future` return types (not async_trait), so the implementation must follow this pattern.

Note: `MailTransport` is NOT object-safe (uses `impl Future`). ImapProvider will implement it directly. If polymorphism is needed later, use enum dispatch.

- [ ] **Step 1: Create crate and Cargo.toml**

Add `pebble-mail` to workspace members in root `Cargo.toml`:
```toml
members = [
    "src-tauri",
    "crates/pebble-core",
    "crates/pebble-store",
    "crates/pebble-privacy",
    "crates/pebble-mail",
]
```

Add workspace dependencies to root `Cargo.toml`:
```toml
async-imap = "0.10"
async-native-tls = "0.5"
mail-parser = "0.9"
lettre = { version = "0.11", default-features = false, features = ["tokio1-rustls-tls", "smtp-transport", "builder"] }
```

Create `crates/pebble-mail/Cargo.toml`:
```toml
[package]
name = "pebble-mail"
version = "0.1.0"
edition = "2021"

[dependencies]
pebble-core = { path = "../pebble-core" }
thiserror = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
async-imap = { workspace = true }
async-native-tls = { workspace = true }
mail-parser = { workspace = true }
```

- [ ] **Step 2: Write parser.rs — MIME parsing tests**

Create `crates/pebble-mail/src/parser.rs`. Write failing tests first:

```rust
use pebble_core::{Message, EmailAddress, Attachment};

/// Parsed output from a raw email
pub struct ParsedMessage {
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_header: Option<String>,
    pub subject: String,
    pub from_address: String,
    pub from_name: String,
    pub to_list: Vec<EmailAddress>,
    pub cc_list: Vec<EmailAddress>,
    pub bcc_list: Vec<EmailAddress>,
    pub date: i64,
    pub body_text: String,
    pub body_html: String,
    pub snippet: String,
    pub has_attachments: bool,
    pub attachments: Vec<AttachmentMeta>,
}

pub struct AttachmentMeta {
    pub filename: String,
    pub mime_type: String,
    pub size: usize,
}

/// Parse raw email bytes (RFC 5322 / MIME) into ParsedMessage
pub fn parse_raw_email(raw: &[u8]) -> pebble_core::Result<ParsedMessage> {
    let message = mail_parser::MessageParser::default()
        .parse(raw)
        .ok_or_else(|| pebble_core::PebbleError::Sync("Failed to parse MIME message".into()))?;

    let from = message.from().and_then(|f| f.first());
    let from_address = from
        .and_then(|a| a.address())
        .unwrap_or_default()
        .to_string();
    let from_name = from
        .and_then(|a| a.name())
        .unwrap_or_default()
        .to_string();

    let to_list = extract_addresses(message.to());
    let cc_list = extract_addresses(message.cc());
    let bcc_list = extract_addresses(message.bcc());

    let subject = message.subject().unwrap_or_default().to_string();
    let body_text = message.body_text(0).unwrap_or_default().to_string();
    let body_html = message.body_html(0).unwrap_or_default().to_string();

    let snippet = make_snippet(&body_text, 200);

    let date = message.date()
        .map(|d| d.to_timestamp())
        .unwrap_or(0);

    let message_id_header = message.message_id().map(|s| format!("<{s}>"));
    let in_reply_to = message.in_reply_to().as_text_list()
        .and_then(|list| list.first().map(|s| format!("<{s}>")));
    let references_header = message.references().as_text_list()
        .map(|list| list.iter().map(|s| format!("<{s}>")).collect::<Vec<_>>().join(" "));

    let mut attachments = Vec::new();
    let has_attachments = message.attachment_count() > 0;
    for attachment in message.attachments() {
        if attachment.is_message() { continue; }
        let filename = attachment.attachment_name().unwrap_or("unnamed").to_string();
        let mime_type = attachment.content_type()
            .map(|ct| {
                let main = ct.ctype();
                match ct.subtype() {
                    Some(sub) => format!("{main}/{sub}"),
                    None => main.to_string(),
                }
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let size = attachment.len();
        attachments.push(AttachmentMeta { filename, mime_type, size });
    }

    Ok(ParsedMessage {
        message_id_header,
        in_reply_to,
        references_header,
        subject,
        from_address,
        from_name,
        to_list,
        cc_list,
        bcc_list,
        date,
        body_text,
        body_html,
        snippet,
        has_attachments,
        attachments,
    })
}

fn extract_addresses(header: Option<&mail_parser::HeaderValue>) -> Vec<EmailAddress> {
    match header {
        Some(mail_parser::HeaderValue::Address(addr)) => {
            vec![EmailAddress {
                name: addr.name().map(|s| s.to_string()),
                address: addr.address().unwrap_or_default().to_string(),
            }]
        }
        Some(mail_parser::HeaderValue::AddressList(list)) => {
            list.iter().map(|addr| EmailAddress {
                name: addr.name().map(|s| s.to_string()),
                address: addr.address().unwrap_or_default().to_string(),
            }).collect()
        }
        Some(mail_parser::HeaderValue::Group(groups)) => {
            groups.iter().flat_map(|g| {
                g.addresses().iter().map(|addr| EmailAddress {
                    name: addr.name().map(|s| s.to_string()),
                    address: addr.address().unwrap_or_default().to_string(),
                })
            }).collect()
        }
        Some(mail_parser::HeaderValue::GroupList(groups)) => {
            groups.iter().flat_map(|g| {
                g.addresses().iter().map(|addr| EmailAddress {
                    name: addr.name().map(|s| s.to_string()),
                    address: addr.address().unwrap_or_default().to_string(),
                })
            }).collect()
        }
        _ => vec![],
    }
}

fn make_snippet(text: &str, max_len: usize) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.len() <= max_len {
        trimmed.to_string()
    } else {
        format!("{}...", &trimmed[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_EMAIL: &[u8] = b"From: Sender Name <sender@example.com>\r\n\
        To: Recipient <recipient@example.com>\r\n\
        Subject: Test Subject\r\n\
        Message-ID: <abc123@example.com>\r\n\
        Date: Fri, 04 Apr 2026 12:00:00 +0000\r\n\
        Content-Type: text/plain; charset=utf-8\r\n\
        \r\n\
        Hello, this is a test email body.";

    #[test]
    fn test_parse_simple_email() {
        let parsed = parse_raw_email(SIMPLE_EMAIL).unwrap();
        assert_eq!(parsed.subject, "Test Subject");
        assert_eq!(parsed.from_address, "sender@example.com");
        assert_eq!(parsed.from_name, "Sender Name");
        assert_eq!(parsed.to_list.len(), 1);
        assert_eq!(parsed.to_list[0].address, "recipient@example.com");
        assert!(parsed.body_text.contains("test email body"));
        assert_eq!(parsed.message_id_header, Some("<abc123@example.com>".to_string()));
        assert!(!parsed.has_attachments);
    }

    #[test]
    fn test_parse_html_email() {
        let raw = b"From: sender@example.com\r\n\
            To: recipient@example.com\r\n\
            Subject: HTML Email\r\n\
            Content-Type: multipart/alternative; boundary=\"boundary\"\r\n\
            \r\n\
            --boundary\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            Plain text version\r\n\
            --boundary\r\n\
            Content-Type: text/html\r\n\
            \r\n\
            <html><body><p>HTML version</p></body></html>\r\n\
            --boundary--";
        let parsed = parse_raw_email(raw).unwrap();
        assert_eq!(parsed.subject, "HTML Email");
        assert!(parsed.body_text.contains("Plain text"));
        assert!(parsed.body_html.contains("<p>HTML version</p>"));
    }

    #[test]
    fn test_snippet_truncation() {
        let snippet = make_snippet(&"a".repeat(300), 200);
        assert!(snippet.len() <= 203); // 200 + "..."
        assert!(snippet.ends_with("..."));
    }

    #[test]
    fn test_parse_with_in_reply_to() {
        let raw = b"From: sender@example.com\r\n\
            To: recipient@example.com\r\n\
            Subject: Re: Original\r\n\
            Message-ID: <reply@example.com>\r\n\
            In-Reply-To: <original@example.com>\r\n\
            References: <original@example.com>\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            Reply body";
        let parsed = parse_raw_email(raw).unwrap();
        assert_eq!(parsed.message_id_header, Some("<reply@example.com>".to_string()));
        assert_eq!(parsed.in_reply_to, Some("<original@example.com>".to_string()));
        assert!(parsed.references_header.is_some());
    }
}
```

- [ ] **Step 3: Run parser tests**

Run: `cargo test -p pebble-mail -- --nocapture`
Expected: All 4 tests pass.

- [ ] **Step 4: Write thread.rs — thread aggregation**

Create `crates/pebble-mail/src/thread.rs`:

```rust
use pebble_core::Message;

/// Compute a thread_id for a message based on its headers.
/// Uses References/In-Reply-To to link replies to originals.
/// Falls back to subject-based grouping (stripped Re:/Fwd: prefix).
pub fn compute_thread_id(message: &Message, existing_threads: &[(String, String)]) -> String {
    // 1. Check if this message is a reply to a known thread via In-Reply-To
    if let Some(ref in_reply_to) = message.in_reply_to {
        for (msg_id_header, thread_id) in existing_threads {
            if msg_id_header == in_reply_to {
                return thread_id.clone();
            }
        }
    }

    // 2. Check References header for any known message
    if let Some(ref references) = message.references_header {
        for ref_id in references.split_whitespace() {
            for (msg_id_header, thread_id) in existing_threads {
                if msg_id_header == ref_id {
                    return thread_id.clone();
                }
            }
        }
    }

    // 3. No known parent — this message starts a new thread.
    // Use the message's own Message-ID as the thread_id if available.
    message.message_id_header.clone()
        .unwrap_or_else(|| pebble_core::new_id())
}

/// Strip Re:, Fwd:, etc. prefixes from a subject for thread matching.
pub fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        if lower.starts_with("re:") {
            s = s[3..].trim_start();
        } else if lower.starts_with("fwd:") {
            s = s[4..].trim_start();
        } else if lower.starts_with("fw:") {
            s = s[3..].trim_start();
        } else if lower.starts_with("回复:") || lower.starts_with("回复：") {
            s = s["回复:".len()..].trim_start();
        } else if lower.starts_with("转发:") || lower.starts_with("转发：") {
            s = s["转发:".len()..].trim_start();
        } else {
            break;
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_subject() {
        assert_eq!(normalize_subject("Re: Hello"), "Hello");
        assert_eq!(normalize_subject("Re: Re: Fwd: Test"), "Test");
        assert_eq!(normalize_subject("Hello"), "Hello");
        assert_eq!(normalize_subject("FWD: Fw: Re: Deep"), "Deep");
    }

    #[test]
    fn test_compute_thread_id_new_thread() {
        let msg = Message {
            id: "m1".into(),
            account_id: "a1".into(),
            remote_id: "1".into(),
            message_id_header: Some("<msg1@example.com>".into()),
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Hello".into(),
            snippet: "".into(),
            from_address: "a@b.com".into(),
            from_name: "".into(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "".into(),
            body_html_raw: "".into(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 0,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 0,
            updated_at: 0,
        };
        let existing: Vec<(String, String)> = vec![];
        let tid = compute_thread_id(&msg, &existing);
        assert_eq!(tid, "<msg1@example.com>");
    }

    #[test]
    fn test_compute_thread_id_reply() {
        let msg = Message {
            id: "m2".into(),
            account_id: "a1".into(),
            remote_id: "2".into(),
            message_id_header: Some("<msg2@example.com>".into()),
            in_reply_to: Some("<msg1@example.com>".into()),
            references_header: Some("<msg1@example.com>".into()),
            thread_id: None,
            subject: "Re: Hello".into(),
            snippet: "".into(),
            from_address: "b@c.com".into(),
            from_name: "".into(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "".into(),
            body_html_raw: "".into(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 0,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 0,
            updated_at: 0,
        };
        let existing = vec![
            ("<msg1@example.com>".to_string(), "thread-1".to_string()),
        ];
        let tid = compute_thread_id(&msg, &existing);
        assert_eq!(tid, "thread-1");
    }
}
```

- [ ] **Step 5: Write imap.rs — ImapProvider**

Create `crates/pebble-mail/src/imap.rs`:

```rust
use async_imap::types::Fetch;
use pebble_core::*;
use crate::parser::{parse_raw_email, ParsedMessage};
use crate::traits::*;
use std::sync::Arc;
use tokio::sync::Mutex;

/// IMAP server configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub use_tls: bool,
}

/// SMTP server configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub use_tls: bool,
}

pub struct ImapProvider {
    config: ImapConfig,
    session: Arc<Mutex<Option<async_imap::Session<async_native_tls::TlsStream<tokio::net::TcpStream>>>>>,
}

impl ImapProvider {
    pub fn new(config: ImapConfig) -> Self {
        Self {
            config,
            session: Arc::new(Mutex::new(None)),
        }
    }

    /// Establish IMAP connection and login
    pub async fn connect(&self) -> Result<()> {
        let tls = async_native_tls::TlsConnector::new();
        let client = async_imap::connect(
            (self.config.host.as_str(), self.config.port),
            &self.config.host,
            tls,
        ).await.map_err(|e| PebbleError::Network(format!("IMAP connect failed: {e}")))?;

        let session = client.login(&self.config.username, &self.config.password)
            .await
            .map_err(|e| PebbleError::Auth(format!("IMAP login failed: {}", e.0)))?;

        let mut guard = self.session.lock().await;
        *guard = Some(session);
        Ok(())
    }

    /// List all IMAP mailboxes and map to Folder structs
    pub async fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        let mut guard = self.session.lock().await;
        let session = guard.as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".into()))?;

        let mailboxes = session.list(Some(""), Some("*"))
            .await
            .map_err(|e| PebbleError::Network(format!("LIST failed: {e}")))?;

        let mut folders = Vec::new();
        for mb in mailboxes.iter() {
            let name = mb.name().to_string();
            let role = detect_folder_role(&name);
            let folder = Folder {
                id: new_id(),
                account_id: account_id.to_string(),
                remote_id: name.clone(),
                name: name.rsplit('/').next().unwrap_or(&name).to_string(),
                folder_type: FolderType::Folder,
                role,
                parent_id: None,
                color: None,
                is_system: role.is_some(),
                sort_order: folder_sort_order(&role),
            };
            folders.push(folder);
        }

        Ok(folders)
    }

    /// Fetch messages from a mailbox. Returns (uid, raw_bytes) pairs.
    /// `since_uid` fetches UIDs > since_uid. If None, fetches last `limit` messages.
    pub async fn fetch_messages_raw(
        &self,
        mailbox: &str,
        since_uid: Option<u32>,
        limit: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut guard = self.session.lock().await;
        let session = guard.as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".into()))?;

        let _mailbox = session.select(mailbox)
            .await
            .map_err(|e| PebbleError::Network(format!("SELECT {mailbox} failed: {e}")))?;

        let sequence = match since_uid {
            Some(uid) => format!("{}:*", uid + 1),
            None => {
                // Fetch last N messages by sequence number
                let exists = _mailbox.exists;
                if exists == 0 { return Ok(vec![]); }
                let start = exists.saturating_sub(limit) + 1;
                format!("{start}:*")
            }
        };

        let fetch_cmd = if since_uid.is_some() {
            session.uid_fetch(&sequence, "(UID FLAGS BODY.PEEK[] INTERNALDATE)")
                .await
                .map_err(|e| PebbleError::Network(format!("UID FETCH failed: {e}")))?
        } else {
            session.fetch(&sequence, "(UID FLAGS BODY.PEEK[] INTERNALDATE)")
                .await
                .map_err(|e| PebbleError::Network(format!("FETCH failed: {e}")))?
        };

        let mut results = Vec::new();
        for msg in fetch_cmd.iter() {
            if let (Some(uid), Some(body)) = (msg.uid, msg.body()) {
                results.push((uid, body.to_vec()));
            }
        }

        Ok(results)
    }

    /// Fetch flags for UIDs (for reconciliation)
    pub async fn fetch_flags(
        &self,
        mailbox: &str,
        uids: &[u32],
    ) -> Result<Vec<(u32, bool, bool)>> {
        if uids.is_empty() { return Ok(vec![]); }

        let mut guard = self.session.lock().await;
        let session = guard.as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".into()))?;

        session.select(mailbox)
            .await
            .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

        let uid_set: String = uids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
        let fetches = session.uid_fetch(&uid_set, "FLAGS")
            .await
            .map_err(|e| PebbleError::Network(format!("UID FETCH FLAGS failed: {e}")))?;

        let mut results = Vec::new();
        for msg in fetches.iter() {
            if let Some(uid) = msg.uid {
                let flags = msg.flags();
                let is_read = flags.iter().any(|f| matches!(f, async_imap::types::Flag::Seen));
                let is_starred = flags.iter().any(|f| matches!(f, async_imap::types::Flag::Flagged));
                results.push((uid, is_read, is_starred));
            }
        }

        Ok(results)
    }

    /// Set/clear flags on the remote server
    pub async fn set_flags(
        &self,
        mailbox: &str,
        uid: u32,
        is_read: Option<bool>,
        is_starred: Option<bool>,
    ) -> Result<()> {
        let mut guard = self.session.lock().await;
        let session = guard.as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".into()))?;

        session.select(mailbox)
            .await
            .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

        let uid_str = uid.to_string();

        if let Some(read) = is_read {
            let flag = "(\\Seen)";
            if read {
                session.uid_store(&uid_str, "+FLAGS.SILENT (\\Seen)")
                    .await
                    .map_err(|e| PebbleError::Network(format!("STORE +Seen failed: {e}")))?;
            } else {
                session.uid_store(&uid_str, "-FLAGS.SILENT (\\Seen)")
                    .await
                    .map_err(|e| PebbleError::Network(format!("STORE -Seen failed: {e}")))?;
            }
        }

        if let Some(starred) = is_starred {
            if starred {
                session.uid_store(&uid_str, "+FLAGS.SILENT (\\Flagged)")
                    .await
                    .map_err(|e| PebbleError::Network(format!("STORE +Flagged failed: {e}")))?;
            } else {
                session.uid_store(&uid_str, "-FLAGS.SILENT (\\Flagged)")
                    .await
                    .map_err(|e| PebbleError::Network(format!("STORE -Flagged failed: {e}")))?;
            }
        }

        Ok(())
    }

    /// Disconnect gracefully
    pub async fn disconnect(&self) -> Result<()> {
        let mut guard = self.session.lock().await;
        if let Some(mut session) = guard.take() {
            let _ = session.logout().await;
        }
        Ok(())
    }
}

fn detect_folder_role(name: &str) -> Option<FolderRole> {
    let lower = name.to_lowercase();
    if lower == "inbox" { return Some(FolderRole::Inbox); }
    let leaf = lower.rsplit('/').next().unwrap_or(&lower);
    match leaf {
        "sent" | "sent items" | "sent messages" | "[gmail]/sent mail" => Some(FolderRole::Sent),
        "drafts" | "draft" | "[gmail]/drafts" => Some(FolderRole::Drafts),
        "trash" | "deleted" | "deleted items" | "[gmail]/trash" => Some(FolderRole::Trash),
        "archive" | "all mail" | "[gmail]/all mail" => Some(FolderRole::Archive),
        "spam" | "junk" | "junk email" | "[gmail]/spam" => Some(FolderRole::Spam),
        _ => None,
    }
}

fn folder_sort_order(role: &Option<FolderRole>) -> i32 {
    match role {
        Some(FolderRole::Inbox) => 0,
        Some(FolderRole::Drafts) => 1,
        Some(FolderRole::Sent) => 2,
        Some(FolderRole::Archive) => 3,
        Some(FolderRole::Spam) => 4,
        Some(FolderRole::Trash) => 5,
        None => 100,
    }
}
```

- [ ] **Step 6: Write sync.rs — sync worker**

Create `crates/pebble-mail/src/sync.rs`:

```rust
use crate::imap::ImapProvider;
use crate::parser::parse_raw_email;
use crate::thread::{compute_thread_id, normalize_subject};
use pebble_core::*;
use pebble_store::Store;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{info, warn, error};

pub struct SyncWorker {
    account_id: String,
    provider: Arc<ImapProvider>,
    store: Arc<Store>,
    stop_rx: watch::Receiver<bool>,
}

/// Configuration for sync intervals
pub struct SyncConfig {
    pub poll_interval_secs: u64,      // Default: 60
    pub reconcile_interval_secs: u64, // Default: 900 (15 min)
    pub initial_fetch_limit: u32,     // Default: 200 (last N messages on first sync)
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 60,
            reconcile_interval_secs: 900,
            initial_fetch_limit: 200,
        }
    }
}

impl SyncWorker {
    pub fn new(
        account_id: String,
        provider: Arc<ImapProvider>,
        store: Arc<Store>,
        stop_rx: watch::Receiver<bool>,
    ) -> Self {
        Self { account_id, provider, store, stop_rx }
    }

    /// Run initial sync: list folders, fetch recent messages
    pub async fn initial_sync(&self) -> Result<()> {
        info!(account_id = %self.account_id, "Starting initial sync");

        // 1. List and store folders
        let folders = self.provider.list_folders(&self.account_id).await?;
        for folder in &folders {
            let _ = self.store.insert_folder(folder); // ignore duplicate errors
        }
        info!(account_id = %self.account_id, folder_count = folders.len(), "Folders synced");

        // 2. Fetch messages from inbox
        let inbox = folders.iter().find(|f| f.role == Some(FolderRole::Inbox));
        if let Some(inbox_folder) = inbox {
            self.sync_folder(inbox_folder, None, 200).await?;
        }

        Ok(())
    }

    /// Sync a single folder: fetch raw messages, parse, store
    pub async fn sync_folder(
        &self,
        folder: &Folder,
        since_uid: Option<u32>,
        limit: u32,
    ) -> Result<u32> {
        let raw_messages = self.provider
            .fetch_messages_raw(&folder.remote_id, since_uid, limit)
            .await?;

        info!(
            account_id = %self.account_id,
            folder = %folder.name,
            count = raw_messages.len(),
            "Fetched raw messages"
        );

        // Load existing thread mappings for this account
        // (message_id_header -> thread_id) for thread computation
        let existing_threads = self.store.get_thread_mappings(&self.account_id)
            .unwrap_or_default();

        let mut max_uid = since_uid.unwrap_or(0);
        let mut new_count = 0u32;

        for (uid, raw) in &raw_messages {
            if *uid > max_uid { max_uid = *uid; }

            // Check if we already have this message (by remote_id = UID)
            let remote_id = uid.to_string();
            if self.store.has_message_by_remote_id(&self.account_id, &remote_id)
                .unwrap_or(false)
            {
                continue;
            }

            match parse_raw_email(raw) {
                Ok(parsed) => {
                    let now = now_timestamp();
                    let mut msg = Message {
                        id: new_id(),
                        account_id: self.account_id.clone(),
                        remote_id: remote_id.clone(),
                        message_id_header: parsed.message_id_header,
                        in_reply_to: parsed.in_reply_to,
                        references_header: parsed.references_header,
                        thread_id: None,
                        subject: parsed.subject,
                        snippet: parsed.snippet,
                        from_address: parsed.from_address,
                        from_name: parsed.from_name,
                        to_list: parsed.to_list,
                        cc_list: parsed.cc_list,
                        bcc_list: parsed.bcc_list,
                        body_text: parsed.body_text,
                        body_html_raw: parsed.body_html,
                        has_attachments: parsed.has_attachments,
                        is_read: false, // Will be updated from FLAGS
                        is_starred: false,
                        is_draft: false,
                        date: parsed.date,
                        remote_version: Some(uid.to_string()),
                        is_deleted: false,
                        deleted_at: None,
                        created_at: now,
                        updated_at: now,
                    };

                    // Compute thread_id
                    msg.thread_id = Some(compute_thread_id(&msg, &existing_threads));

                    if let Err(e) = self.store.insert_message(&msg, &[folder.id.clone()]) {
                        warn!(uid = uid, error = %e, "Failed to insert message");
                    } else {
                        new_count += 1;
                    }
                }
                Err(e) => {
                    warn!(uid = uid, error = %e, "Failed to parse message");
                }
            }
        }

        info!(
            account_id = %self.account_id,
            folder = %folder.name,
            new_messages = new_count,
            max_uid = max_uid,
            "Folder sync complete"
        );

        Ok(max_uid)
    }

    /// Run the periodic sync loop
    pub async fn run(&self, config: SyncConfig) {
        let poll_interval = tokio::time::Duration::from_secs(config.poll_interval_secs);
        let mut stop_rx = self.stop_rx.clone();

        loop {
            tokio::select! {
                _ = tokio::time::sleep(poll_interval) => {
                    if let Err(e) = self.poll_new_messages().await {
                        error!(account_id = %self.account_id, error = %e, "Poll failed");
                    }
                }
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        info!(account_id = %self.account_id, "Sync worker stopping");
                        break;
                    }
                }
            }
        }
    }

    async fn poll_new_messages(&self) -> Result<()> {
        let folders = self.store.list_folders(&self.account_id)?;
        let inbox = folders.iter().find(|f| f.role == Some(FolderRole::Inbox));
        if let Some(inbox_folder) = inbox {
            // Get the max UID we've seen for this folder
            let max_uid = self.store
                .get_max_remote_id(&self.account_id, &inbox_folder.id)
                .unwrap_or(None)
                .and_then(|s| s.parse::<u32>().ok());
            self.sync_folder(inbox_folder, max_uid, 100).await?;
        }
        Ok(())
    }
}
```

- [ ] **Step 7: Write lib.rs for pebble-mail**

Create `crates/pebble-mail/src/lib.rs`:

```rust
pub mod imap;
pub mod parser;
pub mod sync;
pub mod thread;

pub use imap::{ImapConfig, ImapProvider, SmtpConfig};
pub use sync::{SyncWorker, SyncConfig};
```

- [ ] **Step 8: Add store helper methods for sync**

Add to `crates/pebble-store/src/messages.rs` — new methods needed by sync:

```rust
/// Check if a message with this remote_id exists for this account
pub fn has_message_by_remote_id(&self, account_id: &str, remote_id: &str) -> Result<bool> {
    self.with_conn(|conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE account_id = ?1 AND remote_id = ?2",
            params![account_id, remote_id],
            |row| row.get(0),
        ).map_err(|e| PebbleError::Storage(e.to_string()))?;
        Ok(count > 0)
    })
}

/// Get the maximum remote_id (as string) for messages in a folder
pub fn get_max_remote_id(&self, account_id: &str, folder_id: &str) -> Result<Option<String>> {
    self.with_conn(|conn| {
        let result: Option<String> = conn.query_row(
            "SELECT MAX(CAST(m.remote_id AS INTEGER))
             FROM messages m
             JOIN message_folders mf ON m.id = mf.message_id
             WHERE m.account_id = ?1 AND mf.folder_id = ?2 AND m.is_deleted = 0",
            params![account_id, folder_id],
            |row| row.get(0),
        ).optional()
        .map_err(|e| PebbleError::Storage(e.to_string()))?
        .flatten();
        Ok(result)
    })
}

/// Get (message_id_header, thread_id) pairs for thread computation
pub fn get_thread_mappings(&self, account_id: &str) -> Result<Vec<(String, String)>> {
    self.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT message_id_header, thread_id FROM messages
             WHERE account_id = ?1 AND message_id_header IS NOT NULL AND thread_id IS NOT NULL"
        ).map_err(|e| PebbleError::Storage(e.to_string()))?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).map_err(|e| PebbleError::Storage(e.to_string()))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| PebbleError::Storage(e.to_string()))?);
        }
        Ok(results)
    })
}
```

- [ ] **Step 9: Run all Rust tests**

Run: `cargo test --workspace`
Expected: All existing tests + new parser/thread tests pass.

- [ ] **Step 10: Commit**

```bash
git add crates/pebble-mail/ Cargo.toml crates/pebble-store/src/messages.rs
git commit -m "feat(mail): add pebble-mail crate with IMAP provider, MIME parser, sync worker, and thread aggregation"
```

---

### Task 2: pebble-search crate — Tantivy full-text search

**Files:**
- Create: `crates/pebble-search/Cargo.toml`
- Create: `crates/pebble-search/src/lib.rs`
- Create: `crates/pebble-search/src/schema.rs`
- Modify: `Cargo.toml` (workspace members + dependency)

**Context:** Implements the `SearchEngine` trait from pebble-core using Tantivy. Index stored in app_data_dir/index/tantivy/. Separate from SQLite — can be rebuilt from store data.

- [ ] **Step 1: Set up crate**

Add `pebble-search` to workspace members in root `Cargo.toml`:
```toml
members = [
    "src-tauri",
    "crates/pebble-core",
    "crates/pebble-store",
    "crates/pebble-privacy",
    "crates/pebble-mail",
    "crates/pebble-search",
]
```

Add workspace dependency:
```toml
tantivy = "0.22"
```

Create `crates/pebble-search/Cargo.toml`:
```toml
[package]
name = "pebble-search"
version = "0.1.0"
edition = "2021"

[dependencies]
pebble-core = { path = "../pebble-core" }
thiserror = { workspace = true }
tantivy = { workspace = true }
tracing = { workspace = true }
```

- [ ] **Step 2: Write schema.rs**

Create `crates/pebble-search/src/schema.rs`:

```rust
use tantivy::schema::*;

/// Build the Tantivy index schema for email messages.
///
/// Fields indexed:
/// - message_id (STORED, not indexed — for linking back to SQLite)
/// - subject (TEXT, full-text)
/// - body_text (TEXT, full-text)
/// - from_address (TEXT, full-text)
/// - from_name (TEXT, full-text)
/// - to_addresses (TEXT, full-text)
/// - date (DATE, indexed for range queries)
/// - folder_id (STRING, indexed for filtering)
/// - account_id (STRING, indexed for filtering)
/// - has_attachment (BOOL, indexed for filtering)
pub fn build_schema() -> Schema {
    let mut builder = Schema::builder();

    builder.add_text_field("message_id", STRING | STORED);
    builder.add_text_field("subject", TEXT | STORED);
    builder.add_text_field("body_text", TEXT);
    builder.add_text_field("from_address", TEXT | STORED);
    builder.add_text_field("from_name", TEXT | STORED);
    builder.add_text_field("to_addresses", TEXT);
    builder.add_date_field("date", INDEXED | STORED);
    builder.add_text_field("folder_id", STRING);
    builder.add_text_field("account_id", STRING);
    builder.add_text_field("has_attachment", STRING); // "true"/"false" as string

    builder.build()
}
```

- [ ] **Step 3: Write lib.rs with TantivySearch implementation + tests**

Create `crates/pebble-search/src/lib.rs`:

```rust
pub mod schema;

use pebble_core::*;
use schema::build_schema;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, QueryParser, TermQuery};
use tantivy::{Directory, Index, IndexWriter, ReloadPolicy, TantivyDocument};
use tracing::info;

pub struct TantivySearch {
    index: Index,
    writer: std::sync::Mutex<IndexWriter>,
}

impl TantivySearch {
    /// Open or create a Tantivy index at the given directory path
    pub fn open(index_path: &Path) -> Result<Self> {
        std::fs::create_dir_all(index_path)
            .map_err(|e| PebbleError::Storage(format!("Failed to create index dir: {e}")))?;

        let schema = build_schema();
        let index = Index::open_or_create(
            tantivy::directory::MmapDirectory::open(index_path)
                .map_err(|e| PebbleError::Storage(format!("Tantivy dir error: {e}")))?,
            schema.clone(),
        ).map_err(|e| PebbleError::Storage(format!("Tantivy index error: {e}")))?;

        let writer = index.writer(50_000_000) // 50MB writer heap
            .map_err(|e| PebbleError::Storage(format!("Tantivy writer error: {e}")))?;

        Ok(Self {
            index,
            writer: std::sync::Mutex::new(writer),
        })
    }

    /// Open an in-memory index for testing
    pub fn open_in_memory() -> Result<Self> {
        let schema = build_schema();
        let index = Index::create_in_ram(schema);
        let writer = index.writer(15_000_000)
            .map_err(|e| PebbleError::Storage(format!("Tantivy writer error: {e}")))?;
        Ok(Self {
            index,
            writer: std::sync::Mutex::new(writer),
        })
    }

    /// Index a single message
    pub fn index_message(&self, msg: &Message, folder_ids: &[String]) -> Result<()> {
        let schema = self.index.schema();
        let mut doc = TantivyDocument::default();

        doc.add_text(schema.get_field("message_id").unwrap(), &msg.id);
        doc.add_text(schema.get_field("subject").unwrap(), &msg.subject);
        doc.add_text(schema.get_field("body_text").unwrap(), &msg.body_text);
        doc.add_text(schema.get_field("from_address").unwrap(), &msg.from_address);
        doc.add_text(schema.get_field("from_name").unwrap(), &msg.from_name);

        let to_str: String = msg.to_list.iter()
            .map(|a| format!("{} {}", a.name.as_deref().unwrap_or(""), &a.address))
            .collect::<Vec<_>>()
            .join(" ");
        doc.add_text(schema.get_field("to_addresses").unwrap(), &to_str);

        let date = tantivy::DateTime::from_timestamp_secs(msg.date);
        doc.add_date(schema.get_field("date").unwrap(), date);

        doc.add_text(schema.get_field("account_id").unwrap(), &msg.account_id);
        doc.add_text(schema.get_field("has_attachment").unwrap(),
            if msg.has_attachments { "true" } else { "false" });

        for fid in folder_ids {
            doc.add_text(schema.get_field("folder_id").unwrap(), fid);
        }

        let writer = self.writer.lock()
            .map_err(|e| PebbleError::Internal(format!("Writer lock poisoned: {e}")))?;
        writer.add_document(doc)
            .map_err(|e| PebbleError::Storage(format!("Index add failed: {e}")))?;

        Ok(())
    }

    /// Commit pending writes to the index
    pub fn commit(&self) -> Result<()> {
        let mut writer = self.writer.lock()
            .map_err(|e| PebbleError::Internal(format!("Writer lock poisoned: {e}")))?;
        writer.commit()
            .map_err(|e| PebbleError::Storage(format!("Index commit failed: {e}")))?;
        Ok(())
    }

    /// Search messages by free-text query
    pub fn search(&self, query_text: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let reader = self.index.reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| PebbleError::Storage(format!("Reader error: {e}")))?;

        let searcher = reader.searcher();
        let schema = self.index.schema();

        let subject_field = schema.get_field("subject").unwrap();
        let body_field = schema.get_field("body_text").unwrap();
        let from_field = schema.get_field("from_address").unwrap();
        let from_name_field = schema.get_field("from_name").unwrap();

        let query_parser = QueryParser::for_index(
            &self.index,
            vec![subject_field, body_field, from_field, from_name_field],
        );

        let query = query_parser.parse_query(query_text)
            .map_err(|e| PebbleError::Storage(format!("Query parse error: {e}")))?;

        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))
            .map_err(|e| PebbleError::Storage(format!("Search error: {e}")))?;

        let message_id_field = schema.get_field("message_id").unwrap();
        let mut results = Vec::new();

        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)
                .map_err(|e| PebbleError::Storage(format!("Doc retrieve error: {e}")))?;

            let message_id = doc.get_first(message_id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let snippet = doc.get_first(subject_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            results.push(SearchHit {
                message_id,
                score,
                snippet,
            });
        }

        Ok(results)
    }

    /// Delete all documents and rebuild from scratch
    pub fn clear_index(&self) -> Result<()> {
        let mut writer = self.writer.lock()
            .map_err(|e| PebbleError::Internal(format!("Writer lock poisoned: {e}")))?;
        writer.delete_all_documents()
            .map_err(|e| PebbleError::Storage(format!("Clear index failed: {e}")))?;
        writer.commit()
            .map_err(|e| PebbleError::Storage(format!("Clear commit failed: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_message(id: &str, subject: &str, body: &str, from: &str) -> Message {
        Message {
            id: id.to_string(),
            account_id: "acc1".to_string(),
            remote_id: "1".to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: subject.to_string(),
            snippet: "".to_string(),
            from_address: from.to_string(),
            from_name: "".to_string(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: body.to_string(),
            body_html_raw: "".to_string(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 1712188800,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn test_index_and_search_by_subject() {
        let search = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message("m1", "Meeting tomorrow at 10am", "Let's discuss the project", "alice@example.com");
        search.index_message(&msg, &["folder1".to_string()]).unwrap();
        search.commit().unwrap();

        let results = search.search("meeting tomorrow", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message_id, "m1");
    }

    #[test]
    fn test_search_by_body() {
        let search = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message("m1", "Subject", "The quarterly budget report is attached", "bob@corp.com");
        search.index_message(&msg, &["f1".to_string()]).unwrap();
        search.commit().unwrap();

        let results = search.search("quarterly budget", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message_id, "m1");
    }

    #[test]
    fn test_search_by_from() {
        let search = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message("m1", "Hello", "Hi there", "unique-sender@example.com");
        search.index_message(&msg, &["f1".to_string()]).unwrap();
        search.commit().unwrap();

        let results = search.search("unique-sender", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_no_results() {
        let search = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message("m1", "Alpha", "Bravo charlie", "delta@echo.com");
        search.index_message(&msg, &["f1".to_string()]).unwrap();
        search.commit().unwrap();

        let results = search.search("zzzznonexistent", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_clear_index() {
        let search = TantivySearch::open_in_memory().unwrap();
        let msg = make_test_message("m1", "Test", "Body", "a@b.com");
        search.index_message(&msg, &["f1".to_string()]).unwrap();
        search.commit().unwrap();

        search.clear_index().unwrap();
        let results = search.search("Test", 10).unwrap();
        assert!(results.is_empty());
    }
}
```

- [ ] **Step 4: Run search tests**

Run: `cargo test -p pebble-search`
Expected: All 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/pebble-search/ Cargo.toml
git commit -m "feat(search): add pebble-search crate with Tantivy full-text search"
```

---

### Task 3: Replace hand-rolled HTML sanitizer with ammonia

**Files:**
- Modify: `crates/pebble-privacy/Cargo.toml`
- Modify: `crates/pebble-privacy/src/sanitizer.rs`
- Modify: `Cargo.toml` (workspace dependency)

**Context:** The Phase 1 code review identified the hand-rolled HTML sanitizer as a security risk (XSS bypass vectors: event handlers, javascript: URLs, `<style>` tags, SVG). Replace it with the `ammonia` crate which uses an allowlist model. Preserve the same `PrivacyGuard` public API: `render_safe_html(raw_html, PrivacyMode) -> RenderedHtml`.

- [ ] **Step 1: Add ammonia dependency**

Add to root `Cargo.toml` workspace dependencies:
```toml
ammonia = "4"
```

Add to `crates/pebble-privacy/Cargo.toml`:
```toml
ammonia = { workspace = true }
```

- [ ] **Step 2: Rewrite sanitizer.rs using ammonia**

Replace `crates/pebble-privacy/src/sanitizer.rs` entirely:

```rust
use ammonia::Builder;
use pebble_core::{PrivacyMode, RenderedHtml, TrackerInfo};
use crate::tracker::{is_known_tracker, is_tracking_pixel};
use std::collections::HashSet;
use std::borrow::Cow;

pub struct PrivacyGuard;

impl PrivacyGuard {
    pub fn new() -> Self {
        Self
    }

    /// Render raw HTML into safe HTML based on privacy mode.
    /// 1. Always strip dangerous tags/attributes (scripts, event handlers, etc.) via ammonia
    /// 2. Based on mode: block/allow external images, detect trackers
    pub fn render_safe_html(&self, raw_html: &str, mode: &PrivacyMode) -> RenderedHtml {
        let mut trackers_blocked = Vec::new();
        let mut images_blocked: u32 = 0;

        // Phase 1: Detect trackers and images in the raw HTML before sanitization
        // We scan for <img> tags to count blocked images and detect trackers
        let processed_html = preprocess_images(raw_html, mode, &mut trackers_blocked, &mut images_blocked);

        // Phase 2: Sanitize with ammonia (allowlist-based, strips all dangerous content)
        let clean = build_sanitizer(mode).clean(&processed_html).to_string();

        RenderedHtml {
            html: clean,
            trackers_blocked,
            images_blocked,
        }
    }
}

/// Build an ammonia sanitizer configured for our needs.
fn build_sanitizer(mode: &PrivacyMode) -> Builder<'static> {
    let mut builder = Builder::default();

    // Allow standard safe HTML tags
    let allowed_tags: HashSet<&str> = [
        "a", "abbr", "b", "blockquote", "br", "code", "dd", "div", "dl", "dt",
        "em", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "i", "img", "li",
        "ol", "p", "pre", "s", "span", "strong", "sub", "sup", "table",
        "tbody", "td", "th", "thead", "tr", "u", "ul", "center", "font",
    ].iter().copied().collect();
    builder.tags(allowed_tags);

    // Allow safe attributes
    let mut tag_attrs = std::collections::HashMap::new();
    tag_attrs.insert("a", ["href", "title", "target", "rel"].iter().copied().collect::<HashSet<&str>>());
    tag_attrs.insert("img", ["src", "alt", "width", "height", "title"].iter().copied().collect());
    tag_attrs.insert("td", ["colspan", "rowspan", "align", "valign"].iter().copied().collect());
    tag_attrs.insert("th", ["colspan", "rowspan", "align", "valign"].iter().copied().collect());
    tag_attrs.insert("table", ["border", "cellpadding", "cellspacing", "width"].iter().copied().collect());
    tag_attrs.insert("div", ["align"].iter().copied().collect());
    tag_attrs.insert("p", ["align"].iter().copied().collect());
    tag_attrs.insert("font", ["color", "size", "face"].iter().copied().collect());
    builder.tag_attributes(tag_attrs);

    // Allow style attribute globally for email rendering
    builder.add_generic_attributes(["style", "class", "dir"]);

    // Strip javascript: URLs
    builder.url_schemes(["http", "https", "mailto"].iter().copied().collect());

    // Force target="_blank" and rel="noopener noreferrer" on links
    builder.link_rel(Some("noopener noreferrer"));

    builder
}

/// Pre-process HTML to handle images based on privacy mode.
/// Replaces blocked images with placeholders, detects tracking pixels.
fn preprocess_images(
    html: &str,
    mode: &PrivacyMode,
    trackers: &mut Vec<TrackerInfo>,
    images_blocked: &mut u32,
) -> String {
    // Simple regex-free approach: find <img tags and process their src attributes
    let mut result = String::with_capacity(html.len());
    let mut pos = 0;
    let html_bytes = html.as_bytes();

    while pos < html.len() {
        if let Some(img_start) = find_tag_start(html, pos, "img") {
            // Copy everything before <img
            result.push_str(&html[pos..img_start]);

            // Find end of img tag
            let tag_end = match html[img_start..].find('>') {
                Some(offset) => img_start + offset + 1,
                None => {
                    result.push_str(&html[img_start..]);
                    break;
                }
            };

            let img_tag = &html[img_start..tag_end];
            let src = extract_attr_value(img_tag, "src");
            let width = extract_attr_value(img_tag, "width");
            let height = extract_attr_value(img_tag, "height");

            let is_external = src.as_ref().map_or(false, |s|
                s.starts_with("http://") || s.starts_with("https://"));

            if is_external {
                let src_str = src.as_deref().unwrap_or("");
                let domain = extract_domain_from_url(src_str);

                // Check if it's a tracking pixel
                let is_pixel = is_tracking_pixel(
                    width.as_deref().unwrap_or(""),
                    height.as_deref().unwrap_or(""),
                );
                let is_tracker = domain.as_ref().map_or(false, |d| is_known_tracker(d));

                if is_pixel || is_tracker {
                    trackers.push(TrackerInfo {
                        domain: domain.unwrap_or_default(),
                        tracker_type: if is_pixel { "pixel".to_string() } else { "known_domain".to_string() },
                    });
                    *images_blocked += 1;
                    // Always block tracking pixels/known trackers regardless of mode
                    result.push_str("<!-- tracker blocked -->");
                    pos = tag_end;
                    continue;
                }

                match mode {
                    PrivacyMode::Strict => {
                        *images_blocked += 1;
                        result.push_str(&format!(
                            "<img alt=\"[Image blocked]\" title=\"External image blocked\" \
                             style=\"background:#f0f0f0;padding:8px;border:1px dashed #ccc;\" />"
                        ));
                    }
                    PrivacyMode::LoadOnce | PrivacyMode::TrustSender(_) => {
                        result.push_str(img_tag);
                    }
                }
            } else {
                // Inline/data URI images — pass through
                result.push_str(img_tag);
            }

            pos = tag_end;
        } else {
            result.push_str(&html[pos..]);
            break;
        }
    }

    result
}

fn find_tag_start(html: &str, from: usize, tag: &str) -> Option<usize> {
    let search = html[from..].to_lowercase();
    let pattern = format!("<{}", tag);
    search.find(&pattern).map(|offset| {
        let pos = from + offset;
        // Ensure it's actually a tag (followed by space, /, or >)
        let after = pos + pattern.len();
        if after < html.len() {
            let next_char = html.as_bytes()[after];
            if next_char == b' ' || next_char == b'>' || next_char == b'/' || next_char == b'\t' || next_char == b'\n' {
                return Some(pos);
            }
        }
        None
    }).flatten()
}

fn extract_attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let pattern = format!("{}=", attr);
    if let Some(start) = lower.find(&pattern) {
        let after_eq = start + pattern.len();
        let rest = &tag[after_eq..];
        let trimmed = rest.trim_start();
        if trimmed.starts_with('"') {
            let inner = &trimmed[1..];
            if let Some(end) = inner.find('"') {
                return Some(inner[..end].to_string());
            }
        } else if trimmed.starts_with('\'') {
            let inner = &trimmed[1..];
            if let Some(end) = inner.find('\'') {
                return Some(inner[..end].to_string());
            }
        } else {
            // Unquoted attribute value
            let end = trimmed.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(trimmed.len());
            return Some(trimmed[..end].to_string());
        }
    }
    None
}

fn extract_domain_from_url(url: &str) -> Option<String> {
    let without_scheme = url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let domain = without_scheme.split('/').next()?;
    let domain = domain.split(':').next()?; // strip port
    Some(domain.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_removes_script_tags() {
        let guard = PrivacyGuard::new();
        let html = "<p>Hello</p><script>alert('xss')</script><p>World</p>";
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(result.html.contains("Hello"));
        assert!(result.html.contains("World"));
        assert!(!result.html.contains("script"));
        assert!(!result.html.contains("alert"));
    }

    #[test]
    fn test_removes_event_handlers() {
        let guard = PrivacyGuard::new();
        let html = r#"<p onmouseover="alert(1)">Hover me</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(result.html.contains("Hover me"));
        assert!(!result.html.contains("onmouseover"));
    }

    #[test]
    fn test_blocks_javascript_urls() {
        let guard = PrivacyGuard::new();
        let html = r#"<a href="javascript:alert(1)">Click</a>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("javascript:"));
    }

    #[test]
    fn test_removes_iframe_tags() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><iframe src="http://evil.com"></iframe>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("iframe"));
    }

    #[test]
    fn test_removes_style_tags() {
        let guard = PrivacyGuard::new();
        let html = r#"<style>body { background: url('http://tracker.com/pixel.gif'); }</style><p>Content</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("<style>"));
        assert!(result.html.contains("Content"));
    }

    #[test]
    fn test_blocks_tracking_pixel() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://tracking.example.com/pixel.gif" width="1" height="1">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(result.trackers_blocked.len() >= 1 || result.images_blocked >= 1);
    }

    #[test]
    fn test_blocks_known_tracker_domain() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://open.mailchimp.com/track/abc123" width="100" height="100">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.trackers_blocked.is_empty());
    }

    #[test]
    fn test_blocks_external_images_in_strict_mode() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://example.com/photo.jpg" width="500" height="300">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(result.images_blocked >= 1);
        assert!(result.html.contains("Image blocked"));
    }

    #[test]
    fn test_allows_images_in_load_once_mode() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://example.com/photo.jpg" width="500" height="300">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);
        assert_eq!(result.images_blocked, 0);
        assert!(result.html.contains("example.com/photo.jpg"));
    }

    #[test]
    fn test_still_blocks_trackers_in_load_once_mode() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://open.mailchimp.com/track/abc" width="1" height="1">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);
        assert!(!result.trackers_blocked.is_empty());
    }

    #[test]
    fn test_removes_svg_with_event_handlers() {
        let guard = PrivacyGuard::new();
        let html = r#"<svg onload="alert(1)"><circle r="10"/></svg>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("onload"));
        assert!(!result.html.contains("svg")); // svg not in allowed tags
    }
}
```

- [ ] **Step 3: Run privacy tests**

Run: `cargo test -p pebble-privacy`
Expected: All 11 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/pebble-privacy/ Cargo.toml
git commit -m "fix(privacy): replace hand-rolled HTML sanitizer with ammonia for XSS safety"
```

---

### Task 4: Tauri IPC commands — accounts, messages, folders, search, sync

**Files:**
- Create: `src-tauri/src/commands/accounts.rs`
- Create: `src-tauri/src/commands/messages.rs`
- Create: `src-tauri/src/commands/folders.rs`
- Create: `src-tauri/src/commands/search.rs`
- Create: `src-tauri/src/commands/sync_cmd.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

**Context:** Wire up the Rust backend to the frontend via Tauri IPC. AppState needs to hold the search index and sync worker handles. Commands return `Result<T, PebbleError>` which Tauri serializes to the frontend.

- [ ] **Step 1: Update AppState to hold search index and sync handles**

Modify `src-tauri/src/state.rs`:

```rust
use pebble_mail::{ImapProvider, SyncWorker};
use pebble_search::TantivySearch;
use pebble_store::Store;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

pub struct SyncHandle {
    pub stop_tx: watch::Sender<bool>,
    pub task: tokio::task::JoinHandle<()>,
}

pub struct AppState {
    pub store: Arc<Store>,
    pub search: Arc<TantivySearch>,
    pub sync_handles: Mutex<HashMap<String, SyncHandle>>,
}

impl AppState {
    pub fn new(store: Store, search: TantivySearch) -> Self {
        Self {
            store: Arc::new(store),
            search: Arc::new(search),
            sync_handles: Mutex::new(HashMap::new()),
        }
    }
}
```

- [ ] **Step 2: Create accounts.rs commands**

Create `src-tauri/src/commands/accounts.rs`:

```rust
use pebble_core::{Account, PebbleError, Result};
use pebble_mail::ImapConfig;
use tauri::State;
use crate::state::AppState;

#[derive(serde::Deserialize)]
pub struct AddAccountRequest {
    pub email: String,
    pub display_name: String,
    pub provider: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
    pub use_tls: bool,
}

#[tauri::command]
pub async fn add_account(
    state: State<'_, AppState>,
    request: AddAccountRequest,
) -> std::result::Result<Account, PebbleError> {
    let now = pebble_core::now_timestamp();
    let account = Account {
        id: pebble_core::new_id(),
        email: request.email,
        display_name: request.display_name,
        provider: match request.provider.as_str() {
            "gmail" => pebble_core::ProviderType::Gmail,
            "outlook" => pebble_core::ProviderType::Outlook,
            _ => pebble_core::ProviderType::Imap,
        },
        created_at: now,
        updated_at: now,
    };
    state.store.insert_account(&account)?;

    // Store IMAP config as JSON in sync_state for now
    // TODO: Encrypt credentials properly with AES-256-GCM
    let config = ImapConfig {
        host: request.imap_host,
        port: request.imap_port,
        username: request.username,
        password: request.password,
        use_tls: request.use_tls,
    };
    let config_json = serde_json::to_string(&config)
        .map_err(|e| PebbleError::Internal(e.to_string()))?;
    state.store.update_account_sync_state(&account.id, &config_json)?;

    Ok(account)
}

#[tauri::command]
pub async fn list_accounts(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<Account>, PebbleError> {
    state.store.list_accounts()
}

#[tauri::command]
pub async fn delete_account(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<(), PebbleError> {
    // Stop sync worker if running
    let mut handles = state.sync_handles.lock().await;
    if let Some(handle) = handles.remove(&account_id) {
        let _ = handle.stop_tx.send(true);
        let _ = handle.task.await;
    }
    state.store.delete_account(&account_id)
}
```

- [ ] **Step 3: Create messages.rs commands**

Create `src-tauri/src/commands/messages.rs`:

```rust
use pebble_core::{Message, PebbleError, PrivacyMode, RenderedHtml};
use pebble_privacy::sanitizer::PrivacyGuard;
use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn list_messages(
    state: State<'_, AppState>,
    folder_id: String,
    limit: u32,
    offset: u32,
) -> std::result::Result<Vec<Message>, PebbleError> {
    state.store.list_messages_by_folder(&folder_id, limit, offset)
}

#[tauri::command]
pub async fn get_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<Option<Message>, PebbleError> {
    state.store.get_message(&message_id)
}

#[tauri::command]
pub async fn get_rendered_html(
    state: State<'_, AppState>,
    message_id: String,
    privacy_mode: PrivacyMode,
) -> std::result::Result<RenderedHtml, PebbleError> {
    let msg = state.store.get_message(&message_id)?
        .ok_or_else(|| PebbleError::Storage("Message not found".into()))?;

    let guard = PrivacyGuard::new();
    Ok(guard.render_safe_html(&msg.body_html_raw, &privacy_mode))
}

#[tauri::command]
pub async fn update_message_flags(
    state: State<'_, AppState>,
    message_id: String,
    is_read: Option<bool>,
    is_starred: Option<bool>,
) -> std::result::Result<(), PebbleError> {
    state.store.update_message_flags(&message_id, is_read, is_starred)
}
```

- [ ] **Step 4: Create folders.rs commands**

Create `src-tauri/src/commands/folders.rs`:

```rust
use pebble_core::{Folder, PebbleError};
use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn list_folders(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<Vec<Folder>, PebbleError> {
    state.store.list_folders(&account_id)
}
```

- [ ] **Step 5: Create search.rs commands**

Create `src-tauri/src/commands/search.rs`:

```rust
use pebble_core::{PebbleError, SearchHit};
use tauri::State;
use crate::state::AppState;

#[tauri::command]
pub async fn search_messages(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> std::result::Result<Vec<SearchHit>, PebbleError> {
    state.search.search(&query, limit.unwrap_or(50))
}
```

- [ ] **Step 6: Create sync_cmd.rs commands**

Create `src-tauri/src/commands/sync_cmd.rs`:

```rust
use pebble_core::PebbleError;
use pebble_mail::{ImapConfig, ImapProvider, SyncConfig, SyncWorker};
use tauri::State;
use crate::state::{AppState, SyncHandle};
use std::sync::Arc;

#[tauri::command]
pub async fn start_sync(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<String, PebbleError> {
    let mut handles = state.sync_handles.lock().await;
    if handles.contains_key(&account_id) {
        return Ok("Already syncing".into());
    }

    // Load IMAP config from sync_state
    let config_json = state.store.get_account_sync_state(&account_id)?
        .ok_or_else(|| PebbleError::Auth("No connection config found".into()))?;
    let imap_config: ImapConfig = serde_json::from_str(&config_json)
        .map_err(|e| PebbleError::Internal(format!("Invalid config: {e}")))?;

    let provider = Arc::new(ImapProvider::new(imap_config));
    provider.connect().await?;

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let worker = SyncWorker::new(
        account_id.clone(),
        provider.clone(),
        state.store.clone(),
        stop_rx,
    );

    // Run initial sync then start poll loop
    worker.initial_sync().await?;

    let task = tokio::spawn(async move {
        worker.run(SyncConfig::default()).await;
    });

    handles.insert(account_id, SyncHandle { stop_tx, task });

    Ok("Sync started".into())
}

#[tauri::command]
pub async fn stop_sync(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<(), PebbleError> {
    let mut handles = state.sync_handles.lock().await;
    if let Some(handle) = handles.remove(&account_id) {
        let _ = handle.stop_tx.send(true);
    }
    Ok(())
}
```

- [ ] **Step 7: Update mod.rs and add store helpers**

Update `src-tauri/src/commands/mod.rs`:
```rust
pub mod accounts;
pub mod folders;
pub mod health;
pub mod messages;
pub mod search;
pub mod sync_cmd;
```

Add to `crates/pebble-store/src/accounts.rs`:

```rust
/// Store connection config (IMAP settings) in sync_state column
pub fn update_account_sync_state(&self, account_id: &str, sync_state: &str) -> Result<()> {
    self.with_conn(|conn| {
        conn.execute(
            "UPDATE accounts SET sync_state = ?1, updated_at = ?2 WHERE id = ?3",
            params![sync_state, pebble_core::now_timestamp(), account_id],
        ).map_err(|e| PebbleError::Storage(e.to_string()))?;
        Ok(())
    })
}

/// Get connection config from sync_state column
pub fn get_account_sync_state(&self, account_id: &str) -> Result<Option<String>> {
    self.with_conn(|conn| {
        let result: Option<String> = conn.query_row(
            "SELECT sync_state FROM accounts WHERE id = ?1",
            params![account_id],
            |row| row.get(0),
        ).optional()
        .map_err(|e| PebbleError::Storage(e.to_string()))?
        .flatten();
        Ok(result)
    })
}
```

- [ ] **Step 8: Update lib.rs to register all commands and initialize search**

Update `src-tauri/src/lib.rs`:

```rust
mod commands;
mod events;
mod state;

use state::AppState;
use std::path::PathBuf;
use tauri::Manager;

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

fn get_index_path(app: &tauri::App) -> PathBuf {
    let app_data = app
        .path()
        .app_data_dir()
        .expect("Failed to get app data directory");
    let index_dir = app_data.join("index").join("tantivy");
    std::fs::create_dir_all(&index_dir).expect("Failed to create index directory");
    index_dir
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pebble=debug,pebble_store=debug,pebble_mail=debug,pebble_search=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let db_path = get_db_path(app);
            tracing::info!("Database path: {}", db_path.display());
            let store =
                pebble_store::Store::open(&db_path).expect("Failed to open database");
            tracing::info!("Database initialized successfully");

            let index_path = get_index_path(app);
            tracing::info!("Index path: {}", index_path.display());
            let search =
                pebble_search::TantivySearch::open(&index_path).expect("Failed to open search index");
            tracing::info!("Search index initialized successfully");

            app.manage(AppState::new(store, search));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::health::health_check,
            commands::accounts::add_account,
            commands::accounts::list_accounts,
            commands::accounts::delete_account,
            commands::folders::list_folders,
            commands::messages::list_messages,
            commands::messages::get_message,
            commands::messages::get_rendered_html,
            commands::messages::update_message_flags,
            commands::search::search_messages,
            commands::sync_cmd::start_sync,
            commands::sync_cmd::stop_sync,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 9: Update src-tauri/Cargo.toml dependencies**

Add `pebble-mail` and `pebble-search` to `src-tauri/Cargo.toml`:
```toml
pebble-mail = { path = "../crates/pebble-mail" }
pebble-search = { path = "../crates/pebble-search" }
```

- [ ] **Step 10: Run cargo clippy and fix any issues**

Run: `cargo clippy --workspace -- -D warnings`
Expected: Clean (0 warnings).

- [ ] **Step 11: Commit**

```bash
git add src-tauri/ crates/pebble-store/src/accounts.rs
git commit -m "feat(ipc): add Tauri commands for accounts, messages, folders, search, and sync"
```

---

### Task 5: Frontend — MailStore + API layer + hooks

**Files:**
- Modify: `src/lib/api.ts`
- Create: `src/stores/mail.store.ts`
- Create: `src/hooks/useMessages.ts`
- Create: `src/hooks/useFolders.ts`
- Create: `src/hooks/useSearch.ts`
- Modify: `package.json` (add @tanstack/react-virtual)

**Context:** Build the data layer that the UI components will consume. MailStore holds accounts, active account, folders, selected folder. Hooks use the API to fetch data. No TanStack Query for now — keep it simple with Zustand + manual fetches.

- [ ] **Step 1: Install frontend dependency**

```bash
pnpm add @tanstack/react-virtual
```

- [ ] **Step 2: Expand api.ts with all IPC wrappers**

Replace `src/lib/api.ts`:

```typescript
import { invoke } from "@tauri-apps/api/core";

// Types matching Rust structs
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

export interface Message {
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
  body_text: string;
  body_html_raw: string;
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

export interface RenderedHtml {
  html: string;
  trackers_blocked: { domain: string; tracker_type: string }[];
  images_blocked: number;
}

export interface SearchHit {
  message_id: string;
  score: number;
  snippet: string;
}

export type PrivacyMode =
  | "Strict"
  | { TrustSender: string }
  | "LoadOnce";

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
  use_tls: boolean;
}

// Health
export async function healthCheck(): Promise<string> {
  return invoke<string>("health_check");
}

// Accounts
export async function addAccount(request: AddAccountRequest): Promise<Account> {
  return invoke<Account>("add_account", { request });
}

export async function listAccounts(): Promise<Account[]> {
  return invoke<Account[]>("list_accounts");
}

export async function deleteAccount(accountId: string): Promise<void> {
  return invoke<void>("delete_account", { accountId });
}

// Folders
export async function listFolders(accountId: string): Promise<Folder[]> {
  return invoke<Folder[]>("list_folders", { accountId });
}

// Messages
export async function listMessages(
  folderId: string,
  limit: number,
  offset: number
): Promise<Message[]> {
  return invoke<Message[]>("list_messages", { folderId, limit, offset });
}

export async function getMessage(messageId: string): Promise<Message | null> {
  return invoke<Message | null>("get_message", { messageId });
}

export async function getRenderedHtml(
  messageId: string,
  privacyMode: PrivacyMode
): Promise<RenderedHtml> {
  return invoke<RenderedHtml>("get_rendered_html", { messageId, privacyMode });
}

export async function updateMessageFlags(
  messageId: string,
  isRead?: boolean,
  isStarred?: boolean
): Promise<void> {
  return invoke<void>("update_message_flags", { messageId, isRead, isStarred });
}

// Search
export async function searchMessages(
  query: string,
  limit?: number
): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search_messages", { query, limit });
}

// Sync
export async function startSync(accountId: string): Promise<string> {
  return invoke<string>("start_sync", { accountId });
}

export async function stopSync(accountId: string): Promise<void> {
  return invoke<void>("stop_sync", { accountId });
}
```

- [ ] **Step 3: Create mail.store.ts**

Create `src/stores/mail.store.ts`:

```typescript
import { create } from "zustand";
import {
  Account,
  Folder,
  Message,
  listAccounts,
  listFolders,
  listMessages,
  startSync,
} from "@/lib/api";

interface MailState {
  // Data
  accounts: Account[];
  folders: Folder[];
  messages: Message[];
  selectedMessageId: string | null;

  // Active selections
  activeAccountId: string | null;
  activeFolderId: string | null;

  // Loading states
  loadingMessages: boolean;
  loadingFolders: boolean;

  // Actions
  fetchAccounts: () => Promise<void>;
  fetchFolders: (accountId: string) => Promise<void>;
  fetchMessages: (folderId: string, limit?: number, offset?: number) => Promise<void>;
  setActiveAccount: (accountId: string) => Promise<void>;
  setActiveFolder: (folderId: string) => Promise<void>;
  setSelectedMessage: (messageId: string | null) => void;
  syncAccount: (accountId: string) => Promise<void>;
}

export const useMailStore = create<MailState>((set, get) => ({
  accounts: [],
  folders: [],
  messages: [],
  selectedMessageId: null,
  activeAccountId: null,
  activeFolderId: null,
  loadingMessages: false,
  loadingFolders: false,

  fetchAccounts: async () => {
    const accounts = await listAccounts();
    set({ accounts });
  },

  fetchFolders: async (accountId: string) => {
    set({ loadingFolders: true });
    const folders = await listFolders(accountId);
    // Sort by sort_order
    folders.sort((a, b) => a.sort_order - b.sort_order);
    set({ folders, loadingFolders: false });
  },

  fetchMessages: async (folderId: string, limit = 50, offset = 0) => {
    set({ loadingMessages: true });
    const messages = await listMessages(folderId, limit, offset);
    set({ messages, loadingMessages: false });
  },

  setActiveAccount: async (accountId: string) => {
    set({ activeAccountId: accountId, folders: [], messages: [], selectedMessageId: null });
    await get().fetchFolders(accountId);
    // Auto-select inbox
    const inbox = get().folders.find((f) => f.role === "inbox");
    if (inbox) {
      await get().setActiveFolder(inbox.id);
    }
  },

  setActiveFolder: async (folderId: string) => {
    set({ activeFolderId: folderId, messages: [], selectedMessageId: null });
    await get().fetchMessages(folderId);
  },

  setSelectedMessage: (messageId: string | null) => {
    set({ selectedMessageId: messageId });
  },

  syncAccount: async (accountId: string) => {
    await startSync(accountId);
    // After sync starts, refresh folders and messages
    await get().fetchFolders(accountId);
    const activeFolderId = get().activeFolderId;
    if (activeFolderId) {
      await get().fetchMessages(activeFolderId);
    }
  },
}));
```

- [ ] **Step 4: Create hooks**

Create `src/hooks/useMessages.ts`:

```typescript
import { useEffect } from "react";
import { useMailStore } from "@/stores/mail.store";

export function useMessages() {
  const {
    messages,
    loadingMessages,
    activeFolderId,
    fetchMessages,
    selectedMessageId,
    setSelectedMessage,
  } = useMailStore();

  useEffect(() => {
    if (activeFolderId) {
      fetchMessages(activeFolderId);
    }
  }, [activeFolderId, fetchMessages]);

  return { messages, loading: loadingMessages, selectedMessageId, setSelectedMessage };
}
```

Create `src/hooks/useFolders.ts`:

```typescript
import { useEffect } from "react";
import { useMailStore } from "@/stores/mail.store";

export function useFolders() {
  const {
    folders,
    loadingFolders,
    activeAccountId,
    activeFolderId,
    fetchFolders,
    setActiveFolder,
  } = useMailStore();

  useEffect(() => {
    if (activeAccountId) {
      fetchFolders(activeAccountId);
    }
  }, [activeAccountId, fetchFolders]);

  return { folders, loading: loadingFolders, activeFolderId, setActiveFolder };
}
```

Create `src/hooks/useSearch.ts`:

```typescript
import { useState, useCallback } from "react";
import { searchMessages, SearchHit } from "@/lib/api";

export function useSearch() {
  const [results, setResults] = useState<SearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");

  const search = useCallback(async (q: string) => {
    setQuery(q);
    if (!q.trim()) {
      setResults([]);
      return;
    }
    setLoading(true);
    try {
      const hits = await searchMessages(q, 50);
      setResults(hits);
    } finally {
      setLoading(false);
    }
  }, []);

  const clear = useCallback(() => {
    setQuery("");
    setResults([]);
  }, []);

  return { results, loading, query, search, clear };
}
```

- [ ] **Step 5: Commit**

```bash
git add src/lib/api.ts src/stores/mail.store.ts src/hooks/ package.json pnpm-lock.yaml
git commit -m "feat(frontend): add MailStore, API layer, and data hooks for mail/search"
```

---

### Task 6: Frontend — Inbox UI components

**Files:**
- Create: `src/components/MessageList.tsx`
- Create: `src/components/MessageItem.tsx`
- Create: `src/components/MessageDetail.tsx`
- Create: `src/components/PrivacyBanner.tsx`
- Create: `src/components/SearchBar.tsx`
- Modify: `src/features/inbox/InboxView.tsx`
- Modify: `src/components/Sidebar.tsx`
- Modify: `src/app/Layout.tsx`

**Context:** Build the core email client UI. InboxView becomes a split-pane: MessageList on left, MessageDetail on right. Sidebar shows folder list. SearchBar in toolbar area. MessageDetail renders sanitized HTML via the `get_rendered_html` IPC call and shows a PrivacyBanner.

- [ ] **Step 1: Create MessageItem.tsx**

Create `src/components/MessageItem.tsx`:

```tsx
import { Message } from "@/lib/api";
import { Star, Paperclip } from "lucide-react";

interface Props {
  message: Message;
  isSelected: boolean;
  onClick: () => void;
}

function formatDate(timestamp: number): string {
  const date = new Date(timestamp * 1000);
  const now = new Date();
  const isToday = date.toDateString() === now.toDateString();
  if (isToday) {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}

export default function MessageItem({ message, isSelected, onClick }: Props) {
  return (
    <div
      onClick={onClick}
      className="flex items-start gap-3 px-4 py-3 cursor-pointer border-b transition-colors"
      style={{
        backgroundColor: isSelected
          ? "var(--color-sidebar-active)"
          : "transparent",
        borderColor: "var(--color-border)",
        fontWeight: message.is_read ? "normal" : "600",
      }}
    >
      <div className="flex-1 min-w-0">
        <div className="flex items-center justify-between gap-2">
          <span
            className="text-sm truncate"
            style={{ color: "var(--color-text-primary)" }}
          >
            {message.from_name || message.from_address}
          </span>
          <span
            className="text-xs whitespace-nowrap"
            style={{ color: "var(--color-text-secondary)" }}
          >
            {formatDate(message.date)}
          </span>
        </div>
        <div
          className="text-sm truncate mt-0.5"
          style={{ color: "var(--color-text-primary)" }}
        >
          {message.subject || "(no subject)"}
        </div>
        <div
          className="text-xs truncate mt-0.5"
          style={{ color: "var(--color-text-secondary)" }}
        >
          {message.snippet}
        </div>
      </div>
      <div className="flex items-center gap-1 pt-0.5">
        {message.is_starred && (
          <Star size={14} fill="var(--color-accent)" stroke="var(--color-accent)" />
        )}
        {message.has_attachments && (
          <Paperclip size={14} style={{ color: "var(--color-text-secondary)" }} />
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Create MessageList.tsx with virtual scrolling**

Create `src/components/MessageList.tsx`:

```tsx
import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Message } from "@/lib/api";
import MessageItem from "./MessageItem";

interface Props {
  messages: Message[];
  selectedMessageId: string | null;
  onSelectMessage: (id: string) => void;
  loading: boolean;
}

export default function MessageList({
  messages,
  selectedMessageId,
  onSelectMessage,
  loading,
}: Props) {
  const parentRef = useRef<HTMLDivElement>(null);

  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76, // estimated row height
  });

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <span style={{ color: "var(--color-text-secondary)" }}>Loading...</span>
      </div>
    );
  }

  if (messages.length === 0) {
    return (
      <div className="flex items-center justify-center h-full">
        <span style={{ color: "var(--color-text-secondary)" }}>No messages</span>
      </div>
    );
  }

  return (
    <div ref={parentRef} className="h-full overflow-auto">
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((virtualItem) => {
          const message = messages[virtualItem.index];
          return (
            <div
              key={message.id}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${virtualItem.start}px)`,
              }}
            >
              <MessageItem
                message={message}
                isSelected={selectedMessageId === message.id}
                onClick={() => onSelectMessage(message.id)}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Create PrivacyBanner.tsx**

Create `src/components/PrivacyBanner.tsx`:

```tsx
import { Shield, Eye } from "lucide-react";
import { RenderedHtml } from "@/lib/api";

interface Props {
  rendered: RenderedHtml;
  onLoadImages: () => void;
  onTrustSender: () => void;
}

export default function PrivacyBanner({
  rendered,
  onLoadImages,
  onTrustSender,
}: Props) {
  const totalBlocked =
    rendered.trackers_blocked.length + rendered.images_blocked;

  if (totalBlocked === 0) return null;

  return (
    <div
      className="flex items-center gap-3 px-4 py-2 text-sm"
      style={{
        backgroundColor: "var(--color-sidebar-bg)",
        borderBottom: "1px solid var(--color-border)",
        color: "var(--color-text-secondary)",
      }}
    >
      <Shield size={16} />
      <span>
        Blocked {rendered.trackers_blocked.length > 0 && (
          <strong>{rendered.trackers_blocked.length} tracker{rendered.trackers_blocked.length !== 1 ? "s" : ""}</strong>
        )}
        {rendered.trackers_blocked.length > 0 && rendered.images_blocked > 0 && " and "}
        {rendered.images_blocked > 0 && (
          <strong>{rendered.images_blocked} image{rendered.images_blocked !== 1 ? "s" : ""}</strong>
        )}
      </span>
      <div className="flex items-center gap-2 ml-auto">
        <button
          onClick={onLoadImages}
          className="text-xs px-2 py-1 rounded hover:bg-black/5"
          style={{ color: "var(--color-accent)" }}
        >
          Load images
        </button>
        <button
          onClick={onTrustSender}
          className="text-xs px-2 py-1 rounded hover:bg-black/5"
          style={{ color: "var(--color-accent)" }}
        >
          Trust sender
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Create MessageDetail.tsx**

Create `src/components/MessageDetail.tsx`:

```tsx
import { useEffect, useState } from "react";
import { getMessage, getRenderedHtml, updateMessageFlags } from "@/lib/api";
import type { Message, RenderedHtml, PrivacyMode } from "@/lib/api";
import PrivacyBanner from "./PrivacyBanner";
import { ArrowLeft, Star } from "lucide-react";

interface Props {
  messageId: string;
  onBack: () => void;
}

export default function MessageDetail({ messageId, onBack }: Props) {
  const [message, setMessage] = useState<Message | null>(null);
  const [rendered, setRendered] = useState<RenderedHtml | null>(null);
  const [privacyMode, setPrivacyMode] = useState<PrivacyMode>("Strict");

  useEffect(() => {
    let cancelled = false;

    async function load() {
      const msg = await getMessage(messageId);
      if (cancelled || !msg) return;
      setMessage(msg);

      // Mark as read
      if (!msg.is_read) {
        await updateMessageFlags(messageId, true, undefined);
      }

      // Render HTML
      const html = await getRenderedHtml(messageId, "Strict");
      if (!cancelled) {
        setRendered(html);
        setPrivacyMode("Strict");
      }
    }

    load();
    return () => { cancelled = true; };
  }, [messageId]);

  const reloadWithMode = async (mode: PrivacyMode) => {
    setPrivacyMode(mode);
    const html = await getRenderedHtml(messageId, mode);
    setRendered(html);
  };

  if (!message) {
    return (
      <div className="flex items-center justify-center h-full">
        <span style={{ color: "var(--color-text-secondary)" }}>Loading...</span>
      </div>
    );
  }

  const date = new Date(message.date * 1000);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div
        className="flex items-center gap-3 px-4 py-3 border-b"
        style={{ borderColor: "var(--color-border)" }}
      >
        <button
          onClick={onBack}
          className="p-1 rounded hover:bg-black/5"
          aria-label="Back"
        >
          <ArrowLeft size={18} />
        </button>
        <div className="flex-1 min-w-0">
          <h2
            className="text-base font-semibold truncate"
            style={{ color: "var(--color-text-primary)" }}
          >
            {message.subject || "(no subject)"}
          </h2>
          <div className="flex items-center gap-2 text-sm mt-0.5">
            <span style={{ color: "var(--color-text-primary)" }}>
              {message.from_name || message.from_address}
            </span>
            <span style={{ color: "var(--color-text-secondary)" }}>
              &lt;{message.from_address}&gt;
            </span>
            <span
              className="ml-auto text-xs"
              style={{ color: "var(--color-text-secondary)" }}
            >
              {date.toLocaleString()}
            </span>
          </div>
        </div>
      </div>

      {/* Privacy Banner */}
      {rendered && (
        <PrivacyBanner
          rendered={rendered}
          onLoadImages={() => reloadWithMode("LoadOnce")}
          onTrustSender={() =>
            reloadWithMode({ TrustSender: message.from_address })
          }
        />
      )}

      {/* Body */}
      <div className="flex-1 overflow-auto p-4">
        {rendered ? (
          <div
            className="prose max-w-none"
            style={{ color: "var(--color-text-primary)" }}
            dangerouslySetInnerHTML={{ __html: rendered.html }}
          />
        ) : (
          <pre
            className="whitespace-pre-wrap text-sm"
            style={{ color: "var(--color-text-primary)" }}
          >
            {message.body_text}
          </pre>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Create SearchBar.tsx**

Create `src/components/SearchBar.tsx`:

```tsx
import { useState, useCallback } from "react";
import { Search, X } from "lucide-react";

interface Props {
  onSearch: (query: string) => void;
  onClear: () => void;
}

export default function SearchBar({ onSearch, onClear }: Props) {
  const [value, setValue] = useState("");

  const handleSubmit = useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      onSearch(value);
    },
    [value, onSearch]
  );

  const handleClear = useCallback(() => {
    setValue("");
    onClear();
  }, [onClear]);

  return (
    <form
      onSubmit={handleSubmit}
      className="flex items-center gap-2 px-3 py-1.5 rounded-md"
      style={{
        backgroundColor: "var(--color-sidebar-bg)",
        border: "1px solid var(--color-border)",
      }}
    >
      <Search size={16} style={{ color: "var(--color-text-secondary)" }} />
      <input
        type="text"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        placeholder="Search emails..."
        className="flex-1 bg-transparent text-sm outline-none"
        style={{ color: "var(--color-text-primary)" }}
      />
      {value && (
        <button type="button" onClick={handleClear} className="p-0.5">
          <X size={14} style={{ color: "var(--color-text-secondary)" }} />
        </button>
      )}
    </form>
  );
}
```

- [ ] **Step 6: Rewrite InboxView as split-pane**

Replace `src/features/inbox/InboxView.tsx`:

```tsx
import { useMessages } from "@/hooks/useMessages";
import { useSearch } from "@/hooks/useSearch";
import MessageList from "@/components/MessageList";
import MessageDetail from "@/components/MessageDetail";
import SearchBar from "@/components/SearchBar";

export default function InboxView() {
  const { messages, loading, selectedMessageId, setSelectedMessage } =
    useMessages();
  const { results, loading: searchLoading, search, clear } = useSearch();

  const isSearching = results.length > 0;

  return (
    <div className="flex flex-col h-full">
      {/* Search toolbar */}
      <div
        className="px-4 py-2 border-b"
        style={{ borderColor: "var(--color-border)" }}
      >
        <SearchBar onSearch={search} onClear={clear} />
      </div>

      {/* Split pane */}
      <div className="flex flex-1 min-h-0">
        {/* Message list */}
        <div
          className="flex-shrink-0 overflow-hidden"
          style={{
            width: selectedMessageId ? "360px" : "100%",
            borderRight: selectedMessageId
              ? "1px solid var(--color-border)"
              : "none",
          }}
        >
          <MessageList
            messages={messages}
            selectedMessageId={selectedMessageId}
            onSelectMessage={(id) => setSelectedMessage(id)}
            loading={loading}
          />
        </div>

        {/* Message detail */}
        {selectedMessageId && (
          <div className="flex-1 min-w-0">
            <MessageDetail
              messageId={selectedMessageId}
              onBack={() => setSelectedMessage(null)}
            />
          </div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 7: Update Sidebar with folder list**

Replace `src/components/Sidebar.tsx`:

```tsx
import { useEffect } from "react";
import { Inbox, Kanban, Settings, ChevronLeft, ChevronRight, Folder, Send, FileEdit, Trash2, Archive, AlertTriangle } from "lucide-react";
import { useUIStore, ActiveView } from "@/stores/ui.store";
import { useMailStore } from "@/stores/mail.store";
import type { Folder as FolderType } from "@/lib/api";

const ROLE_ICONS: Record<string, React.ElementType> = {
  inbox: Inbox,
  sent: Send,
  drafts: FileEdit,
  trash: Trash2,
  archive: Archive,
  spam: AlertTriangle,
};

export default function Sidebar() {
  const { sidebarCollapsed, toggleSidebar, activeView, setActiveView } =
    useUIStore();
  const { folders, activeFolderId, setActiveFolder, accounts, activeAccountId, setActiveAccount, fetchAccounts } =
    useMailStore();

  useEffect(() => {
    fetchAccounts();
  }, [fetchAccounts]);

  // Auto-select first account
  useEffect(() => {
    if (accounts.length > 0 && !activeAccountId) {
      setActiveAccount(accounts[0].id);
    }
  }, [accounts, activeAccountId, setActiveAccount]);

  const handleFolderClick = (folder: FolderType) => {
    setActiveView("inbox");
    setActiveFolder(folder.id);
  };

  const navItems: { view: ActiveView; icon: React.ElementType; label: string }[] = [
    { view: "kanban", icon: Kanban, label: "Kanban" },
    { view: "settings", icon: Settings, label: "Settings" },
  ];

  return (
    <aside
      className="flex flex-col h-full select-none transition-all"
      style={{
        width: sidebarCollapsed ? "48px" : "220px",
        backgroundColor: "var(--color-sidebar-bg)",
        borderRight: "1px solid var(--color-border)",
      }}
    >
      {/* Collapse toggle */}
      <div className="flex items-center justify-end px-2 py-2">
        <button
          onClick={toggleSidebar}
          className="p-1 rounded hover:bg-black/5"
          aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {sidebarCollapsed ? (
            <ChevronRight size={16} />
          ) : (
            <ChevronLeft size={16} />
          )}
        </button>
      </div>

      {/* Folders */}
      {!sidebarCollapsed && (
        <div className="flex-1 overflow-y-auto px-2">
          <div className="mb-3">
            <span
              className="text-xs font-medium uppercase px-2"
              style={{ color: "var(--color-text-secondary)" }}
            >
              Folders
            </span>
            <div className="mt-1 space-y-0.5">
              {folders.map((folder) => {
                const Icon = ROLE_ICONS[folder.role ?? ""] ?? Folder;
                const isActive = activeFolderId === folder.id;
                return (
                  <button
                    key={folder.id}
                    onClick={() => handleFolderClick(folder)}
                    className="flex items-center gap-2 w-full px-2 py-1.5 rounded text-sm transition-colors"
                    style={{
                      backgroundColor: isActive
                        ? "var(--color-sidebar-active)"
                        : "transparent",
                      color: "var(--color-text-primary)",
                    }}
                  >
                    <Icon size={16} />
                    <span className="truncate">{folder.name}</span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Nav items */}
          <div className="border-t pt-2" style={{ borderColor: "var(--color-border)" }}>
            {navItems.map(({ view, icon: Icon, label }) => (
              <button
                key={view}
                onClick={() => setActiveView(view)}
                className="flex items-center gap-2 w-full px-2 py-1.5 rounded text-sm transition-colors"
                style={{
                  backgroundColor:
                    activeView === view
                      ? "var(--color-sidebar-active)"
                      : "transparent",
                  color: "var(--color-text-primary)",
                }}
              >
                <Icon size={16} />
                <span>{label}</span>
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Collapsed: just icons */}
      {sidebarCollapsed && (
        <div className="flex flex-col items-center gap-1 px-1">
          {folders
            .filter((f) => f.role)
            .map((folder) => {
              const Icon = ROLE_ICONS[folder.role ?? ""] ?? Folder;
              return (
                <button
                  key={folder.id}
                  onClick={() => handleFolderClick(folder)}
                  className="p-2 rounded hover:bg-black/5"
                  title={folder.name}
                >
                  <Icon size={18} />
                </button>
              );
            })}
          <div className="border-t w-full my-1" style={{ borderColor: "var(--color-border)" }} />
          {navItems.map(({ view, icon: Icon, label }) => (
            <button
              key={view}
              onClick={() => setActiveView(view)}
              className="p-2 rounded hover:bg-black/5"
              title={label}
            >
              <Icon size={18} />
            </button>
          ))}
        </div>
      )}
    </aside>
  );
}
```

- [ ] **Step 8: Run frontend tests and verify**

Run: `npx vitest run`
Expected: Existing UIStore tests still pass.

- [ ] **Step 9: Commit**

```bash
git add src/components/ src/features/ src/hooks/ src/stores/
git commit -m "feat(ui): add inbox UI with message list, detail view, privacy banner, search, and folder navigation"
```

---

### Task 7: Account setup dialog

**Files:**
- Create: `src/components/AccountSetup.tsx`
- Create: `src/features/settings/SettingsView.tsx`
- Modify: `src/app/Layout.tsx`

**Context:** Users need a way to add their first IMAP account. Build a simple settings view with an account add form. The form collects IMAP/SMTP server details, tests connection via `start_sync`, and shows accounts list.

- [ ] **Step 1: Create AccountSetup.tsx**

Create `src/components/AccountSetup.tsx`:

```tsx
import { useState } from "react";
import { addAccount, startSync, AddAccountRequest } from "@/lib/api";
import { useMailStore } from "@/stores/mail.store";
import { X } from "lucide-react";

interface Props {
  onClose: () => void;
}

const PRESETS: Record<string, Partial<AddAccountRequest>> = {
  gmail: {
    imap_host: "imap.gmail.com",
    imap_port: 993,
    smtp_host: "smtp.gmail.com",
    smtp_port: 587,
    use_tls: true,
  },
  outlook: {
    imap_host: "outlook.office365.com",
    imap_port: 993,
    smtp_host: "smtp.office365.com",
    smtp_port: 587,
    use_tls: true,
  },
  qq: {
    imap_host: "imap.qq.com",
    imap_port: 993,
    smtp_host: "smtp.qq.com",
    smtp_port: 465,
    use_tls: true,
  },
  "163": {
    imap_host: "imap.163.com",
    imap_port: 993,
    smtp_host: "smtp.163.com",
    smtp_port: 465,
    use_tls: true,
  },
};

export default function AccountSetup({ onClose }: Props) {
  const fetchAccounts = useMailStore((s) => s.fetchAccounts);
  const [form, setForm] = useState<AddAccountRequest>({
    email: "",
    display_name: "",
    provider: "imap",
    imap_host: "",
    imap_port: 993,
    smtp_host: "",
    smtp_port: 587,
    username: "",
    password: "",
    use_tls: true,
  });
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const applyPreset = (key: string) => {
    const preset = PRESETS[key];
    if (preset) {
      setForm((f) => ({ ...f, ...preset }));
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      // Fill username from email if empty
      const request = {
        ...form,
        username: form.username || form.email,
      };
      const account = await addAccount(request);
      await startSync(account.id);
      await fetchAccounts();
      onClose();
    } catch (err: any) {
      setError(err?.message || String(err));
    } finally {
      setLoading(false);
    }
  };

  const update = (key: keyof AddAccountRequest, value: any) =>
    setForm((f) => ({ ...f, [key]: value }));

  return (
    <div className="fixed inset-0 bg-black/30 flex items-center justify-center z-50">
      <div
        className="rounded-lg shadow-lg w-[480px] max-h-[90vh] overflow-auto"
        style={{ backgroundColor: "var(--color-bg)" }}
      >
        <div
          className="flex items-center justify-between px-4 py-3 border-b"
          style={{ borderColor: "var(--color-border)" }}
        >
          <h2
            className="text-base font-semibold"
            style={{ color: "var(--color-text-primary)" }}
          >
            Add Email Account
          </h2>
          <button onClick={onClose} className="p-1 rounded hover:bg-black/5">
            <X size={18} />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="p-4 space-y-4">
          {/* Quick presets */}
          <div className="flex gap-2">
            {Object.keys(PRESETS).map((key) => (
              <button
                key={key}
                type="button"
                onClick={() => applyPreset(key)}
                className="text-xs px-3 py-1 rounded border"
                style={{ borderColor: "var(--color-border)", color: "var(--color-text-secondary)" }}
              >
                {key.charAt(0).toUpperCase() + key.slice(1)}
              </button>
            ))}
          </div>

          <Input label="Email" value={form.email} onChange={(v) => update("email", v)} />
          <Input label="Display Name" value={form.display_name} onChange={(v) => update("display_name", v)} />
          <Input label="IMAP Host" value={form.imap_host} onChange={(v) => update("imap_host", v)} />
          <Input label="IMAP Port" value={String(form.imap_port)} onChange={(v) => update("imap_port", Number(v))} type="number" />
          <Input label="SMTP Host" value={form.smtp_host} onChange={(v) => update("smtp_host", v)} />
          <Input label="SMTP Port" value={String(form.smtp_port)} onChange={(v) => update("smtp_port", Number(v))} type="number" />
          <Input label="Username" value={form.username} onChange={(v) => update("username", v)} placeholder="defaults to email" />
          <Input label="Password / App Password" value={form.password} onChange={(v) => update("password", v)} type="password" />

          {error && (
            <div className="text-sm text-red-500 p-2 rounded" style={{ backgroundColor: "rgba(239,68,68,0.1)" }}>
              {error}
            </div>
          )}

          <button
            type="submit"
            disabled={loading || !form.email || !form.imap_host || !form.password}
            className="w-full py-2 rounded text-white text-sm font-medium disabled:opacity-50"
            style={{ backgroundColor: "var(--color-accent)" }}
          >
            {loading ? "Connecting..." : "Add Account & Sync"}
          </button>
        </form>
      </div>
    </div>
  );
}

function Input({
  label,
  value,
  onChange,
  type = "text",
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: string;
  placeholder?: string;
}) {
  return (
    <div>
      <label
        className="text-xs font-medium mb-1 block"
        style={{ color: "var(--color-text-secondary)" }}
      >
        {label}
      </label>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="w-full px-3 py-1.5 rounded text-sm border outline-none"
        style={{
          borderColor: "var(--color-border)",
          backgroundColor: "var(--color-bg)",
          color: "var(--color-text-primary)",
        }}
      />
    </div>
  );
}
```

- [ ] **Step 2: Create SettingsView.tsx**

Create `src/features/settings/SettingsView.tsx`:

```tsx
import { useState } from "react";
import { useMailStore } from "@/stores/mail.store";
import { deleteAccount } from "@/lib/api";
import AccountSetup from "@/components/AccountSetup";
import { Plus, Trash2, Mail } from "lucide-react";

export default function SettingsView() {
  const { accounts, fetchAccounts } = useMailStore();
  const [showAdd, setShowAdd] = useState(false);

  const handleDelete = async (id: string) => {
    await deleteAccount(id);
    await fetchAccounts();
  };

  return (
    <div className="p-6 max-w-2xl mx-auto">
      <h1
        className="text-lg font-semibold mb-4"
        style={{ color: "var(--color-text-primary)" }}
      >
        Settings
      </h1>

      {/* Accounts section */}
      <section className="mb-8">
        <div className="flex items-center justify-between mb-3">
          <h2
            className="text-sm font-medium"
            style={{ color: "var(--color-text-secondary)" }}
          >
            Email Accounts
          </h2>
          <button
            onClick={() => setShowAdd(true)}
            className="flex items-center gap-1 text-sm px-3 py-1 rounded"
            style={{ color: "var(--color-accent)" }}
          >
            <Plus size={14} /> Add Account
          </button>
        </div>

        {accounts.length === 0 ? (
          <div
            className="text-center py-12 rounded border border-dashed"
            style={{ borderColor: "var(--color-border)" }}
          >
            <Mail
              size={32}
              className="mx-auto mb-2"
              style={{ color: "var(--color-text-secondary)" }}
            />
            <p
              className="text-sm mb-3"
              style={{ color: "var(--color-text-secondary)" }}
            >
              No accounts configured
            </p>
            <button
              onClick={() => setShowAdd(true)}
              className="text-sm px-4 py-1.5 rounded text-white"
              style={{ backgroundColor: "var(--color-accent)" }}
            >
              Add your first account
            </button>
          </div>
        ) : (
          <div className="space-y-2">
            {accounts.map((account) => (
              <div
                key={account.id}
                className="flex items-center justify-between px-4 py-3 rounded border"
                style={{ borderColor: "var(--color-border)" }}
              >
                <div>
                  <div
                    className="text-sm font-medium"
                    style={{ color: "var(--color-text-primary)" }}
                  >
                    {account.display_name || account.email}
                  </div>
                  <div
                    className="text-xs"
                    style={{ color: "var(--color-text-secondary)" }}
                  >
                    {account.email} — {account.provider}
                  </div>
                </div>
                <button
                  onClick={() => handleDelete(account.id)}
                  className="p-1.5 rounded hover:bg-red-50"
                  title="Delete account"
                >
                  <Trash2 size={16} className="text-red-500" />
                </button>
              </div>
            ))}
          </div>
        )}
      </section>

      {showAdd && <AccountSetup onClose={() => setShowAdd(false)} />}
    </div>
  );
}
```

- [ ] **Step 3: Update Layout.tsx to route settings view**

Modify `src/app/Layout.tsx` to import and render SettingsView:

```tsx
import TitleBar from "@/components/TitleBar";
import Sidebar from "@/components/Sidebar";
import StatusBar from "@/components/StatusBar";
import InboxView from "@/features/inbox/InboxView";
import SettingsView from "@/features/settings/SettingsView";
import { useUIStore } from "@/stores/ui.store";

export default function Layout() {
  const activeView = useUIStore((s) => s.activeView);

  return (
    <div className="flex flex-col h-screen" style={{ backgroundColor: "var(--color-bg)" }}>
      <TitleBar />
      <div className="flex flex-1 min-h-0">
        <Sidebar />
        <main className="flex-1 min-w-0">
          {activeView === "inbox" && <InboxView />}
          {activeView === "settings" && <SettingsView />}
          {activeView === "kanban" && (
            <div className="flex items-center justify-center h-full">
              <span style={{ color: "var(--color-text-secondary)" }}>
                Kanban view — coming in Phase 3
              </span>
            </div>
          )}
        </main>
      </div>
      <StatusBar />
    </div>
  );
}
```

- [ ] **Step 4: Run tests**

Run: `npx vitest run`
Expected: Existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/AccountSetup.tsx src/features/settings/ src/app/Layout.tsx
git commit -m "feat(ui): add account setup dialog and settings view"
```

---

### Task 8: Full integration test — build + test suite

**Files:**
- Modify: `tests/stores/ui.store.test.ts` (verify still works)
- Create: `tests/stores/mail.store.test.ts`

**Context:** Run the full test suite (Rust + frontend) and verify the build compiles. Add basic frontend tests for the new MailStore.

- [ ] **Step 1: Write mail.store tests**

Create `tests/stores/mail.store.test.ts`:

```typescript
import { describe, it, expect, vi, beforeEach } from "vitest";
import { useMailStore } from "../src/stores/mail.store";

// Mock the Tauri invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const mockInvoke = vi.mocked(invoke);

describe("MailStore", () => {
  beforeEach(() => {
    // Reset store state
    useMailStore.setState({
      accounts: [],
      folders: [],
      messages: [],
      selectedMessageId: null,
      activeAccountId: null,
      activeFolderId: null,
      loadingMessages: false,
      loadingFolders: false,
    });
    vi.clearAllMocks();
  });

  it("should have correct initial state", () => {
    const state = useMailStore.getState();
    expect(state.accounts).toEqual([]);
    expect(state.folders).toEqual([]);
    expect(state.messages).toEqual([]);
    expect(state.selectedMessageId).toBeNull();
    expect(state.activeAccountId).toBeNull();
    expect(state.activeFolderId).toBeNull();
  });

  it("should fetch and set accounts", async () => {
    const mockAccounts = [
      { id: "a1", email: "test@example.com", display_name: "Test", provider: "imap", created_at: 0, updated_at: 0 },
    ];
    mockInvoke.mockResolvedValueOnce(mockAccounts);

    await useMailStore.getState().fetchAccounts();
    expect(useMailStore.getState().accounts).toEqual(mockAccounts);
  });

  it("should set selected message", () => {
    useMailStore.getState().setSelectedMessage("msg-1");
    expect(useMailStore.getState().selectedMessageId).toBe("msg-1");

    useMailStore.getState().setSelectedMessage(null);
    expect(useMailStore.getState().selectedMessageId).toBeNull();
  });

  it("should fetch folders sorted by sort_order", async () => {
    const mockFolders = [
      { id: "f2", name: "Sent", sort_order: 2, role: "sent" },
      { id: "f1", name: "Inbox", sort_order: 0, role: "inbox" },
    ];
    mockInvoke.mockResolvedValueOnce(mockFolders);

    await useMailStore.getState().fetchFolders("a1");
    const folders = useMailStore.getState().folders;
    expect(folders[0].name).toBe("Inbox");
    expect(folders[1].name).toBe("Sent");
  });
});
```

- [ ] **Step 2: Run all tests**

Run in parallel:
```bash
cargo test --workspace
npx vitest run
cargo clippy --workspace -- -D warnings
```

Expected: All Rust tests pass, all frontend tests pass, clippy clean.

- [ ] **Step 3: Commit**

```bash
git add tests/
git commit -m "test: add MailStore unit tests and verify full test suite"
```

---

## Summary

| Task | Description | Estimated complexity |
|------|-------------|---------------------|
| 1 | pebble-mail: IMAP provider, MIME parser, sync, threading | High |
| 2 | pebble-search: Tantivy full-text search | Medium |
| 3 | Replace HTML sanitizer with ammonia | Medium |
| 4 | Tauri IPC commands (accounts, messages, folders, search, sync) | Medium |
| 5 | Frontend: MailStore + API layer + hooks | Medium |
| 6 | Frontend: Inbox UI components (list, detail, privacy, search) | High |
| 7 | Account setup dialog + Settings view | Medium |
| 8 | Full integration test suite | Low |

**Total:** 8 tasks, ~65 steps
