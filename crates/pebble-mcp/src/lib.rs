//! Pebble's local MCP bridge (#107, M4).
//!
//! An MCP client (Claude Desktop, Cursor…) starts `Pebble --mcp
//! --endpoint <socket>` and talks MCP over stdio. The bridge never opens
//! the mailbox: it forwards each tool call to the running Pebble app over a
//! local socket (a named pipe on Windows), where pairing, grants, rate
//! limits and the audit log are enforced. The token Pebble issues on
//! pairing is kept in the system keychain.

pub mod protocol;
pub mod server;
pub mod transport;

use serde_json::{json, Value};

pub use server::{run_stdio, Bridge};
pub use transport::{Connector, KeyringTokens, SocketConnector, TokenStore};

/// The tools Pebble offers. What each one may return depends on the
/// client's grant, which Pebble checks on every call.
pub fn tool_definitions() -> Value {
    let read_only = json!({"readOnlyHint": true, "openWorldHint": false});
    json!([
        {
            "name": protocol::TOOL_LIST_ACCOUNTS,
            "title": "List accounts",
            "description": "The mail accounts the user allowed this app to use, the access level for each (metadata, summary, body, drafts), which tools work there, and how far back mail is synced. Call this first.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": read_only
        },
        {
            "name": protocol::TOOL_SEARCH,
            "title": "Search mail",
            "description": "Search the user's mail in Pebble (allowed accounts only), newest first. Returns sender, subject, date, folders and ids, plus snippets when allowed. Spam, trash and drafts are left out. Give at least a query, sender, subject or date; for what's new use mail_list_recent. `coverage` says which accounts were searched and how far back mail is synced: when nothing matches, say what was searched rather than claiming the mail doesn't exist.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words to find in the subject or text"},
                    "from": {"type": "string", "description": "Sender name or address"},
                    "to": {"type": "string"},
                    "subject": {"type": "string"},
                    "after": {"type": "string", "description": "YYYY-MM-DD, inclusive"},
                    "before": {"type": "string", "description": "YYYY-MM-DD, exclusive"},
                    "unread": {"type": "boolean"},
                    "starred": {"type": "boolean"},
                    "has_attachment": {"type": "boolean"},
                    "folder": {"type": "string", "enum": ["inbox", "sent", "archive"]},
                    "account": {"type": "string", "description": "Limit to one account (its address)"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50},
                    "cursor": {"type": "string", "description": "next_cursor from the previous page"}
                }
            },
            "annotations": read_only
        },
        {
            "name": protocol::TOOL_LIST_RECENT,
            "title": "Recent mail",
            "description": "Recent mail in the inbox, newest first (\"what's new\", \"anything unread today\").",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "days": {"type": "integer", "minimum": 1, "maximum": 14, "description": "How far back (default 2)"},
                    "unread_only": {"type": "boolean"},
                    "account": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 30}
                }
            },
            "annotations": read_only
        },
        {
            "name": protocol::TOOL_GET_THREAD,
            "title": "Read a thread",
            "description": "Read the conversation a message belongs to, as plain text: quoted history removed, one-time codes masked, attachments listed by name and size only, spam and trash left out. Needs the user's \"full text\" permission for that account.",
            "inputSchema": {
                "type": "object",
                "properties": {"message_id": {"type": "string", "description": "A message_id from a search or listing"}},
                "required": ["message_id"]
            },
            "annotations": read_only
        },
        {
            "name": protocol::TOOL_GET_MESSAGE,
            "title": "Read a message",
            "description": "Read one message as plain text (same rules as mail_get_thread). Needs \"full text\" permission.",
            "inputSchema": {
                "type": "object",
                "properties": {"message_id": {"type": "string"}},
                "required": ["message_id"]
            },
            "annotations": read_only
        },
        {
            "name": protocol::TOOL_LIST_OPEN_ITEMS,
            "title": "Open items",
            "description": "What Pebble is keeping track of for the user: threads it follows (waiting for someone's reply) and mail that seems to be waiting for the user's reply.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": read_only
        },
        {
            "name": protocol::TOOL_PROPOSE_DRAFT,
            "title": "Propose a draft",
            "description": "Propose an email for the user to review in Pebble. Nothing is sent: the proposal appears in Pebble's assistant, where the user can open it in the composer, edit it and decide. For a reply give reply_to_message_id (recipients come from that message); for new mail give to and subject. Returns a proposal_id for mail_proposal_status. Needs the user's \"drafts\" permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reply_to_message_id": {"type": "string"},
                    "to": {"type": "array", "items": {"type": "string"}},
                    "cc": {"type": "array", "items": {"type": "string"}},
                    "subject": {"type": "string"},
                    "body": {"type": "string", "description": "Plain text"},
                    "account": {"type": "string", "description": "Account to write from (its address), for new mail"}
                },
                "required": ["body"]
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}
        },
        {
            "name": protocol::TOOL_PROPOSAL_STATUS,
            "title": "Proposal status",
            "description": "Whether the user has opened a proposed draft in the composer, dismissed it, or not looked yet.",
            "inputSchema": {
                "type": "object",
                "properties": {"proposal_id": {"type": "string"}},
                "required": ["proposal_id"]
            },
            "annotations": read_only
        },
        {
            "name": protocol::TOOL_FOLLOW_UP,
            "title": "Follow up on a thread",
            "description": "Ask Pebble to watch a thread and remind the user if nobody replies (after `days`, or just tell them when a reply comes). Doesn't replace a follow-up the user already has. Needs the user's \"drafts\" permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string"},
                    "days": {"type": "integer", "minimum": 1, "maximum": 60}
                },
                "required": ["message_id"]
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        }
    ])
}

/// Entry point for `--mcp`: parse the arguments and serve stdio until the
/// client closes it. Returns the process exit code.
pub fn cli_main(args: &[String]) -> i32 {
    let endpoint = args
        .windows(2)
        .find(|pair| pair[0] == "--endpoint")
        .map(|pair| pair[1].clone());
    let Some(endpoint) = endpoint else {
        eprintln!("pebble-mcp: missing --endpoint <socket>. Copy the command from Pebble → Settings → Connections.");
        return 2;
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("pebble-mcp: {e}");
            return 1;
        }
    };
    runtime.block_on(async move {
        let bridge = Bridge::new(
            SocketConnector::new(endpoint.clone()),
            KeyringTokens::new(endpoint),
        );
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        match run_stdio(bridge, stdin, stdout).await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("pebble-mcp: {e}");
                1
            }
        }
    })
}
