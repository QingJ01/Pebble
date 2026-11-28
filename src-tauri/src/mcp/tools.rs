//! The MCP tools, run with a paired client's grant (#107, M4).
//!
//! Every call is checked here, not in the bridge: the client must hold a
//! live token, the tool must exist, and only accounts granted at the level
//! the tool needs are touched. Gmail accounts are never reachable.

use std::collections::{BTreeMap, HashMap, HashSet};

use pebble_core::{now_timestamp, ProviderType};
use pebble_mcp::protocol::{
    ErrorCode, Level, TOOL_FOLLOW_UP, TOOL_GET_MESSAGE, TOOL_GET_THREAD, TOOL_LIST_ACCOUNTS,
    TOOL_LIST_OPEN_ITEMS, TOOL_LIST_RECENT, TOOL_PROPOSAL_STATUS, TOOL_PROPOSE_DRAFT, TOOL_SEARCH,
};
use pebble_notify::triage_model::redact_codes;
use pebble_search::TantivySearch;
use pebble_store::agent::NewAgentEvent;
use pebble_store::mcp::McpClient;
use pebble_store::Store;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::capabilities::{local_day, Capabilities, DraftArgs, FollowArgs, RecentArgs};
use crate::agent::mail_search::{self, MailQuery};

const SNIPPET_CHARS: usize = 200;

/// What a successful call touched (for the audit log).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub value: Value,
    pub accounts: Vec<String>,
    pub results: u32,
}

pub type ToolError = (ErrorCode, String);

/// An account a client may use, with its level.
#[derive(Debug, Clone)]
pub struct Granted {
    pub id: String,
    pub email: String,
    pub level: Level,
}

/// The accounts in a client's grant that still exist and may be granted
/// (never Gmail), with valid levels.
pub fn granted_accounts(store: &Store, client: &McpClient) -> Vec<Granted> {
    let accounts = store.list_accounts().unwrap_or_default();
    accounts
        .into_iter()
        .filter(|account| account.provider != ProviderType::Gmail)
        .filter_map(|account| {
            let level = client
                .grants
                .get(&account.id)
                .and_then(|l| Level::parse(l))?;
            Some(Granted {
                id: account.id,
                email: account.email,
                level,
            })
        })
        .collect()
}

