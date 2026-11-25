//! One path for every remote mail mutation.
//!
//! Commands describe a change as a typed [`MailOp`] and hand it to a
//! [`RemoteSession`]. The session runs it through the provider's
//! [`RemoteMailOps`] implementation and, when the provider is unreachable or
//! the call fails, queues the very same op for the pending-op worker, which
//! later replays it through [`execute_mail_op`]. Provider quirks live in the
//! `RemoteMailOps` impls (pebble-mail); the few provider facts commands need
//! before connecting are the policy helpers below.

use pebble_core::traits::{
    FlagUpdate, LabelDelta, MoveRequest, RemoteFolderRef, RemoteIdUpdate, RemoteMailOps,
    RemoteMessageRef,
};
use pebble_core::{Folder, FolderRole, Message, PebbleError, ProviderType, Result};
use pebble_mail::{GmailProvider, ImapProvider, OutlookProvider};
use pebble_store::pending_ops::{
    DeleteOp, FolderChangeOp, MailOp, MailOpEnvelope, PendingMailOpEnqueue, PreviousLocalState,
};
use pebble_store::Store;

use crate::state::AppState;

use super::{
    classify_remote_delete_result, connect_gmail, connect_imap, connect_outlook,
    find_message_folder, queued_remote_error, remote_mutation_allows_local_commit,
    RemoteMutationOutcome,
};

/// A connected provider, ready for operations.
///
/// Wraps the three provider types so callers can match once instead of
/// duplicating the connect / disconnect boilerplate everywhere. Draft sync
/// still dispatches on it; mutations go through [`connect_remote`].
pub(in crate::commands) enum ConnectedProvider {
    Gmail(GmailProvider),
    Outlook(OutlookProvider),
    Imap(ImapProvider),
}

impl ConnectedProvider {
    /// Connect to the appropriate provider for the given account.
    pub async fn connect(
        state: &AppState,
        account_id: &str,
        provider_type: &ProviderType,
    ) -> Result<Self> {
        match provider_type {
            ProviderType::Gmail => Ok(Self::Gmail(connect_gmail(state, account_id).await?)),
            ProviderType::Outlook => Ok(Self::Outlook(connect_outlook(state, account_id).await?)),
            ProviderType::Imap => Ok(Self::Imap(connect_imap(state, account_id).await?)),
            ProviderType::Pop3 => Err(pop3_unsupported()),
        }
    }

    /// Disconnect the provider (only meaningful for IMAP).
    pub async fn disconnect(&self) {
        if let Self::Imap(imap) = self {
            let _ = imap.disconnect().await;
        }
    }
}

fn pop3_unsupported() -> PebbleError {
    PebbleError::UnsupportedProvider("POP3 does not support remote message mutations".to_string())
}

/// Connect to the account's provider as a [`RemoteMailOps`]. POP3 has no
/// remote mutations and yields [`PebbleError::UnsupportedProvider`].
pub(crate) async fn connect_remote(
    state: &AppState,
    account_id: &str,
    provider_type: &ProviderType,
) -> Result<Box<dyn RemoteMailOps>> {
    Ok(match provider_type {
        ProviderType::Gmail => Box::new(connect_gmail(state, account_id).await?),
        ProviderType::Outlook => Box::new(connect_outlook(state, account_id).await?),
        ProviderType::Imap => Box::new(connect_imap(state, account_id).await?),
        ProviderType::Pop3 => return Err(pop3_unsupported()),
    })
}

// ---------------------------------------------------------------------------
// Provider policy needed before a connection exists
// ---------------------------------------------------------------------------

/// Whether a message's remote id is only valid inside its folder (IMAP UIDs).
/// Such messages cannot be touched remotely from a device-only folder.
pub(crate) fn remote_ids_are_folder_scoped(provider_type: &ProviderType) -> bool {
    matches!(provider_type, ProviderType::Imap)
}

/// Whether the provider can archive without an Archive folder (Gmail drops
/// the INBOX label).
pub(crate) fn archives_without_folder(provider_type: &ProviderType) -> bool {
    matches!(provider_type, ProviderType::Gmail)
}

/// A move touching a device-only folder stays local, except Gmail's Spam
/// which is a label the server understands whatever its local id is.
pub(crate) fn move_is_local_only(
    provider_type: &ProviderType,
    source: &Folder,
    target: &Folder,
) -> bool {
    let touches_local =
        source.remote_id.starts_with("__local_") || target.remote_id.starts_with("__local_");
    touches_local
        && !(matches!(provider_type, ProviderType::Gmail) && target.role == Some(FolderRole::Spam))
}

