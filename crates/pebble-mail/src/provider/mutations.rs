//! [`RemoteMailOps`] for the three providers that can change server state.
//!
//! Commands and the pending-op worker talk to this trait only; everything
//! provider-specific about a mutation lives here:
//! - Gmail expresses folders, read and starred state as labels;
//! - Microsoft Graph assigns a new message id on every move;
//! - IMAP UIDs are scoped to a mailbox, so a move must re-discover the UID
//!   in the destination, and a missing trash means EXPUNGE.
//!
//! POP3 has no remote mutations; callers treat it as local-only.

use async_trait::async_trait;
use pebble_core::traits::{
    CreateFolderRequest, FlagUpdate, FolderProvider, LabelDelta, LabelProvider, MoveRequest,
    RemoteFolderRef, RemoteIdUpdate, RemoteMailOps, RemoteMessageRef, RenameFolderRequest,
};
use pebble_core::{new_id, Folder, FolderRole, FolderType, PebbleError, Result};

use crate::imap::folder_management::encode_folder_segment;
use crate::imap::ImapProvider;

use super::gmail::GmailProvider;
use super::outlook::OutlookProvider;

// ---------------------------------------------------------------------------
// Shared rules
// ---------------------------------------------------------------------------

/// Gmail label edit equivalent to moving a message between two folders.
pub fn gmail_move_label_delta(
    source_remote_id: Option<&str>,
    target_remote_id: &str,
    target_role: Option<FolderRole>,
) -> LabelDelta {
    if target_role == Some(FolderRole::Spam) {
        return LabelDelta {
            add_labels: vec!["SPAM".to_string()],
            remove_labels: vec!["INBOX".to_string()],
        };
    }

    if target_role == Some(FolderRole::Archive) {
        return LabelDelta {
            add_labels: vec![],
            remove_labels: vec!["INBOX".to_string()],
        };
    }

    let target = valid_gmail_label(target_remote_id);
    let source = source_remote_id.and_then(valid_gmail_label);

    let mut add_labels = Vec::new();
    if let Some(label) = target {
        push_unique(&mut add_labels, label);
    }

    let mut remove_labels = Vec::new();
    match source {
        Some(label) if Some(label) != target => push_unique(&mut remove_labels, label),
        Some(_) => {}
        None => push_unique(&mut remove_labels, "INBOX"),
    }

    remove_labels.retain(|label| !add_labels.contains(label));

    LabelDelta {
        add_labels,
        remove_labels,
    }
}

fn valid_gmail_label(label: &str) -> Option<&str> {
    let trimmed = label.trim();
    if trimmed.is_empty() || trimmed.starts_with("__local_") {
        None
    } else {
        Some(trimmed)
    }
}

fn push_unique(labels: &mut Vec<String>, label: &str) {
    if !labels.iter().any(|existing| existing == label) {
        labels.push(label.to_string());
    }
}

/// Gmail label edit for a flag change, unless an earlier attempt recorded one.
fn gmail_flag_label_delta(update: &FlagUpdate<'_>) -> LabelDelta {
    if let Some(labels) = update.labels {
        return labels.clone();
    }
    let mut delta = LabelDelta::default();
    match update.is_read {
        Some(true) => delta.remove_labels.push("UNREAD".to_string()),
        Some(false) => delta.add_labels.push("UNREAD".to_string()),
        None => {}
    }
    match update.is_starred {
        Some(true) => delta.add_labels.push("STARRED".to_string()),
        Some(false) => delta.remove_labels.push("STARRED".to_string()),
        None => {}
    }
    delta
}

/// Folders a user may rename. Local system folders (`__local_archive__`, the
/// outbox, ...) are fixed; user-created local folders are not.
pub fn editable_folder(folder: &Folder) -> Result<()> {
    if folder.is_system
        || folder.role.is_some()
        || (folder.remote_id.starts_with("__local_")
            && !folder.remote_id.starts_with("__local_folder_"))
    {
        return Err(PebbleError::Validation(
            "System folders cannot be renamed".into(),
        ));
    }
    Ok(())
}

