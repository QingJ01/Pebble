//! One search for every caller that reads mail on the user's behalf: the
//! assistant's chat and local MCP clients (#107). Keeping it in one place
//! keeps the rules the same everywhere:
//!
//! - only the given accounts, filtered in the index (not after the fact);
//! - spam and trash are left out (the likeliest place for instructions
//!   aimed at a model), as are drafts and deleted mail;
//! - newest first, with a stable cursor (date + id), so mail arriving
//!   between pages doesn't shift them;
//! - every result says what was covered.

use std::collections::HashMap;

use pebble_core::{FolderRole, Message};
use pebble_search::{AdvancedSearchParams, TantivySearch};
use pebble_store::Store;
use serde::Deserialize;
use serde_json::{json, Value};

/// Most results per page.
pub const MAX_PAGE: usize = 50;
/// Hits read from the index per search (sorted by date afterwards).
const FETCH: usize = 500;

/// What to look for. Dates are `YYYY-MM-DD` (local).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct MailQuery {
    pub query: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    /// Inclusive.
    pub after: Option<String>,
    /// Exclusive.
    pub before: Option<String>,
    pub has_attachment: Option<bool>,
    pub unread: Option<bool>,
    pub starred: Option<bool>,
    /// A folder role: `inbox`, `sent` or `archive`.
    pub folder: Option<String>,
    pub limit: Option<usize>,
    /// `next_cursor` from the previous page.
    pub cursor: Option<String>,
}

/// One result with the folders it is filed in.
#[derive(Debug, Clone)]
pub struct Found {
    pub message: Message,
    pub folders: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub items: Vec<Found>,
    pub next_cursor: Option<String>,
}

fn parse_day(value: &str) -> Option<i64> {
    let date = chrono::NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()?;
    crate::deadlines::local_ts(date.and_hms_opt(0, 0, 0)?)
}

fn parse_role(value: &str) -> Result<FolderRole, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "inbox" => Ok(FolderRole::Inbox),
        "sent" => Ok(FolderRole::Sent),
        "archive" => Ok(FolderRole::Archive),
        other => Err(format!(
            "folder must be inbox, sent or archive (not {other})"
        )),
    }
}

fn parse_cursor(value: &str) -> Result<(i64, String), String> {
    let (date, id) = value.split_once(':').ok_or("invalid cursor")?;
    Ok((date.parse().map_err(|_| "invalid cursor")?, id.to_string()))
}

/// Folder names and roles of the given accounts.
fn folders_of(store: &Store, accounts: &[String]) -> HashMap<String, (String, Option<FolderRole>)> {
    let mut map = HashMap::new();
    for account in accounts {
        for folder in store.list_folders(account).unwrap_or_default() {
            map.insert(
                folder.id.clone(),
                (folder.name.clone(), folder.role.clone()),
            );
        }
    }
    map
}