// ---------------------------------------------------------------------------
// Building ops
// ---------------------------------------------------------------------------

pub(crate) fn folder_change(source: Option<&Folder>, target: Option<&Folder>) -> FolderChangeOp {
    FolderChangeOp {
        source_folder_id: source.map(|f| f.id.clone()),
        source_folder_remote_id: source.map(|f| f.remote_id.clone()),
        target_folder_id: target.map(|f| f.id.clone()),
        target_folder_remote_id: target.map(|f| f.remote_id.clone()),
        ..FolderChangeOp::default()
    }
}

pub(crate) fn delete_op(
    source: Option<&Folder>,
    trash: Option<&Folder>,
    permanent: bool,
) -> MailOp {
    let op = DeleteOp {
        source_folder_id: source.map(|f| f.id.clone()),
        source_folder_remote_id: source.map(|f| f.remote_id.clone()),
        trash_folder_id: trash.map(|f| f.id.clone()),
        trash_folder_remote_id: trash.map(|f| f.remote_id.clone()),
        permanent: Some(permanent),
    };
    if permanent {
        MailOp::DeletePermanent(op)
    } else {
        MailOp::Delete(op)
    }
}

/// Snapshot of the local state an optimistic commit is about to change.
pub(crate) fn local_previous_state(store: &Store, message: &Message) -> Result<PreviousLocalState> {
    Ok(PreviousLocalState {
        is_read: Some(message.is_read),
        is_starred: Some(message.is_starred),
        is_deleted: Some(message.is_deleted),
        deleted_at: message.deleted_at,
        remote_id: Some(message.remote_id.clone()),
        folder_ids: Some(store.get_message_folder_ids(&message.id)?),
    })
}

/// Queue `op` for the pending-op worker with merge/cancel semantics.
///
/// `local_commit`: the caller commits the change locally right away, so the
/// pre-change state is recorded and an abandoned op reverts it. `error`
/// marks the op as a failed first attempt (backoff starts now).
pub(crate) fn queue_mail_op(
    store: &Store,
    message: &Message,
    op: MailOp,
    local_commit: bool,
    error: Option<&str>,
) -> Result<RemoteMutationOutcome> {
    let op_type = op.op_type();
    let previous = if local_commit {
        Some(local_previous_state(store, message)?)
    } else {
        None
    };
    let envelope = MailOpEnvelope::new(&message.account_id, &message.remote_id, op)
        .with_local_commit(previous);
    match store.enqueue_mail_op(&message.account_id, &message.id, &envelope)? {
        PendingMailOpEnqueue::Inserted(op_id) | PendingMailOpEnqueue::Merged(op_id) => {
            if let Some(error) = error {
                store.mark_pending_mail_op_failed(&op_id, error)?;
            }
            Ok(if local_commit {
                RemoteMutationOutcome::QueuedLocalCommit
            } else {
                RemoteMutationOutcome::Queued
            })
        }
        // The new op undoes a queued op that never reached the server, so
        // the remote already matches the state this change produces locally.
        PendingMailOpEnqueue::Cancelled { cancelled_op_id } => {
            tracing::debug!(
                "Pending {op_type} for {} cancelled queued op {cancelled_op_id}",
                message.id
            );
            Ok(RemoteMutationOutcome::LocalOnly)
        }
    }
}

// ---------------------------------------------------------------------------
// Running ops
// ---------------------------------------------------------------------------

/// A folder resolved from a queued op, with the role the provider may need.
struct ResolvedFolder {
    remote_id: String,
    role: Option<FolderRole>,
}

impl ResolvedFolder {
    fn as_ref(&self) -> RemoteFolderRef<'_> {
        RemoteFolderRef {
            remote_id: &self.remote_id,
            role: self.role.as_ref(),
        }
    }

    fn of(folder: &Folder) -> Self {
        Self {
            remote_id: folder.remote_id.clone(),
            role: folder.role.clone(),
        }
    }
}

struct FolderResolver<'a> {
    state: &'a AppState,
    message: &'a Message,
    folders: Vec<Folder>,
}

impl<'a> FolderResolver<'a> {
    fn new(state: &'a AppState, message: &'a Message) -> Result<Self> {
        Ok(Self {
            state,
            message,
            folders: state.store.list_folders(&message.account_id)?,
        })
    }