/// Reject a name (or remote id) that already exists among `folders`.
pub fn folder_name_collision(
    folders: &[Folder],
    remote_id: Option<&str>,
    name: &str,
    parent: Option<&str>,
    exclude: Option<&str>,
) -> Result<()> {
    if folders.iter().any(|f| {
        Some(f.id.as_str()) != exclude
            && (remote_id.is_some_and(|r| f.remote_id.eq_ignore_ascii_case(r))
                || (f.parent_id.as_deref() == parent && f.name.eq_ignore_ascii_case(name)))
    }) {
        return Err(PebbleError::Validation(
            "A folder with that name already exists".into(),
        ));
    }
    Ok(())
}

/// Cached folders affected by renaming `source` to `new_remote`, with their
/// new remote ids and names. IMAP RENAME carries the whole hierarchy below
/// the delimiter boundary; local ids stay stable.
pub fn imap_rename_changes(
    folders: &[Folder],
    source: &Folder,
    new_remote: &str,
    delimiter: Option<&str>,
) -> Vec<Folder> {
    let prefix = delimiter.map(|d| format!("{}{d}", source.remote_id));
    folders
        .iter()
        .filter(|f| {
            f.id == source.id || prefix.as_ref().is_some_and(|p| f.remote_id.starts_with(p))
        })
        .map(|f| {
            let remote_id = format!("{new_remote}{}", &f.remote_id[source.remote_id.len()..]);
            Folder {
                name: crate::imap::decode_folder_name(&remote_id),
                remote_id,
                ..f.clone()
            }
        })
        .collect()
}

fn required<'a>(folder: Option<RemoteFolderRef<'a>>, what: &str) -> Result<RemoteFolderRef<'a>> {
    folder.ok_or_else(|| PebbleError::Internal(format!("No {what} folder for pending op")))
}

// ---------------------------------------------------------------------------
// Gmail
// ---------------------------------------------------------------------------

#[async_trait]
impl RemoteMailOps for GmailProvider {
    async fn apply_flags(
        &self,
        message: RemoteMessageRef<'_>,
        update: FlagUpdate<'_>,
    ) -> Result<()> {
        let delta = gmail_flag_label_delta(&update);
        if delta.add_labels.is_empty() && delta.remove_labels.is_empty() {
            return Ok(());
        }
        self.modify_labels(message.remote_id, &delta.add_labels, &delta.remove_labels)
            .await
    }

    async fn move_to_folder(
        &self,
        message: RemoteMessageRef<'_>,
        request: MoveRequest<'_>,
    ) -> Result<RemoteIdUpdate> {
        let delta = match request.labels {
            Some(labels) => labels.clone(),
            None => gmail_move_label_delta(
                request.source.map(|source| source.remote_id),
                request.target.remote_id,
                request.target.role.cloned(),
            ),
        };
        self.modify_labels(message.remote_id, &delta.add_labels, &delta.remove_labels)
            .await?;
        Ok(RemoteIdUpdate::Unchanged)
    }

    async fn archive(
        &self,
        message: RemoteMessageRef<'_>,
        _source: Option<RemoteFolderRef<'_>>,
        _archive: Option<RemoteFolderRef<'_>>,
    ) -> Result<RemoteIdUpdate> {
        // Gmail's archive is the absence of the INBOX label.
        self.modify_labels(message.remote_id, &[], &["INBOX".to_string()])
            .await?;
        Ok(RemoteIdUpdate::Unchanged)
    }

    async fn restore_to_inbox(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
        _inbox: RemoteFolderRef<'_>,
    ) -> Result<RemoteIdUpdate> {
        if source.is_some_and(|source| source.has_role(FolderRole::Trash)) {
            self.untrash_message(message.remote_id).await?;
        } else {
            self.modify_labels(message.remote_id, &["INBOX".to_string()], &[])
                .await?;
        }
        Ok(RemoteIdUpdate::Unchanged)
    }

    async fn trash(
        &self,
        message: RemoteMessageRef<'_>,
        _source: Option<RemoteFolderRef<'_>>,
        _trash: Option<RemoteFolderRef<'_>>,
    ) -> Result<RemoteIdUpdate> {
        self.trash_message(message.remote_id).await?;
        Ok(RemoteIdUpdate::Unchanged)
    }

