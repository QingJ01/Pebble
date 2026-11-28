//! The MCP tools, run with a paired client's grant (#107, M4).
//!
//! Every call is checked here, not in the bridge: the client must hold a
//! live token, the tool must exist, and only accounts granted at the level
//! the tool needs are touched. Gmail accounts are never reachable.

use std::collections::{BTreeMap, HashMap, HashSet};

use pebble_core::{now_timestamp, FolderRole, Message, ProviderType};
use pebble_mcp::protocol::{ErrorCode, Level, TOOL_GET_THREAD, TOOL_PROPOSE_DRAFT, TOOL_SEARCH};
use pebble_notify::triage_model::redact_codes;
use pebble_search::{AdvancedSearchParams, TantivySearch};
use pebble_store::agent::NewAgentEvent;
use pebble_store::mcp::McpClient;
use pebble_store::Store;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::capabilities::{Capabilities, DraftArgs};

pub const MAX_PAGE: usize = 50;
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

fn local_time(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn parse_day(value: &str) -> Option<i64> {
    let date = chrono::NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()?;
    crate::deadlines::local_ts(date.and_hms_opt(0, 0, 0)?)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SearchArgs {
    query: Option<String>,
    from: Option<String>,
    to: Option<String>,
    subject: Option<String>,
    after: Option<String>,
    before: Option<String>,
    account: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadArgs {
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

fn invalid(e: impl std::fmt::Display) -> ToolError {
    (ErrorCode::Invalid, format!("Invalid arguments: {e}"))
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
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        TOOL_SEARCH => search_mail(
            store,
            search,
            &granted,
            serde_json::from_value(args).map_err(invalid)?,
        ),
        TOOL_GET_THREAD => get_thread(
            store,
            search,
            &granted,
            serde_json::from_value(args).map_err(invalid)?,
        ),
        TOOL_PROPOSE_DRAFT => propose_draft(
            store,
            search,
            client,
            &granted,
            serde_json::from_value(args).map_err(invalid)?,
        ),
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

fn search_mail(
    store: &Store,
    search: &TantivySearch,
    granted: &[Granted],
    args: SearchArgs,
) -> Result<ToolResult, ToolError> {
    let accounts = pick_account(granted, args.account.as_deref())?;
    let levels: HashMap<&str, &Granted> = accounts.iter().map(|g| (g.id.as_str(), *g)).collect();
    let limit = args.limit.unwrap_or(20).clamp(1, MAX_PAGE);
    let offset: usize = args
        .cursor
        .as_deref()
        .map(|c| c.parse().map_err(|_| invalid("cursor")))
        .transpose()?
        .unwrap_or(0)
        .min(1_000);
    // Over-fetch: other accounts, spam and trash are dropped below.
    let hits = search
        .advanced_search(AdvancedSearchParams {
            text: args.query.as_deref().filter(|q| !q.trim().is_empty()),
            from: args.from.as_deref(),
            to: args.to.as_deref(),
            subject: args.subject.as_deref(),
            date_from: args.after.as_deref().and_then(parse_day),
            date_to: args.before.as_deref().and_then(parse_day),
            has_attachment: None,
            folder_id: None,
            limit: ((offset + limit) * 4).min(2_000),
        })
        .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
    let ids: Vec<String> = hits.iter().map(|h| h.message_id.clone()).collect();
    let messages: HashMap<String, Message> = store
        .get_messages_batch(&ids)
        .map_err(|e| (ErrorCode::Internal, e.to_string()))?
        .into_iter()
        .map(|m| (m.id.clone(), m))
        .collect();
    let folder_ids = store
        .get_message_folder_ids_batch(&ids)
        .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
    let mut folders: HashMap<String, (String, Option<FolderRole>)> = HashMap::new();
    for account in &accounts {
        for folder in store.list_folders(&account.id).unwrap_or_default() {
            folders.insert(
                folder.id.clone(),
                (folder.name.clone(), folder.role.clone()),
            );
        }
    }
    let mut matching = Vec::new();
    for id in &ids {
        let Some(message) = messages.get(id) else {
            continue;
        };
        if message.is_deleted
            || message.is_draft
            || !levels.contains_key(message.account_id.as_str())
        {
            continue;
        }
        let names: Vec<(String, Option<FolderRole>)> = folder_ids
            .get(id)
            .map(|ids| ids.iter().filter_map(|f| folders.get(f).cloned()).collect())
            .unwrap_or_default();
        if names
            .iter()
            .any(|(_, role)| matches!(role, Some(FolderRole::Spam | FolderRole::Trash)))
        {
            continue;
        }
        matching.push((message, names));
    }
    let total_seen = matching.len();
    let page: Vec<Value> = matching
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(message, names)| {
            let grant = levels[message.account_id.as_str()];
            let mut item = json!({
                "message_id": message.id,
                "thread_id": message.thread_id,
                "account": grant.email,
                "from": message.from_name,
                "from_address": message.from_address,
                "subject": redact_codes(&message.subject),
                "date": local_time(message.date),
                "folders": names.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>(),
                "unread": !message.is_read,
                "has_attachments": message.has_attachments,
            });
            if grant.level >= Level::Summary {
                item["snippet"] = json!(truncate(&redact_codes(&message.snippet), SNIPPET_CHARS));
            }
            item
        })
        .collect();
    let next = offset + page.len();
    let coverage: Vec<Value> = accounts
        .iter()
        .map(|g| {
            json!({
                "account": g.email,
                "synced_since": store
                    .earliest_message_date(&g.id)
                    .ok()
                    .flatten()
                    .map(local_time),
                "snippets": g.level >= Level::Summary,
            })
        })
        .collect();
    let touched: Vec<String> = accounts.iter().map(|g| g.id.clone()).collect();
    let count = page.len() as u32;
    Ok(ToolResult {
        value: json!({
            "results": page,
            "next_cursor": (next < total_seen).then(|| next.to_string()),
            "coverage": {
                "accounts": coverage,
                "excluded": ["spam", "trash"],
                "note": "Only mail synced to this device and only the accounts the user allowed. No match doesn't mean the mail doesn't exist.",
            },
        }),
        accounts: touched,
        results: count,
    })
}

fn scope_of(granted: &[&Granted]) -> HashMap<String, String> {
    granted
        .iter()
        .map(|g| (g.id.clone(), g.email.clone()))
        .collect()
}

fn get_thread(
    store: &Store,
    search: &TantivySearch,
    granted: &[Granted],
    args: ThreadArgs,
) -> Result<ToolResult, ToolError> {
    let all: Vec<&Granted> = granted.iter().collect();
    let mut caps = Capabilities::new(store, search, scope_of(&all));
    let mut value = caps
        .read_thread(&args.message_id)
        .map_err(|e| (ErrorCode::NotFound, e))?;
    // Attachments: names and sizes only.
    if let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            let Some(id) = message
                .get("message_id")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
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
    }
    let accounts: Vec<String> = caps
        .used
        .iter()
        .map(|m| m.account_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let count = caps.used.len() as u32;
    Ok(ToolResult {
        value,
        accounts,
        results: count,
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
        payload: json!({ "client": client.name, "draft": draft }),
        push: true,
        ..Default::default()
    };
    store
        .insert_agent_event(&event, now)
        .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
    Ok(ToolResult {
        value: json!({
            "status": "proposed",
            "note": "The draft is waiting for the user in Pebble's assistant. It has NOT been sent; the user reviews and sends it themselves.",
            "to": draft.to,
            "subject": draft.subject,
        }),
        accounts: vec![draft.account_id],
        results: 1,
    })
}

#[cfg(test)]
mod tests;