    fn by_id_or_remote(&self, id: Option<&str>, remote_id: Option<&str>) -> Option<ResolvedFolder> {
        let local = id.and_then(|id| self.folders.iter().find(|f| f.id == id));
        match (remote_id, local) {
            (Some(remote_id), local) => Some(ResolvedFolder {
                remote_id: remote_id.to_string(),
                role: local
                    .or_else(|| self.folders.iter().find(|f| f.remote_id == remote_id))
                    .and_then(|f| f.role.clone()),
            }),
            (None, Some(folder)) => Some(ResolvedFolder::of(folder)),
            (None, None) => None,
        }
    }

    /// Recorded source, else the folder the message is in now.
    fn source(&self, id: Option<&str>, remote_id: Option<&str>) -> Option<ResolvedFolder> {
        self.by_id_or_remote(id, remote_id).or_else(|| {
            find_message_folder(self.state, &self.message.id, &self.message.account_id)
                .ok()
                .map(|folder| ResolvedFolder::of(&folder))
        })
    }

    /// Recorded target, else the account's folder with `fallback_role`.
    fn target(
        &self,
        id: Option<&str>,
        remote_id: Option<&str>,
        fallback_role: FolderRole,
    ) -> Option<ResolvedFolder> {
        self.by_id_or_remote(id, remote_id).or_else(|| {
            self.folders
                .iter()
                .find(|f| f.role.as_ref() == Some(&fallback_role))
                .map(ResolvedFolder::of)
        })
    }
}

fn required(folder: Option<ResolvedFolder>) -> Result<ResolvedFolder> {
    folder.ok_or_else(|| PebbleError::Internal("No target folder for pending op".to_string()))
}

fn recorded_labels(add: &Option<Vec<String>>, remove: &Option<Vec<String>>) -> Option<LabelDelta> {
    match (add, remove) {
        (Some(add), Some(remove)) => Some(LabelDelta {
            add_labels: add.clone(),
            remove_labels: remove.clone(),
        }),
        _ => None,
    }
}

/// Perform the remote half of `op`. Local state is not touched.
async fn execute_remote(
    state: &AppState,
    remote: &dyn RemoteMailOps,
    message: &Message,
    op: &MailOp,
) -> Result<RemoteIdUpdate> {
    let msg = RemoteMessageRef {
        remote_id: &message.remote_id,
        message_id_header: message.message_id_header.as_deref(),
    };
    let folders = FolderResolver::new(state, message)?;
    match op {
        MailOp::UpdateFlags(flags) => {
            let folder_remote_id = flags
                .folder_remote_id
                .clone()
                .or_else(|| folders.source(None, None).map(|f| f.remote_id));
            let labels = recorded_labels(&flags.add_labels, &flags.remove_labels);
            remote
                .apply_flags(
                    msg,
                    FlagUpdate {
                        is_read: flags.is_read,
                        is_starred: flags.is_starred,
                        folder_remote_id: folder_remote_id.as_deref(),
                        labels: labels.as_ref(),
                    },
                )
                .await?;
            Ok(RemoteIdUpdate::Unchanged)
        }
        MailOp::Archive(change) => {
            let source = folders.source(
                change.source_folder_id.as_deref(),
                change.source_folder_remote_id.as_deref(),
            );
            let archive = folders.target(
                change.target_id(),
                change.target_remote_id(),
                FolderRole::Archive,
            );
            remote
                .archive(
                    msg,
                    source.as_ref().map(ResolvedFolder::as_ref),
                    archive.as_ref().map(ResolvedFolder::as_ref),
                )
                .await
        }
        MailOp::Unarchive(change) | MailOp::Restore(change) => {
            let source = folders.source(
                change.source_folder_id.as_deref(),
                change.source_folder_remote_id.as_deref(),
            );
            let inbox = required(folders.target(
                change.target_id(),
                change.target_remote_id(),
                FolderRole::Inbox,
            ))?;
            remote
                .restore_to_inbox(
                    msg,
                    source.as_ref().map(ResolvedFolder::as_ref),
                    inbox.as_ref(),
                )
                .await
        }
        MailOp::MoveToFolder(change) => {
            let source = folders.source(
                change.source_folder_id.as_deref(),
                change.source_folder_remote_id.as_deref(),
            );
            let target = required(folders.target(
                change.target_id(),
                change.target_remote_id(),
                FolderRole::Inbox,
            ))?;
            let labels = recorded_labels(&change.add_labels, &change.remove_labels);
            remote
                .move_to_folder(
                    msg,
                    MoveRequest {
                        source: source.as_ref().map(ResolvedFolder::as_ref),
                        target: target.as_ref(),
                        labels: labels.as_ref(),
                        recover_previous_attempt: false,
                    },
                )
                .await
        }
        MailOp::Delete(delete) => {
            let source = folders.source(
                delete.source_folder_id.as_deref(),
                delete.source_folder_remote_id.as_deref(),
            );
            let trash = folders.target(
                delete.trash_folder_id.as_deref(),
                delete.trash_folder_remote_id.as_deref(),
                FolderRole::Trash,
            );
            remote
                .trash(
                    msg,
                    source.as_ref().map(ResolvedFolder::as_ref),
                    trash.as_ref().map(ResolvedFolder::as_ref),
                )
                .await
        }
        MailOp::DeletePermanent(delete) => {
            let source = folders.source(
                delete.source_folder_id.as_deref(),
                delete.source_folder_remote_id.as_deref(),
            );
            let result = remote
                .delete_permanently(msg, source.as_ref().map(ResolvedFolder::as_ref))
                .await;
            classify_remote_delete_result(result, true)?;
            Ok(RemoteIdUpdate::Unchanged)
        }
        MailOp::ManagedMove(_) | MailOp::Send(_) => Err(PebbleError::Internal(format!(
            "{} is not a plain remote mutation",
            op.op_type()
        ))),
    }
}