    async fn delete_permanently(
        &self,
        message: RemoteMessageRef<'_>,
        _source: Option<RemoteFolderRef<'_>>,
    ) -> Result<()> {
        self.delete_message_permanently(message.remote_id).await
    }

    async fn create_remote_folder(&self, request: CreateFolderRequest<'_>) -> Result<Folder> {
        if request.name.contains('/') {
            return Err(PebbleError::Validation(
                "Use the parent picker to create a nested label".into(),
            ));
        }
        let full = request.parent.map_or_else(
            || request.name.to_owned(),
            |parent| format!("{}/{}", parent.name, request.name),
        );
        folder_name_collision(&self.list_folders().await?, None, &full, None, None)?;
        self.create_folder_label(&full).await
    }

    async fn rename_remote_folder(&self, request: RenameFolderRequest<'_>) -> Result<Vec<Folder>> {
        let RenameFolderRequest { folder, name, .. } = request;
        if name.contains('/') {
            return Err(PebbleError::Validation(
                "A label name cannot contain '/' here".into(),
            ));
        }
        let fresh = self.list_folders().await?;
        let server = fresh
            .iter()
            .find(|f| f.remote_id == folder.remote_id)
            .ok_or_else(|| PebbleError::Validation("Label no longer exists".into()))?;
        editable_folder(server)?;
        if fresh
            .iter()
            .any(|f| f.name.starts_with(&format!("{}/", folder.name)))
        {
            return Err(PebbleError::UnsupportedProvider("Gmail does not support atomic label hierarchy rename; rename leaf labels individually".into()));
        }
        let full = folder
            .name
            .rsplit_once('/')
            .map_or_else(|| name.to_owned(), |(parent, _)| format!("{parent}/{name}"));
        if full == folder.name {
            return Err(PebbleError::Validation(
                "The label already has this name".into(),
            ));
        }
        folder_name_collision(&fresh, None, &full, None, Some(&server.id))?;
        let updated = self.rename_folder_label(&folder.remote_id, &full).await?;
        Ok(vec![Folder {
            name: updated.name,
            ..folder.clone()
        }])
    }
}

// ---------------------------------------------------------------------------
// Outlook (Microsoft Graph)
// ---------------------------------------------------------------------------

impl OutlookProvider {
    async fn graph_move(&self, remote_id: &str, target_remote_id: &str) -> Result<RemoteIdUpdate> {
        FolderProvider::move_message(self, remote_id, target_remote_id)
            .await
            .map(RemoteIdUpdate::Global)
    }
}

#[async_trait]
impl RemoteMailOps for OutlookProvider {
    async fn apply_flags(
        &self,
        message: RemoteMessageRef<'_>,
        update: FlagUpdate<'_>,
    ) -> Result<()> {
        if let Some(read) = update.is_read {
            self.update_read_status(message.remote_id, read).await?;
        }
        if let Some(starred) = update.is_starred {
            self.update_flag_status(message.remote_id, starred).await?;
        }
        Ok(())
    }

    async fn move_to_folder(
        &self,
        message: RemoteMessageRef<'_>,
        request: MoveRequest<'_>,
    ) -> Result<RemoteIdUpdate> {
        if request.recover_previous_attempt {
            if let Some(header) = message.message_id_header.filter(|h| !h.trim().is_empty()) {
                if let Some(id) = self
                    .recover_managed_move(message.remote_id, request.target.remote_id, header)
                    .await?
                {
                    return Ok(RemoteIdUpdate::Global(id));
                }
            }
        }
        self.graph_move(message.remote_id, request.target.remote_id)
            .await
    }

    async fn archive(
        &self,
        message: RemoteMessageRef<'_>,
        _source: Option<RemoteFolderRef<'_>>,
        archive: Option<RemoteFolderRef<'_>>,
    ) -> Result<RemoteIdUpdate> {
        let archive = required(archive, "target")?;
        self.graph_move(message.remote_id, archive.remote_id).await
    }

    async fn restore_to_inbox(
        &self,
        message: RemoteMessageRef<'_>,
        _source: Option<RemoteFolderRef<'_>>,
        inbox: RemoteFolderRef<'_>,
    ) -> Result<RemoteIdUpdate> {
        self.graph_move(message.remote_id, inbox.remote_id).await
    }