/// Search `accounts` (ids). Needs at least one of query, from, to,
/// subject, after or before; for "what's new" use the store's recent mail.
pub fn search(
    store: &Store,
    index: &TantivySearch,
    accounts: &[String],
    q: &MailQuery,
) -> Result<Page, String> {
    if accounts.is_empty() {
        return Ok(Page {
            items: Vec::new(),
            next_cursor: None,
        });
    }
    let text = q.query.as_deref().filter(|t| !t.trim().is_empty());
    let has_terms = text.is_some()
        || q.from.is_some()
        || q.to.is_some()
        || q.subject.is_some()
        || q.after.is_some()
        || q.before.is_some();
    if !has_terms {
        return Err(
            "Give a query, a sender, a subject or a date range (for recent mail, list recent mail instead)"
                .into(),
        );
    }
    let role = q.folder.as_deref().map(parse_role).transpose()?;
    let cursor = q.cursor.as_deref().map(parse_cursor).transpose()?;
    let limit = q.limit.unwrap_or(20).clamp(1, MAX_PAGE);

    let hits = index
        .advanced_search(AdvancedSearchParams {
            text,
            from: q.from.as_deref(),
            to: q.to.as_deref(),
            subject: q.subject.as_deref(),
            date_from: q.after.as_deref().and_then(parse_day),
            date_to: q.before.as_deref().and_then(parse_day).map(|d| d - 1),
            has_attachment: q.has_attachment,
            folder_id: None,
            account_ids: accounts,
            limit: FETCH,
        })
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = hits.iter().map(|h| h.message_id.clone()).collect();
    let mut messages = store.get_messages_batch(&ids).map_err(|e| e.to_string())?;
    let folder_ids = store
        .get_message_folder_ids_batch(&ids)
        .map_err(|e| e.to_string())?;
    let folders = folders_of(store, accounts);

    messages.retain(|m| accounts.contains(&m.account_id) && !m.is_deleted && !m.is_draft);
    messages.retain(|m| q.unread.is_none_or(|unread| unread == !m.is_read));
    messages.retain(|m| q.starred.is_none_or(|starred| starred == m.is_starred));
    let roles_of = |id: &str| -> Vec<Option<FolderRole>> {
        folder_ids
            .get(id)
            .map(|ids| {
                ids.iter()
                    .map(|f| folders.get(f).and_then(|(_, r)| r.clone()))
                    .collect()
            })
            .unwrap_or_default()
    };
    messages.retain(|m| {
        let roles = roles_of(&m.id);
        !roles
            .iter()
            .any(|r| matches!(r, Some(FolderRole::Spam | FolderRole::Trash)))
            && role
                .as_ref()
                .is_none_or(|want| roles.iter().any(|r| r.as_ref() == Some(want)))
    });
    // Newest first; ties by id, so the cursor is exact.
    messages.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.id.cmp(&a.id)));
    if let Some((date, id)) = &cursor {
        messages.retain(|m| (m.date, m.id.as_str()) < (*date, id.as_str()));
    }
    let more = messages.len() > limit;
    messages.truncate(limit);
    let next_cursor = more
        .then(|| messages.last().map(|m| format!("{}:{}", m.date, m.id)))
        .flatten();
    let items = messages
        .into_iter()
        .map(|message| {
            let names = folder_ids
                .get(&message.id)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|f| folders.get(f).map(|(n, _)| n.clone()))
                        .collect()
                })
                .unwrap_or_default();
            Found {
                message,
                folders: names,
            }
        })
        .collect();
    Ok(Page { items, next_cursor })
}

/// `messages` without those filed in spam or trash.
pub fn outside_spam_and_trash(store: &Store, messages: &[Message]) -> Vec<Message> {
    let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
    let folder_ids = store.get_message_folder_ids_batch(&ids).unwrap_or_default();
    let mut accounts: Vec<String> = messages.iter().map(|m| m.account_id.clone()).collect();
    accounts.sort();
    accounts.dedup();
    let folders = folders_of(store, &accounts);
    messages
        .iter()
        .filter(|m| {
            !folder_ids.get(&m.id).is_some_and(|ids| {
                ids.iter().any(|f| {
                    matches!(
                        folders.get(f).and_then(|(_, r)| r.clone()),
                        Some(FolderRole::Spam | FolderRole::Trash)
                    )
                })
            })
        })
        .cloned()
        .collect()
}

/// What a search covered, per account (id → email).
pub fn coverage(store: &Store, accounts: &[(String, String)]) -> Value {
    let per_account: Vec<Value> = accounts
        .iter()
        .map(|(id, email)| {
            json!({
                "account": email,
                "synced_since": store
                    .earliest_message_date(id)
                    .ok()
                    .flatten()
                    .map(crate::agent::capabilities::local_day),
            })
        })
        .collect();
    json!({
        "accounts": per_account,
        "excluded": ["spam", "trash", "drafts"],
        "note": "Only mail synced to this device, in the accounts listed. No match doesn't mean the mail doesn't exist.",
    })
}

#[cfg(test)]
mod tests;