/// Store the message's new remote identity after a successful mutation.
pub(crate) fn persist_remote_id_update(
    state: &AppState,
    message: &Message,
    update: RemoteIdUpdate,
) -> Result<()> {
    match update {
        RemoteIdUpdate::Unchanged => Ok(()),
        RemoteIdUpdate::Global(remote_id) => state.store.update_remote_id(&message.id, &remote_id),
        // A folder-scoped id is only valid together with its folder: move
        // membership and id in one statement.
        RemoteIdUpdate::InFolder {
            folder_remote_id,
            remote_id,
        } => {
            let target_folder = state
                .store
                .list_folders(&message.account_id)?
                .into_iter()
                .find(|folder| folder.remote_id == folder_remote_id)
                .ok_or_else(|| PebbleError::Internal("IMAP target folder not found".into()))?;
            state.store.move_message_to_folder_with_remote_id(
                &message.id,
                &target_folder.id,
                &remote_id,
            )
        }
    }
}

/// Run `op` remotely and persist the resulting remote identity. Used by the
/// pending-op worker; failures are returned, never queued.
pub(crate) async fn execute_mail_op(
    state: &AppState,
    remote: &dyn RemoteMailOps,
    message: &Message,
    op: &MailOp,
) -> Result<()> {
    let update = execute_remote(state, remote, message, op).await?;
    persist_remote_id_update(state, message, update)
}

/// The outcome of [`RemoteSession::apply`], with the remote error when the
/// op was queued.
pub(crate) struct AppliedMutation {
    pub outcome: RemoteMutationOutcome,
    pub error: Option<String>,
}

/// One account's connection for a command, reused across messages.
pub(crate) enum RemoteSession {
    Connected(Box<dyn RemoteMailOps>),
    /// The provider could not be reached; ops are queued.
    Offline(String),
    /// The account has no remote mutations (POP3).
    LocalOnly,
}

impl RemoteSession {
    pub async fn open(state: &AppState, account_id: &str, provider_type: &ProviderType) -> Self {
        if matches!(provider_type, ProviderType::Pop3) {
            return Self::LocalOnly;
        }
        match connect_remote(state, account_id, provider_type).await {
            Ok(remote) => Self::Connected(remote),
            Err(error) => Self::Offline(error.to_string()),
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected(_))
    }

    /// Apply `op` remotely, or queue it:
    /// - unreachable provider: queued, and the caller may commit locally
    ///   unless the op cannot be reverted (permanent delete);
    /// - remote call failed: queued, and the caller must not commit locally.
    ///
    /// An error is returned only when local bookkeeping fails.
    pub async fn apply(
        &self,
        state: &AppState,
        message: &Message,
        op: MailOp,
    ) -> Result<AppliedMutation> {
        let (local_commit, error) = match self {
            Self::LocalOnly => {
                return Ok(AppliedMutation {
                    outcome: RemoteMutationOutcome::LocalOnly,
                    error: None,
                })
            }
            Self::Offline(error) => (op.commits_locally_when_offline(), error.clone()),
            Self::Connected(remote) => {
                match execute_remote(state, remote.as_ref(), message, &op).await {
                    Ok(update) => {
                        persist_remote_id_update(state, message, update)?;
                        return Ok(AppliedMutation {
                            outcome: RemoteMutationOutcome::Applied,
                            error: None,
                        });
                    }
                    Err(error) => (false, error.to_string()),
                }
            }
        };
        let outcome = queue_mail_op(&state.store, message, op, local_commit, Some(&error))?;
        Ok(AppliedMutation {
            outcome,
            error: Some(error),
        })
    }

