use super::*;

#[cfg(test)]
pub(super) async fn collect_paginated_gmail_refs<F, Fut>(
    label_id: &str,
    limit: u32,
    mut fetch_page: F,
) -> Result<Vec<GmailMessageRef>>
where
    F: FnMut(String, u32, Option<String>) -> Fut,
    Fut: Future<Output = Result<(Vec<GmailMessageRef>, Option<String>)>>,
{
    let mut all_refs = Vec::new();
    let mut page_token = None;

    loop {
        let (mut refs, next_page) =
            fetch_page(label_id.to_string(), limit, page_token.take()).await?;
        all_refs.append(&mut refs);

        match next_page {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => break,
        }
    }

    Ok(all_refs)
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GmailHistoryPage {
    pub(super) history: Option<Vec<GmailHistoryEntry>>,
    #[serde(rename = "historyId")]
    pub(super) history_id: Option<String>,
    #[serde(rename = "nextPageToken")]
    pub(super) next_page_token: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GmailHistoryEntry {
    #[serde(rename = "messagesAdded")]
    pub(super) messages_added: Option<Vec<GmailHistoryMessage>>,
    #[serde(rename = "messagesDeleted")]
    pub(super) messages_deleted: Option<Vec<GmailHistoryMessage>>,
    #[serde(rename = "labelsAdded")]
    pub(super) labels_added: Option<Vec<GmailHistoryLabelChange>>,
    #[serde(rename = "labelsRemoved")]
    pub(super) labels_removed: Option<Vec<GmailHistoryLabelChange>>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GmailHistoryMessage {
    pub(super) message: GmailHistoryMessageRef,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GmailHistoryLabelChange {
    pub(super) message: GmailHistoryMessageRef,
    #[serde(rename = "labelIds")]
    pub(super) label_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct GmailHistoryMessageRef {
    pub(super) id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GmailHistoryLabelAction {
    Add,
    Remove,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct GmailHistoryLabelMutation {
    pub(super) remote_id: String,
    pub(super) label_id: String,
    pub(super) action: GmailHistoryLabelAction,
}

pub(super) fn gmail_history_label_mutations(
    entries: &[GmailHistoryEntry],
) -> Vec<GmailHistoryLabelMutation> {
    let mut sequence = 0usize;
    let mut last_events: HashMap<(String, String), (usize, GmailHistoryLabelAction)> =
        HashMap::new();

    for entry in entries {
        for (changes, action) in [
            (&entry.labels_added, GmailHistoryLabelAction::Add),
            (&entry.labels_removed, GmailHistoryLabelAction::Remove),
        ] {
            for change in changes.iter().flatten() {
                for label_id in &change.label_ids {
                    last_events.insert(
                        (change.message.id.clone(), label_id.clone()),
                        (sequence, action),
                    );
                    sequence += 1;
                }
            }
        }
    }

    let mut final_events = last_events
        .into_iter()
        .map(|((remote_id, label_id), (sequence, action))| {
            (
                sequence,
                GmailHistoryLabelMutation {
                    remote_id,
                    label_id,
                    action,
                },
            )
        })
        .collect::<Vec<_>>();
    final_events.sort_by_key(|(sequence, _)| *sequence);
    final_events
        .into_iter()
        .map(|(_, mutation)| mutation)
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct GmailHistoryFlagChange {
    pub(super) remote_id: String,
    pub(super) is_read: Option<bool>,
    pub(super) is_starred: Option<bool>,
}

pub(super) fn gmail_history_flag_changes(
    entries: &[GmailHistoryEntry],
) -> Vec<GmailHistoryFlagChange> {
    let mut by_message: HashMap<String, (usize, Option<bool>, Option<bool>)> = HashMap::new();
    for (sequence, mutation) in gmail_history_label_mutations(entries)
        .into_iter()
        .enumerate()
    {
        let entry = by_message
            .entry(mutation.remote_id)
            .or_insert((sequence, None, None));
        entry.0 = sequence;
        match mutation.label_id.as_str() {
            "UNREAD" => {
                entry.1 = Some(matches!(mutation.action, GmailHistoryLabelAction::Remove));
            }
            "STARRED" => {
                entry.2 = Some(matches!(mutation.action, GmailHistoryLabelAction::Add));
            }
            _ => {}
        }
    }

    let mut changes = by_message
        .into_iter()
        .filter_map(|(remote_id, (sequence, is_read, is_starred))| {
            (is_read.is_some() || is_starred.is_some()).then_some((
                sequence,
                GmailHistoryFlagChange {
                    remote_id,
                    is_read,
                    is_starred,
                },
            ))
        })
        .collect::<Vec<_>>();
    changes.sort_by_key(|(sequence, _)| *sequence);
    changes.into_iter().map(|(_, change)| change).collect()
}

pub(super) enum GmailHistoryPageFetch {
    Page(GmailHistoryPage),
    CursorExpired,
}

pub(super) enum GmailHistoryReplayOutcome {
    Applied(Option<String>),
    CursorExpired,
}

pub(super) async fn replay_gmail_history_and_checkpoint<F, Fut>(
    replay: F,
    checkpoint: impl FnOnce(&str) -> Result<()>,
) -> Result<GmailHistoryReplayOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<GmailHistoryReplayOutcome>>,
{
    let outcome = replay().await?;
    if let GmailHistoryReplayOutcome::Applied(Some(history_id)) = &outcome {
        checkpoint(history_id)?;
    }
    Ok(outcome)
}

pub(super) async fn fetch_cached_gmail_metadata<F, Fut>(
    remote_ids: Vec<String>,
    fetch: F,
) -> Result<Vec<(String, Option<GmailMessageMetadata>)>>
where
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<Option<GmailMessageMetadata>>> + Send,
{
    use futures::stream::{self, StreamExt};
    let fetched = stream::iter(remote_ids.into_iter().map(|remote_id| {
        let fetch = fetch.clone();
        async move {
            let metadata = fetch(remote_id.clone()).await?;
            Ok((remote_id, metadata))
        }
    }))
    .buffer_unordered(10)
    .collect::<Vec<Result<_>>>()
    .await;
    fetched.into_iter().collect()
}

pub(super) struct GmailHistoryBatch {
    pub(super) entries: Vec<GmailHistoryEntry>,
    pub(super) history_id: Option<String>,
    pub(super) cursor_expired: bool,
}

pub(super) fn gmail_history_cursor_expired(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::NOT_FOUND
}

pub(super) async fn collect_paginated_gmail_history<F, Fut>(
    _history_id: &str,
    mut fetch_page: F,
) -> Result<GmailHistoryBatch>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<GmailHistoryPageFetch>>,
{
    let mut entries = Vec::new();
    let mut history_id = None;
    let mut page_token = None;

    loop {
        let page = match fetch_page(page_token.take()).await? {
            GmailHistoryPageFetch::Page(page) => page,
            GmailHistoryPageFetch::CursorExpired => {
                return Ok(GmailHistoryBatch {
                    entries: Vec::new(),
                    history_id: None,
                    cursor_expired: true,
                });
            }
        };
        entries.extend(page.history.unwrap_or_default());
        if page.history_id.is_some() {
            history_id = page.history_id;
        }

        match page.next_page_token {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => break,
        }
    }

    Ok(GmailHistoryBatch {
        entries,
        history_id,
        cursor_expired: false,
    })
}