/// Keep only grant entries for accounts that may be granted, at valid
/// levels (used when saving grants).
pub fn sanitize_grants(
    store: &Store,
    grants: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let allowed: HashSet<String> = store
        .list_accounts()
        .unwrap_or_default()
        .into_iter()
        .filter(|account| account.provider != ProviderType::Gmail)
        .map(|account| account.id)
        .collect();
    grants
        .iter()
        .filter(|(id, level)| allowed.contains(*id) && Level::parse(level).is_some())
        .map(|(id, level)| (id.clone(), level.clone()))
        .collect()
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Metadata => "search",
        Level::Summary => "search with snippets",
        Level::Body => "full text",
        Level::Drafts => "draft proposals",
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SearchArgs {
    #[serde(flatten)]
    query: MailQuery,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RecentToolArgs {
    days: Option<u32>,
    unread_only: Option<bool>,
    account: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct MessageArgs {
    message_id: String,
}

#[derive(Debug, Deserialize)]
struct ProposeArgs {
    reply_to_message_id: Option<String>,
    to: Option<Vec<String>>,
    cc: Option<Vec<String>>,
    subject: Option<String>,
    body: String,
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FollowToolArgs {
    message_id: String,
    days: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct StatusArgs {
    proposal_id: String,
}

fn invalid(e: impl std::fmt::Display) -> ToolError {
    (ErrorCode::Invalid, format!("Invalid arguments: {e}"))
}

fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, ToolError> {
    serde_json::from_value(if args.is_null() { json!({}) } else { args }).map_err(invalid)
}

/// Run one tool for `client`.
pub fn run(
    store: &Store,
    search: &TantivySearch,
    client: &McpClient,
    tool: &str,
    args: Value,
) -> Result<ToolResult, ToolError> {
    let Some(needed) = Level::required_for(tool) else {
        return Err((
            ErrorCode::Invalid,
            format!("Pebble has no tool named {tool}"),
        ));
    };
    let granted: Vec<Granted> = granted_accounts(store, client)
        .into_iter()
        .filter(|g| g.level >= needed)
        .collect();
    if granted.is_empty() {
        return Err((
            ErrorCode::Forbidden,
            format!(
                "The user hasn't allowed {} for any account. They can change this in Pebble → Settings → Connections.",
                level_name(needed)
            ),
        ));
    }
    match tool {
        TOOL_LIST_ACCOUNTS => list_accounts(store, client),
        TOOL_SEARCH => search_mail(store, search, &granted, parse(args)?),
        TOOL_LIST_RECENT => list_recent(store, search, &granted, parse(args)?),
        TOOL_GET_THREAD => get_thread(store, search, &granted, parse(args)?),
        TOOL_GET_MESSAGE => get_message(store, search, &granted, parse(args)?),
        TOOL_LIST_OPEN_ITEMS => list_open_items(store, search, &granted),
        TOOL_PROPOSE_DRAFT => propose_draft(store, search, client, &granted, parse(args)?),
        TOOL_FOLLOW_UP => follow_up(store, search, &granted, parse(args)?),
        TOOL_PROPOSAL_STATUS => proposal_status(store, client, parse(args)?),
        _ => Err((
            ErrorCode::Invalid,
            format!("Pebble has no tool named {tool}"),
        )),
    }
}

/// Narrow `granted` to the account named by address, if any.
fn pick_account<'a>(
    granted: &'a [Granted],
    account: Option<&str>,
) -> Result<Vec<&'a Granted>, ToolError> {
    match account.map(str::trim).filter(|a| !a.is_empty()) {
        None => Ok(granted.iter().collect()),
        Some(address) => {
            let found: Vec<&Granted> = granted
                .iter()
                .filter(|g| g.email.eq_ignore_ascii_case(address) || g.id == address)
                .collect();
            if found.is_empty() {
                Err((
                    ErrorCode::Forbidden,
                    format!("The account {address} isn't one this client may use."),
                ))
            } else {
                Ok(found)
            }
        }
    }
}

fn scope_of(granted: &[&Granted]) -> HashMap<String, String> {
    granted
        .iter()
        .map(|g| (g.id.clone(), g.email.clone()))
        .collect()
}

fn pairs_of(granted: &[&Granted]) -> Vec<(String, String)> {
    granted
        .iter()
        .map(|g| (g.id.clone(), g.email.clone()))
        .collect()
}

fn level_code(level: Level) -> &'static str {
    match level {
        Level::Metadata => "metadata",
        Level::Summary => "summary",
        Level::Body => "body",
        Level::Drafts => "drafts",
    }
}

/// A list entry for one message, with a snippet only at summary level.
fn listing(message: &pebble_core::Message, folders: &[String], grant: &Granted) -> Value {
    let mut item = json!({
        "message_id": message.id,
        "thread_id": message.thread_id,
        "account": grant.email,
        "from": message.from_name,
        "from_address": message.from_address,
        "subject": redact_codes(&message.subject),
        "date": local_day(message.date),
        "folders": folders,
        "unread": !message.is_read,
        "starred": message.is_starred,
        "has_attachments": message.has_attachments,
    });
    if grant.level >= Level::Summary {
        item["snippet"] = json!(truncate(&redact_codes(&message.snippet), SNIPPET_CHARS));
    }
    item
}

/// The accounts this client may use, its level for each, and how far back
/// mail is synced. Lets a client know what it can ask before asking.
fn list_accounts(store: &Store, client: &McpClient) -> Result<ToolResult, ToolError> {
    let granted = granted_accounts(store, client);
    let accounts: Vec<Value> = granted
        .iter()
        .map(|g| {
            let mut tools = vec!["mail_search", "mail_list_recent", "mail_list_open_items"];
            if g.level >= Level::Body {
                tools.extend(["mail_get_thread", "mail_get_message"]);
            }
            if g.level >= Level::Drafts {
                tools.extend([
                    "mail_propose_draft",
                    "mail_follow_up",
                    "mail_proposal_status",
                ]);
            }
            json!({
                "account": g.email,
                "level": level_code(g.level),
                "snippets": g.level >= Level::Summary,
                "tools": tools,
                "synced_since": store.earliest_message_date(&g.id).ok().flatten().map(local_day),
            })
        })
        .collect();
    let count = accounts.len() as u32;
    Ok(ToolResult {
        value: json!({ "accounts": accounts }),
        accounts: granted.iter().map(|g| g.id.clone()).collect(),
        results: count,
    })
}

fn search_mail(
    store: &Store,
    search: &TantivySearch,
    granted: &[Granted],
    args: SearchArgs,
) -> Result<ToolResult, ToolError> {
    let accounts = pick_account(granted, args.account.as_deref())?;
    let by_id: HashMap<&str, &Granted> = accounts.iter().map(|g| (g.id.as_str(), *g)).collect();
    let ids: Vec<String> = accounts.iter().map(|g| g.id.clone()).collect();
    let page = mail_search::search(store, search, &ids, &args.query)
        .map_err(|e| (ErrorCode::Invalid, e))?;
    let results: Vec<Value> = page
        .items
        .iter()
        .filter_map(|f| {
            by_id
                .get(f.message.account_id.as_str())
                .map(|g| listing(&f.message, &f.folders, g))
        })
        .collect();
    let count = results.len() as u32;
    Ok(ToolResult {
        value: json!({
            "results": results,
            "next_cursor": page.next_cursor,
            "coverage": mail_search::coverage(store, &pairs_of(&accounts)),
        }),
        accounts: ids,
        results: count,
    })
}

/// Recent inbox mail ("what's new"), newest first.
fn list_recent(
    store: &Store,
    search: &TantivySearch,
    granted: &[Granted],
    args: RecentToolArgs,
) -> Result<ToolResult, ToolError> {
    let accounts = pick_account(granted, args.account.as_deref())?;
    let by_id: HashMap<&str, &Granted> = accounts.iter().map(|g| (g.id.as_str(), *g)).collect();
    let mut caps = Capabilities::new(store, search, scope_of(&accounts));
    let value = caps
        .list_recent_mail(RecentArgs {
            days: args.days,
            unread_only: args.unread_only,
            limit: Some(args.limit.unwrap_or(20).clamp(1, 30) as u32),
        })
        .map_err(|e| (ErrorCode::Internal, e))?;
    let ids: Vec<String> = value["results"]
        .as_array()
        .map(|r| {
            r.iter()
                .filter_map(|m| m["message_id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let messages = store
        .get_messages_batch(&ids)
        .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
    let messages = mail_search::outside_spam_and_trash(store, &messages);
    let mut results: Vec<(i64, Value)> = messages
        .iter()
        .filter_map(|m| {
            by_id
                .get(m.account_id.as_str())
                .map(|g| (m.date, listing(m, &["Inbox".to_string()], g)))
        })
        .collect();
    results.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
    let results: Vec<Value> = results.into_iter().map(|(_, v)| v).collect();
    let count = results.len() as u32;
    Ok(ToolResult {
        value: json!({
            "results": results,
            "coverage": mail_search::coverage(store, &pairs_of(&accounts)),
        }),
        accounts: accounts.iter().map(|g| g.id.clone()).collect(),
        results: count,
    })
}

/// Attachments by name, size and type only.
fn add_attachments(store: &Store, message: &mut Value) {
    let Some(id) = message
        .get("message_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let attachments: Vec<Value> = store
        .list_attachments_by_message(&id)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| !a.is_inline)
        .map(|a| json!({"name": a.filename, "size": a.size, "type": a.mime_type}))
        .collect();
    message["attachments"] = json!(attachments);
}

fn used_accounts(caps: &Capabilities<'_>) -> Vec<String> {
    caps.used
        .iter()
        .map(|m| m.account_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

fn get_thread(
    store: &Store,
    search: &TantivySearch,
    granted: &[Granted],
    args: MessageArgs,
) -> Result<ToolResult, ToolError> {
    let all: Vec<&Granted> = granted.iter().collect();
    let mut caps = Capabilities::new(store, search, scope_of(&all));
    let mut value = caps
        .read_thread(&args.message_id)
        .map_err(|e| (ErrorCode::NotFound, e))?;
    if let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            add_attachments(store, message);
        }
    }
    Ok(ToolResult {
        value,
        accounts: used_accounts(&caps),
        results: caps.used.len() as u32,
    })
}

fn get_message(
    store: &Store,
    search: &TantivySearch,
    granted: &[Granted],
    args: MessageArgs,
) -> Result<ToolResult, ToolError> {
    let all: Vec<&Granted> = granted.iter().collect();
    let mut caps = Capabilities::new(store, search, scope_of(&all));
    let mut value = caps
        .read_message(&args.message_id)
        .map_err(|e| (ErrorCode::NotFound, e))?;
    add_attachments(store, &mut value);
    Ok(ToolResult {
        value,
        accounts: used_accounts(&caps),
        results: 1,
    })
}

fn list_open_items(
    store: &Store,
    search: &TantivySearch,
    granted: &[Granted],
) -> Result<ToolResult, ToolError> {
    let all: Vec<&Granted> = granted.iter().collect();
    let mut caps = Capabilities::new(store, search, scope_of(&all));
    let value = caps
        .list_open_items()
        .map_err(|e| (ErrorCode::Internal, e))?;
    let count = value["following"].as_array().map_or(0, Vec::len)
        + value["waiting_for_your_reply"]
            .as_array()
            .map_or(0, Vec::len);
    Ok(ToolResult {
        value,
        accounts: granted.iter().map(|g| g.id.clone()).collect(),
        results: count as u32,
    })
}

fn follow_up(
    store: &Store,
    search: &TantivySearch,
    granted: &[Granted],
    args: FollowToolArgs,
) -> Result<ToolResult, ToolError> {
    let all: Vec<&Granted> = granted.iter().collect();
    let mut caps = Capabilities::new(store, search, scope_of(&all));
    let value = caps
        .follow_up(FollowArgs {
            message_id: args.message_id,
            days: args.days,
        })
        .map_err(|e| (ErrorCode::NotFound, e))?;
    let accounts = caps
        .follow_ups
        .iter()
        .map(|f| f.account_id.clone())
        .collect();
    Ok(ToolResult {
        value,
        accounts,
        results: 1,
    })
}

fn propose_draft(
    store: &Store,
    search: &TantivySearch,
    client: &McpClient,
    granted: &[Granted],
    args: ProposeArgs,
) -> Result<ToolResult, ToolError> {
    let accounts = pick_account(granted, args.account.as_deref())?;
    let mut caps = Capabilities::new(store, search, scope_of(&accounts));
    caps.draft_email(DraftArgs {
        reply_to_message_id: args.reply_to_message_id.clone(),
        to: args.to,
        cc: args.cc,
        subject: args.subject,
        body: args.body,
    })
    .map_err(|e| (ErrorCode::Invalid, e))?;
    let draft = caps
        .drafts
        .pop()
        .ok_or_else(|| (ErrorCode::Internal, "No draft".to_string()))?;
    let now = now_timestamp();
    let event = NewAgentEvent {
        kind: "draft_proposal".into(),
        dedupe_key: format!("draft_proposal:{}", pebble_core::new_id()),
        account_id: Some(draft.account_id.clone()),
        message_id: draft.reply_to_message_id.clone(),
        payload: json!({ "client": client.name, "clientId": client.id, "draft": draft }),
        push: true,
        ..Default::default()
    };
    let created = store
        .insert_agent_event(&event, now)
        .map_err(|e| (ErrorCode::Internal, e.to_string()))?
        .ok_or_else(|| {
            (
                ErrorCode::Internal,
                "The proposal wasn't stored".to_string(),
            )
        })?;
    Ok(ToolResult {
        value: json!({
            "status": "proposed",
            "proposal_id": created.id,
            "note": "The draft is waiting for the user in Pebble's assistant. It has NOT been sent; the user reviews and sends it themselves. Check mail_proposal_status later.",
            "to": draft.to,
            "subject": redact_codes(&draft.subject),
        }),
        accounts: vec![draft.account_id],
        results: 1,
    })
}

/// Whether the user opened or dismissed a proposal this client made.
fn proposal_status(
    store: &Store,
    client: &McpClient,
    args: StatusArgs,
) -> Result<ToolResult, ToolError> {
    let event = store
        .list_agent_events(1_000, None)
        .map_err(|e| (ErrorCode::Internal, e.to_string()))?
        .into_iter()
        .find(|e| {
            e.id == args.proposal_id
                && e.kind == "draft_proposal"
                && e.payload["clientId"].as_str() == Some(client.id.as_str())
        })
        .ok_or_else(|| {
            (
                ErrorCode::NotFound,
                "No such proposal from this client".to_string(),
            )
        })?;
    let status = match event.resolution.as_ref().and_then(|r| r["action"].as_str()) {
        Some("opened") => "opened_in_composer",
        Some("dismissed") => "dismissed",
        _ => "waiting_for_the_user",
    };
    Ok(ToolResult {
        value: json!({
            "proposal_id": event.id,
            "status": status,
            "note": "Pebble can't tell whether an opened draft was sent; the user decides in the composer.",
        }),
        accounts: event.account_id.into_iter().collect(),
        results: 1,
    })
}

#[cfg(test)]
mod tests;