    async fn trash(
        &self,
        message: RemoteMessageRef<'_>,
        _source: Option<RemoteFolderRef<'_>>,
        _trash: Option<RemoteFolderRef<'_>>,
    ) -> Result<RemoteIdUpdate> {
        self.trash_message(message.remote_id)
            .await
            .map(RemoteIdUpdate::Global)
    }

    async fn delete_permanently(
        &self,
        message: RemoteMessageRef<'_>,
        _source: Option<RemoteFolderRef<'_>>,
    ) -> Result<()> {
        self.delete_message_permanently(message.remote_id).await
    }

    async fn create_remote_folder(&self, request: CreateFolderRequest<'_>) -> Result<Folder> {
        let parent_remote = request.parent.map(|f| f.remote_id.as_str());
        let fresh = self.list_folders().await?;
        folder_name_collision(&fresh, None, request.name, parent_remote, None)?;
        self.create_managed_folder(parent_remote, request.name)
            .await
    }

    async fn rename_remote_folder(&self, request: RenameFolderRequest<'_>) -> Result<Vec<Folder>> {
        let RenameFolderRequest { folder, name, .. } = request;
        let fresh = self.list_folders().await?;
        let server = fresh
            .iter()
            .find(|f| f.remote_id == folder.remote_id)
            .ok_or_else(|| PebbleError::Validation("Folder no longer exists".into()))?;
        editable_folder(server)?;
        if name == folder.name {
            return Err(PebbleError::Validation(
                "The folder already has this name".into(),
            ));
        }
        folder_name_collision(
            &fresh,
            None,
            name,
            server.parent_id.as_deref(),
            Some(&server.id),
        )?;
        self.rename_managed_folder(&folder.remote_id, name).await?;
        Ok(vec![Folder {
            name: name.into(),
            ..folder.clone()
        }])
    }
}

// ---------------------------------------------------------------------------
// IMAP
// ---------------------------------------------------------------------------

fn imap_uid(message: &RemoteMessageRef<'_>) -> Result<u32> {
    message
        .remote_id
        .parse::<u32>()
        .map_err(|_| PebbleError::Internal(format!("Invalid IMAP UID: {}", message.remote_id)))
}

impl ImapProvider {
    /// MOVE a message and return its UID in the destination. An IMAP MOVE
    /// assigns a new UID; without it, later flag changes would hit an
    /// unrelated message in the destination mailbox.
    pub async fn move_tracked(
        &self,
        message: RemoteMessageRef<'_>,
        source_mailbox: &str,
        target_mailbox: &str,
    ) -> Result<u32> {
        let uid = imap_uid(&message)?;
        let header = message
            .message_id_header
            .filter(|value| !value.trim().is_empty());
        if let Some(header) = header {
            if let Some(source_uid) = self.find_uid_by_message_id(source_mailbox, header).await? {
                self.move_message_with_dedup(
                    source_mailbox,
                    source_uid,
                    target_mailbox,
                    Some(header),
                )
                .await?;
                if source_uid != uid {
                    tracing::debug!("Resolved changed IMAP UID {uid} -> {source_uid}");
                }
            }
            // Also handles a retry after MOVE succeeded but local persistence failed.
            return self
                .find_uid_by_message_id(target_mailbox, header)
                .await?
                .ok_or_else(|| {
                    PebbleError::Sync("Moved IMAP message was not found in the destination".into())
                });
        }

        let (_, source_raw) = self
            .fetch_messages_by_uids(source_mailbox, &[uid])
            .await?
            .pop()
            .ok_or_else(|| PebbleError::Sync("Source IMAP message not found".into()))?;
        let before: std::collections::HashSet<_> = self
            .fetch_all_uids(target_mailbox)
            .await?
            .into_iter()
            .collect();
        self.move_message_with_dedup(source_mailbox, uid, target_mailbox, None)
            .await?;
        let added: Vec<_> = self
            .fetch_all_uids(target_mailbox)
            .await?
            .into_iter()
            .filter(|uid| !before.contains(uid))
            .collect();
        let matching: Vec<_> = self
            .fetch_messages_by_uids(target_mailbox, &added)
            .await?
            .into_iter()
            .filter(|(_, raw)| raw == &source_raw)
            .map(|(uid, _)| uid)
            .collect();
        match matching.as_slice() {
            [uid] => Ok(*uid),
            _ => Err(PebbleError::Sync(
                "Cannot uniquely resolve moved IMAP message".into(),
            )),
        }
    }