    pub async fn close(self) {
        if let Self::Connected(remote) = self {
            remote.close().await;
        }
    }
}

/// Single-message helper for the message commands: apply `op` (unless
/// `local_only`), queue it on failure, and return an error unless the local
/// change may now be committed.
pub(crate) async fn apply_remote_or_queue(
    state: &AppState,
    message: &Message,
    provider_type: &ProviderType,
    op: MailOp,
    local_only: bool,
) -> Result<RemoteMutationOutcome> {
    if local_only {
        return Ok(RemoteMutationOutcome::LocalOnly);
    }
    let label = match op {
        MailOp::DeletePermanent(_) => "delete",
        ref op => op.op_type(),
    };
    let session = RemoteSession::open(state, &message.account_id, provider_type).await;
    let result = session.apply(state, message, op).await;
    session.close().await;
    let applied = result?;
    if !remote_mutation_allows_local_commit(applied.outcome) {
        return Err(queued_remote_error(
            label,
            applied.error.as_deref().unwrap_or("remote unavailable"),
        ));
    }
    Ok(applied.outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{Account, FolderType};
    use pebble_store::pending_ops::{FlagsOp, PendingMailOpStatus};

    fn state(provider: ProviderType) -> AppState {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_account(&Account {
                id: "a".into(),
                email: "test@example.invalid".into(),
                display_name: "Test".into(),
                account_label: None,
                provider_display_name: None,
                color: None,
                provider,
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        AppState::new(
            store,
            pebble_search::TantivySearch::open_in_memory().unwrap(),
            pebble_crypto::CryptoService::from_key([0; 32]),
            std::sync::mpsc::channel().0,
            std::env::temp_dir(),
        )
    }

    fn folder(id: &str, remote: &str, role: Option<FolderRole>) -> Folder {
        Folder {
            id: id.into(),
            account_id: "a".into(),
            remote_id: remote.into(),
            name: remote.into(),
            folder_type: FolderType::Folder,
            is_system: role.is_some(),
            role,
            parent_id: None,
            color: None,
            sort_order: 0,
        }
    }

    fn seed(state: &AppState) -> Message {
        for f in [
            folder("inbox", "INBOX", Some(FolderRole::Inbox)),
            folder("archive", "Archive", Some(FolderRole::Archive)),
            folder("trash", "Trash", Some(FolderRole::Trash)),
        ] {
            state.store.insert_folder(&f).unwrap();
        }
        let message: Message = serde_json::from_value(serde_json::json!({
            "id":"m","account_id":"a","remote_id":"5","message_id_header":"<m@example.invalid>",
            "in_reply_to":null,"references_header":null,"thread_id":null,"subject":"Test",
            "snippet":"","from_address":"test@example.invalid","from_name":"Test","to_list":[],
            "cc_list":[],"bcc_list":[],"body_text":"","body_html_raw":"","has_attachments":false,
            "is_read":false,"is_starred":false,"is_draft":false,"date":1,"remote_version":null,
            "is_deleted":false,"deleted_at":null,"created_at":1,"updated_at":1}))
        .unwrap();
        state
            .store
            .insert_message(&message, &["inbox".into()])
            .unwrap();
        message
    }

    fn flags(is_read: bool) -> MailOp {
        MailOp::UpdateFlags(FlagsOp {
            is_read: Some(is_read),
            folder_remote_id: Some("INBOX".into()),
            ..FlagsOp::default()
        })
    }

    #[tokio::test]
    async fn offline_session_queues_with_local_commit_and_merges_repeated_flag_changes() {
        // No stored IMAP credentials: connecting fails like being offline.
        let state = state(ProviderType::Imap);
        let message = seed(&state);
        let session = RemoteSession::open(&state, "a", &ProviderType::Imap).await;
        assert!(!session.is_connected());

        let first = session.apply(&state, &message, flags(true)).await.unwrap();
        assert_eq!(first.outcome, RemoteMutationOutcome::QueuedLocalCommit);
        assert!(first.error.is_some());
        state
            .store
            .update_message_flags("m", Some(true), None)
            .unwrap();
        let committed = state.store.get_message("m").unwrap().unwrap();
        let second = session
            .apply(&state, &committed, flags(false))
            .await
            .unwrap();
        assert_eq!(second.outcome, RemoteMutationOutcome::QueuedLocalCommit);

        let ops = state.store.list_pending_mail_ops("a").unwrap();
        assert_eq!(ops.len(), 1, "flag toggles collapse into one op");
        let envelope = ops[0].envelope().unwrap();
        assert_eq!(envelope.local_committed, Some(true));
        // The first commit captured the true pre-change state.
        assert_eq!(envelope.previous_state.unwrap().is_read, Some(false));
        let MailOp::UpdateFlags(merged) = envelope.op else {
            panic!("expected flags");
        };
        assert_eq!(merged.is_read, Some(false));
    }

    #[tokio::test]
    async fn offline_permanent_delete_waits_for_the_server() {
        let state = state(ProviderType::Imap);
        let message = seed(&state);
        let trash = folder("trash", "Trash", Some(FolderRole::Trash));
        let result = apply_remote_or_queue(
            &state,
            &message,
            &ProviderType::Imap,
            delete_op(Some(&trash), None, true),
            false,
        )
        .await;
        assert!(result.is_err(), "the local copy must not be deleted yet");
        let ops = state.store.list_pending_mail_ops("a").unwrap();
        assert_eq!(ops[0].op_type, "delete_permanent");
        assert_eq!(ops[0].status, PendingMailOpStatus::Failed);
        assert_eq!(ops[0].envelope().unwrap().local_committed, Some(false));
    }

    #[tokio::test]
    async fn archive_then_unarchive_while_offline_cancels_out() {
        let state = state(ProviderType::Imap);
        let message = seed(&state);
        let inbox = folder("inbox", "INBOX", Some(FolderRole::Inbox));
        let archive = folder("archive", "Archive", Some(FolderRole::Archive));
        let session = RemoteSession::open(&state, "a", &ProviderType::Imap).await;
        let archived = session
            .apply(
                &state,
                &message,
                MailOp::Archive(folder_change(Some(&inbox), Some(&archive))),
            )
            .await
            .unwrap();
        assert_eq!(archived.outcome, RemoteMutationOutcome::QueuedLocalCommit);
        let restored = session
            .apply(
                &state,
                &message,
                MailOp::Unarchive(folder_change(Some(&archive), Some(&inbox))),
            )
            .await
            .unwrap();
        assert_eq!(restored.outcome, RemoteMutationOutcome::LocalOnly);
        assert!(state
            .store
            .list_active_pending_mail_ops(Some("a"), 10)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn worker_replays_legacy_batch_archive_row_for_local_only_account() {
        let state = state(ProviderType::Pop3);
        seed(&state);
        // Written by the old batch_archive with a plain insert.
        state
            .store
            .insert_pending_mail_op(
                "a",
                "m",
                "archive",
                r#"{"provider_account_id":"a","remote_id":"5","op":"archive",
                    "payload":{"archive_folder_id":"archive","archive_folder_remote_id":"Archive"}}"#,
            )
            .unwrap();
        let completed = crate::commands::pending_mail_ops::process_pending_mail_ops(&state, None)
            .await
            .unwrap();
        assert_eq!(completed, 1);
        assert_eq!(
            state.store.get_message_folder_ids("m").unwrap(),
            vec!["archive".to_string()]
        );
    }

    #[test]
    fn local_only_policy_matches_provider_identity_rules() {
        let inbox = folder("inbox", "INBOX", Some(FolderRole::Inbox));
        let local = folder("local", "__local_archive__", None);
        let mut spam = folder("spam", "__local_spam", Some(FolderRole::Spam));
        assert!(move_is_local_only(&ProviderType::Imap, &inbox, &local));
        assert!(!move_is_local_only(&ProviderType::Gmail, &inbox, &spam));
        spam.role = None;
        assert!(move_is_local_only(&ProviderType::Gmail, &inbox, &spam));
        assert!(remote_ids_are_folder_scoped(&ProviderType::Imap));
        assert!(!remote_ids_are_folder_scoped(&ProviderType::Outlook));
        assert!(archives_without_folder(&ProviderType::Gmail));
    }
}
