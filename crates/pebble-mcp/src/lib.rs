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
    json!([
        {
            "name": protocol::TOOL_SEARCH,
            "title": "Search mail",
            "description": "Search the user's mail in Pebble (only accounts the user allowed). Returns sender, subject, date, folder and ids, plus snippets when allowed. Spam and trash are left out. The `coverage` field says which accounts were searched and how far back mail is synced: when nothing matches, say what was searched rather than claiming the mail doesn't exist.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words to find in the subject or text"},
                    "from": {"type": "string", "description": "Sender name or address"},
                    "to": {"type": "string"},
                    "subject": {"type": "string"},
                    "after": {"type": "string", "description": "YYYY-MM-DD, inclusive"},
                    "before": {"type": "string", "description": "YYYY-MM-DD, exclusive"},
                    "account": {"type": "string", "description": "Limit to one account (its address)"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50},
                    "cursor": {"type": "string", "description": "next_cursor from the previous page"}
                }
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": protocol::TOOL_GET_THREAD,
            "title": "Read a thread",
            "description": "Read the conversation a message belongs to, as plain text: quoted history removed, one-time codes masked, attachments listed by name and size only. Needs the user's \"full text\" permission for that account.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "description": "A message_id from mail_search"}
                },
                "required": ["message_id"]
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": protocol::TOOL_PROPOSE_DRAFT,
            "title": "Propose a draft",
            "description": "Propose an email for the user to review in Pebble. Nothing is sent: the proposal appears in Pebble's assistant, where the user can open it in the composer, edit it and decide. For a reply give reply_to_message_id (recipients come from that message); for new mail give to and subject. Needs the user's \"drafts\" permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reply_to_message_id": {"type": "string"},
                    "to": {"type": "array", "items": {"type": "string"}},
                    "cc": {"type": "array", "items": {"type": "string"}},
                    "subject": {"type": "string"},
                    "body": {"type": "string", "description": "Plain text"}
                },
                "required": ["body"]
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}
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