    async fn tracked_move_update(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
        target: RemoteFolderRef<'_>,
    ) -> Result<RemoteIdUpdate> {
        let source = required(source, "source")?;
        let uid = self
            .move_tracked(message, source.remote_id, target.remote_id)
            .await?;
        Ok(RemoteIdUpdate::InFolder {
            folder_remote_id: target.remote_id.to_string(),
            remote_id: uid.to_string(),
        })
    }
}

#[async_trait]
impl RemoteMailOps for ImapProvider {
    async fn apply_flags(
        &self,
        message: RemoteMessageRef<'_>,
        update: FlagUpdate<'_>,
    ) -> Result<()> {
        let folder = update.folder_remote_id.ok_or_else(|| {
            PebbleError::Internal("Pending update_flags has no IMAP folder".to_string())
        })?;
        let uid = imap_uid(&message)?;
        ImapProvider::set_flags(self, folder, uid, update.is_read, update.is_starred).await
    }

    async fn move_to_folder(
        &self,
        message: RemoteMessageRef<'_>,
        request: MoveRequest<'_>,
    ) -> Result<RemoteIdUpdate> {
        // Tracking by Message-ID already makes a replayed move idempotent.
        self.tracked_move_update(message, request.source, request.target)
            .await
    }

    async fn archive(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
        archive: Option<RemoteFolderRef<'_>>,
    ) -> Result<RemoteIdUpdate> {
        let archive = required(archive, "target")?;
        self.tracked_move_update(message, source, archive).await
    }

    async fn restore_to_inbox(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
        inbox: RemoteFolderRef<'_>,
    ) -> Result<RemoteIdUpdate> {
        self.tracked_move_update(message, source, inbox).await
    }

    async fn trash(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
        trash: Option<RemoteFolderRef<'_>>,
    ) -> Result<RemoteIdUpdate> {
        let source = required(source, "source")?;
        let uid = imap_uid(&message)?;
        match trash.filter(|trash| trash.remote_id != source.remote_id) {
            Some(trash) => {
                self.move_message_with_dedup(
                    source.remote_id,
                    uid,
                    trash.remote_id,
                    message.message_id_header,
                )
                .await?
            }
            // No separate trash mailbox: the delete is an EXPUNGE.
            None => self.delete_message(source.remote_id, uid).await?,
        }
        Ok(RemoteIdUpdate::Unchanged)
    }

    async fn delete_permanently(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
    ) -> Result<()> {
        let source = required(source, "source")?;
        let uid = imap_uid(&message)?;
        self.delete_message(source.remote_id, uid).await
    }

    async fn create_remote_folder(&self, request: CreateFolderRequest<'_>) -> Result<Folder> {
        let available = self.folder_parents().await?;
        let parent_remote = request
            .parent
            .map(|p| p.remote_id.as_str())
            .or(request.parent_remote_id);
        let delimiter = if let Some(remote) = parent_remote {
            available
                .iter()
                .find(|p| p.remote_id == remote)
                .ok_or_else(|| PebbleError::Validation("Parent namespace no longer exists".into()))?
                .delimiter
                .clone()
        } else {
            self.folder_delimiter(None).await?
        };
        let segment = encode_folder_segment(request.name, delimiter.as_deref())?;
        let remote = if let Some(parent_remote) = parent_remote {
            let delimiter = delimiter.as_deref().ok_or_else(|| {
                PebbleError::Validation("This IMAP namespace does not support child folders".into())
            })?;
            format!("{parent_remote}{delimiter}{segment}")
        } else {
            segment
        };
        if available
            .iter()
            .any(|p| p.remote_id.eq_ignore_ascii_case(&remote))
        {
            return Err(PebbleError::Validation(
                "A folder with that name already exists".into(),
            ));
        }
        ImapProvider::create_folder(self, &remote).await?;
        // Tagged OK is the server receipt; no local row is created on failure.
        Ok(Folder {
            id: new_id(),
            account_id: request.account_id.into(),
            name: crate::imap::decode_folder_name(&remote),
            remote_id: remote,
            folder_type: FolderType::Folder,
            role: None,
            parent_id: request
                .parent
                .map(|p| p.id.as_str())
                .or(request.parent_remote_id)
                .map(str::to_owned),
            color: None,
            is_system: false,
            sort_order: 100,
        })
    }

    async fn rename_remote_folder(&self, request: RenameFolderRequest<'_>) -> Result<Vec<Folder>> {
        let RenameFolderRequest {
            account_id,
            local_folders: folders,
            folder,
            name,
        } = request;
        let fresh = self.list_folders(account_id).await?;
        let server = fresh
            .iter()
            .find(|f| f.remote_id == folder.remote_id)
            .ok_or_else(|| {
                PebbleError::Validation("Folder no longer exists on the server".into())
            })?;
        editable_folder(server)?;
        let delimiter = self.folder_delimiter(Some(&folder.remote_id)).await?;
        let segment = encode_folder_segment(name, delimiter.as_deref())?;
        let new_remote = delimiter
            .as_deref()
            .and_then(|d| {
                folder
                    .remote_id
                    .rsplit_once(d)
                    .map(|(p, _)| format!("{p}{d}{segment}"))
            })
            .unwrap_or(segment);
        if new_remote == folder.remote_id {
            return Err(PebbleError::Validation(
                "The folder already has this name".into(),
            ));
        }
        folder_name_collision(&fresh, Some(&new_remote), &new_remote, None, None)?;
        let changes = imap_rename_changes(folders, folder, &new_remote, delimiter.as_deref());
        for changed in &changes {
            let prior = folders.iter().find(|f| f.id == changed.id).ok_or_else(|| {
                PebbleError::Internal(format!(
                    "Renamed folder {} is missing from the local folder list",
                    changed.id
                ))
            })?;
            if fresh.iter().any(|f| {
                f.remote_id != prior.remote_id
                    && f.remote_id.eq_ignore_ascii_case(&changed.remote_id)
            }) {
                return Err(PebbleError::Validation(
                    "The renamed hierarchy would collide with an existing folder".into(),
                ));
            }
        }
        let prefix = delimiter
            .as_deref()
            .map(|d| format!("{}{d}", folder.remote_id));
        if fresh
            .iter()
            .any(|f| prefix.as_ref().is_some_and(|p| f.remote_id.starts_with(p)) && f.is_system)
        {
            return Err(PebbleError::Validation(
                "A hierarchy containing system folders cannot be renamed".into(),
            ));
        }
        for changed in &changes {
            folder_name_collision(
                folders,
                Some(&changed.remote_id),
                &changed.name,
                changed.parent_id.as_deref(),
                Some(&changed.id),
            )?;
        }
        ImapProvider::rename_folder(self, &folder.remote_id, &new_remote).await?;
        Ok(changes)
    }

    async fn close(&self) {
        let _ = self.disconnect().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_flag_delta_derives_labels_or_uses_recorded_ones() {
        let derived = gmail_flag_label_delta(&FlagUpdate {
            is_read: Some(true),
            is_starred: Some(true),
            ..FlagUpdate::default()
        });
        assert_eq!(derived.add_labels, vec!["STARRED"]);
        assert_eq!(derived.remove_labels, vec!["UNREAD"]);

        let recorded = LabelDelta {
            add_labels: vec!["X".into()],
            remove_labels: vec![],
        };
        let used = gmail_flag_label_delta(&FlagUpdate {
            is_read: Some(true),
            labels: Some(&recorded),
            ..FlagUpdate::default()
        });
        assert_eq!(used, recorded);
    }

    #[test]
    fn imap_uid_errors_are_typed_internal_errors() {
        let message = RemoteMessageRef {
            remote_id: "not-a-uid",
            message_id_header: None,
        };
        assert!(matches!(imap_uid(&message), Err(PebbleError::Internal(_))));
    }
}
