//! Server-first folder edits and durable, account-scoped Move To operations.
use super::messages::{
    connect_gmail, connect_imap, connect_outlook, imap_move_tracked, refresh_search_document,
};
use crate::state::AppState;
use pebble_core::traits::{FolderProvider, LabelProvider};
use pebble_core::{new_id, Folder, FolderType, Message, PebbleError, ProviderType, Result};
use pebble_mail::imap::folder_management::encode_folder_segment;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};
use tauri::{Emitter, State};
use tokio::sync::Mutex;

static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
async fn account_lock(id: &str) -> Arc<Mutex<()>> {
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .await
        .entry(id.to_owned())
        .or_default()
        .clone()
}

// A live sync must not rediscover old names between server RENAME and local
// reconciliation. Mirrors sync_cmd's existing stop-and-await implementation.
async fn pause_sync(state: &AppState, account: &str) -> bool {
    if let Some(handle) = state.sync_handles.lock().await.remove(account) {
        let _ = handle.stop_tx.send(true);
        handle.task.abort();
        let _ = handle.task.await;
        true
    } else {
        false
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name != name.trim()
        || name.len() > 255
        || name.chars().any(char::is_control)
        || name == "."
        || name == ".."
    {
        return Err(PebbleError::Validation("Use a nonempty name, at most 255 bytes, without control characters or leading/trailing spaces".into()));
    }
    Ok(())
}

fn owned_folder(folders: &[Folder], id: &str) -> Result<Folder> {
    folders
        .iter()
        .find(|f| f.id == id)
        .cloned()
        .ok_or_else(|| PebbleError::Validation("Folder does not belong to this account".into()))
}

fn editable_folder(folder: &Folder) -> Result<()> {
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

fn collision(
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

#[tauri::command]
pub async fn create_mail_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    name: String,
    parent_id: Option<String>,
    parent_remote_id: Option<String>,
) -> Result<Folder> {
    validate_name(&name)?;
    let lock = account_lock(&account_id).await;
    let _guard = lock.lock().await;
    let restart = pause_sync(&state, &account_id).await;
    let result = create_folder_inner(
        &state,
        &account_id,
        &name,
        parent_id.as_deref(),
        parent_remote_id.as_deref(),
    )
    .await;
    if restart {
        let _ = super::sync_cmd::start_sync(app, state, account_id, None).await;
    }
    result
}

#[tauri::command]
pub async fn get_mail_folder_parents(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<pebble_mail::imap::folder_management::ImapFolderParent>> {
    let account = state
        .store
        .get_account(&account_id)?
        .ok_or_else(|| PebbleError::Validation("Account not found".into()))?;
    if account.provider != ProviderType::Imap {
        return Ok(Vec::new());
    }
    let imap = connect_imap(&state, &account_id).await?;
    let result = imap.folder_parents().await;
    let _ = imap.disconnect().await;
    result
}

async fn create_folder_inner(
    state: &AppState,
    account_id: &str,
    name: &str,
    parent_id: Option<&str>,
    parent_remote_id: Option<&str>,
) -> Result<Folder> {
    let account = state
        .store
        .get_account(account_id)?
        .ok_or_else(|| PebbleError::Validation("Account not found".into()))?;
    let folders = state.store.list_folders(account_id)?;
    let parent = parent_id.map(|id| owned_folder(&folders, id)).transpose()?;
    if parent_remote_id.is_some() && (account.provider != ProviderType::Imap || parent_id.is_some())
    {
        return Err(PebbleError::Validation(
            "A remote parent is only supported for IMAP hierarchy selection".into(),
        ));
    }
    if parent.as_ref().is_some_and(|p| {
        p.folder_type == FolderType::Category
            || (account.provider == ProviderType::Gmail && p.is_system)
    }) {
        return Err(PebbleError::UnsupportedProvider(
            "This folder cannot contain child folders".into(),
        ));
    }
    if parent.as_ref().is_some_and(|f| {
        f.remote_id.starts_with("__local_") && account.provider != ProviderType::Pop3
    }) {
        return Err(PebbleError::Validation(
            "Remote folders cannot be created inside a local folder".into(),
        ));
    }
    collision(&folders, None, name, parent_id, None)?;
    let mut created = match account.provider {
        ProviderType::Imap => {
            let imap = connect_imap(state, account_id).await?;
            let result: Result<Folder> = async {
                let available = imap.folder_parents().await?;
                let parent_remote = parent
                    .as_ref()
                    .map(|p| p.remote_id.as_str())
                    .or(parent_remote_id);
                let delimiter = if let Some(remote) = parent_remote {
                    available
                        .iter()
                        .find(|p| p.remote_id == remote)
                        .ok_or_else(|| {
                            PebbleError::Validation("Parent namespace no longer exists".into())
                        })?
                        .delimiter
                        .clone()
                } else {
                    imap.folder_delimiter(None).await?
                };
                let segment = encode_folder_segment(name, delimiter.as_deref())?;
                let remote = if let Some(parent_remote) = parent_remote {
                    let delimiter = delimiter.as_deref().ok_or_else(|| {
                        PebbleError::Validation(
                            "This IMAP namespace does not support child folders".into(),
                        )
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
                imap.create_folder(&remote).await?;
                // Tagged OK is the server receipt; no local row is created on failure.
                Ok(Folder {
                    id: new_id(),
                    account_id: account_id.into(),
                    name: pebble_mail::imap::decode_folder_name(&remote),
                    remote_id: remote,
                    folder_type: FolderType::Folder,
                    role: None,
                    parent_id: parent_id.or(parent_remote_id).map(str::to_owned),
                    color: None,
                    is_system: false,
                    sort_order: 100,
                })
            }
            .await;
            let _ = imap.disconnect().await;
            result?
        }
        ProviderType::Gmail => {
            if name.contains('/') {
                return Err(PebbleError::Validation(
                    "Use the parent picker to create a nested label".into(),
                ));
            }
            let full = parent
                .as_ref()
                .map_or_else(|| name.to_owned(), |p| format!("{}/{name}", p.name));
            let provider = connect_gmail(state, account_id).await?;
            collision(&provider.list_folders().await?, None, &full, None, None)?;
            provider.create_folder_label(&full).await?
        }
        ProviderType::Outlook => {
            let provider = connect_outlook(state, account_id).await?;
            let fresh = provider.list_folders().await?;
            collision(
                &fresh,
                None,
                name,
                parent.as_ref().map(|f| f.remote_id.as_str()),
                None,
            )?;
            provider
                .create_managed_folder(parent.as_ref().map(|f| f.remote_id.as_str()), name)
                .await?
        }
        ProviderType::Pop3 => Folder {
            id: new_id(),
            account_id: account_id.into(),
            remote_id: format!("__local_folder_{}", new_id()),
            name: name.into(),
            folder_type: FolderType::Folder,
            role: None,
            parent_id: parent_id.map(str::to_owned),
            color: None,
            is_system: false,
            sort_order: 100,
        },
    };
    created.account_id = account_id.into();
    created.parent_id = parent_id.or(parent_remote_id).map(str::to_owned);
    created.id = state.store.insert_folder(&created)?;
    if account.provider == ProviderType::Imap {
        state
            .store
            .include_managed_imap_folder(account_id, &created.remote_id)?;
    }
    Ok(created)
}

#[tauri::command]
pub async fn rename_mail_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    folder_id: String,
    name: String,
) -> Result<Vec<Folder>> {
    validate_name(&name)?;
    let lock = account_lock(&account_id).await;
    let _guard = lock.lock().await;
    let restart = pause_sync(&state, &account_id).await;
    let result = rename_folder_inner(&state, &account_id, &folder_id, &name).await;
    if restart {
        let _ = super::sync_cmd::start_sync(app, state, account_id, None).await;
    }
    result
}

async fn rename_folder_inner(
    state: &AppState,
    account_id: &str,
    folder_id: &str,
    name: &str,
) -> Result<Vec<Folder>> {
    let account = state
        .store
        .get_account(account_id)?
        .ok_or_else(|| PebbleError::Validation("Account not found".into()))?;
    if state
        .store
        .pending_mail_ops_summary(Some(account_id))?
        .total_active_count
        > 0
    {
        return Err(PebbleError::Validation(
            "Finish or dismiss pending mail operations before renaming folders".into(),
        ));
    }
    let folders = state.store.list_folders(account_id)?;
    let folder = owned_folder(&folders, folder_id)?;
    // Older IMAP rows marked every folder as system. The fresh LIST below is
    // authoritative for their attributes; roles/local system IDs stay guarded.
    if account.provider != ProviderType::Imap {
        editable_folder(&folder)?;
    } else if folder.role.is_some() || folder.remote_id.starts_with("__local_") {
        return Err(PebbleError::Validation(
            "System folders cannot be renamed".into(),
        ));
    }
    let changes = match account.provider {
        ProviderType::Imap => {
            let imap = connect_imap(state, account_id).await?;
            let result = async {
                let fresh = imap.list_folders(account_id).await?;
                let server = fresh
                    .iter()
                    .find(|f| f.remote_id == folder.remote_id)
                    .ok_or_else(|| {
                        PebbleError::Validation("Folder no longer exists on the server".into())
                    })?;
                editable_folder(server)?;
                let delimiter = imap.folder_delimiter(Some(&folder.remote_id)).await?;
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
                collision(&fresh, Some(&new_remote), &new_remote, None, None)?;
                let changes =
                    imap_rename_changes(&folders, &folder, &new_remote, delimiter.as_deref());
                for changed in &changes {
                    let prior = folders
                        .iter()
                        .find(|f| f.id == changed.id)
                        .expect("rename source exists");
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
                if fresh.iter().any(|f| {
                    prefix.as_ref().is_some_and(|p| f.remote_id.starts_with(p)) && f.is_system
                }) {
                    return Err(PebbleError::Validation(
                        "A hierarchy containing system folders cannot be renamed".into(),
                    ));
                }
                for changed in &changes {
                    collision(
                        &folders,
                        Some(&changed.remote_id),
                        &changed.name,
                        changed.parent_id.as_deref(),
                        Some(&changed.id),
                    )?;
                }
                imap.rename_folder(&folder.remote_id, &new_remote).await?;
                Ok(changes)
            }
            .await;
            let _ = imap.disconnect().await;
            result?
        }
        ProviderType::Gmail => {
            if name.contains('/') {
                return Err(PebbleError::Validation(
                    "A label name cannot contain '/' here".into(),
                ));
            }
            let provider = connect_gmail(state, account_id).await?;
            let fresh = provider.list_folders().await?;
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
            collision(&fresh, None, &full, None, Some(&server.id))?;
            let updated = provider
                .rename_folder_label(&folder.remote_id, &full)
                .await?;
            vec![Folder {
                name: updated.name,
                ..folder
            }]
        }
        ProviderType::Outlook => {
            let provider = connect_outlook(state, account_id).await?;
            let fresh = provider.list_folders().await?;
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
            collision(
                &fresh,
                None,
                name,
                server.parent_id.as_deref(),
                Some(&server.id),
            )?;
            provider
                .rename_managed_folder(&folder.remote_id, name)
                .await?;
            vec![Folder {
                name: name.into(),
                ..folder
            }]
        }
        ProviderType::Pop3 => {
            collision(
                &folders,
                None,
                name,
                folder.parent_id.as_deref(),
                Some(&folder.id),
            )?;
            vec![Folder {
                name: name.into(),
                ..folder
            }]
        }
    };
    state.store.reconcile_folder_rename(account_id, &changes)?;
    state.store.list_folders(account_id)
}

#[tauri::command]
pub async fn rename_mail_folder_namespace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    remote_id: String,
    name: String,
) -> Result<Vec<Folder>> {
    validate_name(&name)?;
    let lock = account_lock(&account_id).await;
    let _guard = lock.lock().await;
    let restart = pause_sync(&state, &account_id).await;
    let result = rename_namespace_inner(&state, &account_id, &remote_id, &name).await;
    if restart {
        let _ = super::sync_cmd::start_sync(app, state, account_id, None).await;
    }
    result
}

async fn rename_namespace_inner(
    state: &AppState,
    account_id: &str,
    remote_id: &str,
    name: &str,
) -> Result<Vec<Folder>> {
    let account = state
        .store
        .get_account(account_id)?
        .ok_or_else(|| PebbleError::Validation("Account not found".into()))?;
    if account.provider != ProviderType::Imap
        || state
            .store
            .pending_mail_ops_summary(Some(account_id))?
            .total_active_count
            > 0
    {
        return Err(PebbleError::Validation(
            "Only idle IMAP namespaces can be renamed".into(),
        ));
    }
    let imap = connect_imap(state, account_id).await?;
    let result = async {
        let parents = imap.folder_parents().await?;
        let source = parents
            .iter()
            .find(|p| p.remote_id == remote_id && !p.selectable)
            .ok_or_else(|| PebbleError::Validation("Nonselectable namespace not found".into()))?;
        if source.is_system {
            return Err(PebbleError::Validation(
                "System folders cannot be renamed".into(),
            ));
        }
        let delimiter = source.delimiter.as_deref().ok_or_else(|| {
            PebbleError::Validation("Flat namespaces cannot be renamed here".into())
        })?;
        let segment = encode_folder_segment(name, Some(delimiter))?;
        let new_remote = remote_id.rsplit_once(delimiter).map_or_else(
            || segment.clone(),
            |(p, _)| format!("{p}{delimiter}{segment}"),
        );
        let prefix = format!("{remote_id}{delimiter}");
        if parents
            .iter()
            .any(|p| p.remote_id.starts_with(&prefix) && p.is_system)
        {
            return Err(PebbleError::Validation(
                "A hierarchy containing system folders cannot be renamed".into(),
            ));
        }
        for p in parents
            .iter()
            .filter(|p| p.remote_id == remote_id || p.remote_id.starts_with(&prefix))
        {
            let new = format!("{new_remote}{}", &p.remote_id[remote_id.len()..]);
            if parents
                .iter()
                .any(|f| f.remote_id.eq_ignore_ascii_case(&new))
            {
                return Err(PebbleError::Validation(
                    "A folder with that name already exists".into(),
                ));
            }
        }
        let folders = state.store.list_folders(account_id)?;
        let source_folder = Folder {
            id: String::new(),
            account_id: account_id.into(),
            remote_id: remote_id.into(),
            name: source.name.clone(),
            folder_type: FolderType::Folder,
            role: None,
            parent_id: None,
            color: None,
            is_system: false,
            sort_order: 0,
        };
        let mut changes =
            imap_rename_changes(&folders, &source_folder, &new_remote, Some(delimiter));
        for child in &mut changes {
            if child.parent_id.as_deref() == Some(remote_id) {
                child.parent_id = Some(new_remote.clone());
            }
        }
        imap.rename_folder(remote_id, &new_remote).await?;
        state.store.reconcile_folder_rename(account_id, &changes)?;
        state.store.list_folders(account_id)
    }
    .await;
    let _ = imap.disconnect().await;
    result
}

fn imap_rename_changes(
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
                name: pebble_mail::imap::decode_folder_name(&remote_id),
                remote_id,
                ..f.clone()
            }
        })
        .collect()
}

#[derive(Serialize)]
pub struct MoveFailure {
    pub message_id: String,
    pub error: String,
}
#[derive(Default, Serialize)]
pub struct MoveResult {
    pub moved_ids: Vec<String>,
    pub failures: Vec<MoveFailure>,
}

#[derive(Serialize)]
pub struct MailMoveContext {
    pub provider: ProviderType,
    pub folder_ids_by_message: HashMap<String, Vec<String>>,
}

#[tauri::command]
pub fn get_mail_move_context(
    state: State<'_, AppState>,
    account_id: String,
    message_ids: Vec<String>,
) -> Result<MailMoveContext> {
    if message_ids.is_empty() || message_ids.len() > 500 {
        return Err(PebbleError::Validation(
            "Select between 1 and 500 messages".into(),
        ));
    }
    let provider = state
        .store
        .get_account(&account_id)?
        .ok_or_else(|| PebbleError::Validation("Account not found".into()))?
        .provider;
    let mut folder_ids_by_message = HashMap::new();
    for id in message_ids {
        let message = state
            .store
            .get_message(&id)?
            .ok_or_else(|| PebbleError::Validation("Message not found".into()))?;
        if message.account_id != account_id {
            return Err(PebbleError::Validation(
                "Cross-account moves are not supported".into(),
            ));
        }
        folder_ids_by_message.insert(id.clone(), state.store.get_message_folder_ids(&id)?);
    }
    Ok(MailMoveContext {
        provider,
        folder_ids_by_message,
    })
}

#[tauri::command]
pub async fn move_mail_messages(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    message_ids: Vec<String>,
    source_folder_id: Option<String>,
    target_folder_id: String,
) -> Result<MoveResult> {
    let lock = account_lock(&account_id).await;
    let _guard = lock.lock().await;
    let restart = pause_sync(&state, &account_id).await;
    let result = move_messages_inner(
        &state,
        &account_id,
        message_ids,
        source_folder_id,
        target_folder_id,
    )
    .await;
    let _ = app.emit(crate::events::MAIL_PENDING_OPS_CHANGED, ());
    if restart {
        let _ = super::sync_cmd::start_sync(app, state, account_id, None).await;
    }
    result
}

async fn move_messages_inner(
    state: &AppState,
    account_id: &str,
    message_ids: Vec<String>,
    source_folder_id: Option<String>,
    target_folder_id: String,
) -> Result<MoveResult> {
    if message_ids.is_empty() || message_ids.len() > 500 {
        return Err(PebbleError::Validation(
            "Select between 1 and 500 messages".into(),
        ));
    }
    let folders = state.store.list_folders(account_id)?;
    let target = owned_folder(&folders, &target_folder_id)?;
    if target.role == Some(pebble_core::FolderRole::Drafts)
        || target.remote_id == "__local_outbox__"
        || target.folder_type == FolderType::Category
    {
        return Err(PebbleError::Validation(
            "This folder cannot receive moved messages".into(),
        ));
    }
    // Validate the entire batch account boundary before any remote side effects.
    let mut messages = Vec::new();
    for id in message_ids
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
    {
        let msg = state
            .store
            .get_message(&id)?
            .ok_or_else(|| PebbleError::Validation("Message not found".into()))?;
        if msg.account_id != account_id {
            return Err(PebbleError::Validation(
                "Cross-account moves are not supported".into(),
            ));
        }
        messages.push(msg);
    }
    let mut result = MoveResult::default();
    for message in messages {
        let outcome = async {
            if message.is_draft { return Err(PebbleError::Validation("Drafts cannot be moved; finish editing first".into())); }
            let memberships = state.store.get_message_folder_ids(&message.id)?;
            if memberships.contains(&target.id) { return Err(PebbleError::Validation("Message is already in this folder".into())); }
            let source = if let Some(id) = source_folder_id.as_deref() {
                let source = owned_folder(&folders,id)?;
                if !memberships.contains(&source.id) { return Err(PebbleError::Validation("Message is not in the selected source folder".into())); }
                source
            } else {
                // A thread/search can have multiple labels; make the source explicit
                // instead of silently removing an arbitrary label.
                if memberships.len() != 1 { return Err(PebbleError::Validation("Choose a source folder for messages with multiple labels".into())); }
                owned_folder(&folders,&memberships[0])?
            };
            let payload = json!({"source_folder_id":source.id,"target_folder_id":target.id,"source_folder_remote_id":source.remote_id,"target_folder_remote_id":target.remote_id});
            validate_move_route(state, &message, &source, &target)?;
            let envelope = json!({"provider_account_id":account_id,"remote_id":message.remote_id,"payload":payload});
            let op_id = state.store.stage_managed_move(account_id,&message.id,&envelope.to_string())?;
            match replay_managed_move(state,&message,&payload).await {
                Ok(()) => { state.store.mark_pending_mail_op_done(&op_id)?; Ok(()) },
                Err(error) => {
                    state.store.mark_pending_mail_op_failed(&op_id,&error.to_string())?;
                    Err(PebbleError::Network(format!("Move failed and is queued for retry: {error}")))
                }
            }
        }.await;
        match outcome {
            Ok(()) => result.moved_ids.push(message.id),
            Err(error) => result.failures.push(MoveFailure {
                message_id: message.id,
                error: error.to_string(),
            }),
        }
    }
    Ok(result)
}

fn validate_move_route(
    state: &AppState,
    message: &Message,
    source: &Folder,
    target: &Folder,
) -> Result<ProviderType> {
    if message.is_draft
        || target.role == Some(pebble_core::FolderRole::Drafts)
        || target.folder_type == FolderType::Category
        || target.remote_id == "__local_outbox__"
    {
        return Err(PebbleError::Validation(
            "Drafts, Outbox and categories cannot be moved with Move To".into(),
        ));
    }
    if source.account_id != message.account_id
        || target.account_id != message.account_id
        || source.id == target.id
    {
        return Err(PebbleError::Validation(
            "Invalid or cross-account move".into(),
        ));
    }
    let provider = state
        .store
        .get_account(&message.account_id)?
        .ok_or_else(|| PebbleError::Validation("Account not found".into()))?
        .provider;
    if provider == ProviderType::Gmail && !target.remote_id.starts_with("__local_") {
        let mutable = |f: &Folder| {
            !f.is_system
                || matches!(
                    f.role,
                    Some(
                        pebble_core::FolderRole::Inbox
                            | pebble_core::FolderRole::Spam
                            | pebble_core::FolderRole::Trash
                    )
                )
        };
        if !mutable(source) || !mutable(target) {
            return Err(PebbleError::UnsupportedProvider(
                "Gmail permits moving only between user labels, Inbox, Spam and Trash".into(),
            ));
        }
    }
    if provider != ProviderType::Pop3
        && source.remote_id.starts_with("__local_") != target.remote_id.starts_with("__local_")
    {
        return Err(PebbleError::UnsupportedProvider(
            "Move To cannot transfer messages between server and device-only folders".into(),
        ));
    }
    Ok(provider)
}

/// Called by the pending-op worker too. Completes membership and remote identity
/// together, so replay sees an already-completed move rather than a UID neighbor.
pub async fn replay_managed_move(
    state: &AppState,
    message: &Message,
    payload: &Value,
) -> Result<()> {
    let field = |key| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| PebbleError::Validation("Invalid move payload".into()))
    };
    let folders = state.store.list_folders(&message.account_id)?;
    let source = owned_folder(&folders, field("source_folder_id")?)?;
    let target = owned_folder(&folders, field("target_folder_id")?)?;
    let provider = validate_move_route(state, message, &source, &target)?;
    let memberships = state.store.get_message_folder_ids(&message.id)?;
    if memberships.contains(&target.id) && !memberships.contains(&source.id) {
        return Ok(());
    }
    if !memberships.contains(&source.id) {
        return Err(PebbleError::Validation(
            "The source folder changed while this move was pending".into(),
        ));
    }
    let local = provider == ProviderType::Pop3
        || source.remote_id.starts_with("__local_")
        || target.remote_id.starts_with("__local_");
    if local {
        state
            .store
            .move_message_to_folder(&message.id, &target.id)?;
    } else {
        match provider {
            ProviderType::Imap => {
                let uid = message
                    .remote_id
                    .parse::<u32>()
                    .map_err(|_| PebbleError::Validation("Invalid IMAP UID".into()))?;
                let imap = connect_imap(state, &message.account_id).await?;
                let result = imap_move_tracked(
                    state,
                    &imap,
                    message,
                    &source.remote_id,
                    uid,
                    &target.remote_id,
                )
                .await;
                let _ = imap.disconnect().await;
                result?;
            }
            ProviderType::Outlook => {
                let outlook = connect_outlook(state, &message.account_id).await?;
                let id = if let Some(header) = message
                    .message_id_header
                    .as_deref()
                    .filter(|h| !h.trim().is_empty())
                {
                    match outlook
                        .recover_managed_move(&message.remote_id, &target.remote_id, header)
                        .await?
                    {
                        Some(id) => id,
                        None => {
                            outlook
                                .move_message(&message.remote_id, &target.remote_id)
                                .await?
                        }
                    }
                } else {
                    outlook
                        .move_message(&message.remote_id, &target.remote_id)
                        .await?
                };
                state
                    .store
                    .move_message_to_folder_with_remote_id(&message.id, &target.id, &id)?;
            }
            ProviderType::Gmail => {
                // Remove exactly the source label; labels on other messages and the
                // other labels on this message stay unchanged.
                connect_gmail(state, &message.account_id)
                    .await?
                    .modify_labels(
                        &message.remote_id,
                        std::slice::from_ref(&target.remote_id),
                        std::slice::from_ref(&source.remote_id),
                    )
                    .await?;
                state
                    .store
                    .move_message_label_membership(&message.id, &source.id, &target.id)?;
            }
            ProviderType::Pop3 => unreachable!(),
        }
    }
    refresh_search_document(state, &message.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn folder(id: &str, remote: &str) -> Folder {
        Folder {
            id: id.into(),
            account_id: "a".into(),
            remote_id: remote.into(),
            name: remote.into(),
            folder_type: FolderType::Folder,
            role: None,
            parent_id: None,
            color: None,
            is_system: false,
            sort_order: 0,
        }
    }
    #[test]
    fn rename_subtree_uses_delimiter_boundaries_and_keeps_ids() {
        let source = folder("parent", "A");
        let folders = vec![
            source.clone(),
            folder("child", "A.Child"),
            folder("neighbor", "AB.Child"),
        ];
        let changed = imap_rename_changes(&folders, &source, "B", Some("."));
        assert_eq!(changed.len(), 2);
        assert_eq!(changed[1].id, "child");
        assert_eq!(changed[1].remote_id, "B.Child");
    }
    #[test]
    fn invalid_names_and_system_rename_are_rejected() {
        for name in ["", " ", ".", "..", "x\nCREATE other"] {
            assert!(validate_name(name).is_err());
        }
        let mut system = folder("inbox", "INBOX");
        system.is_system = true;
        assert!(editable_folder(&system).is_err());
        assert!(owned_folder(&[folder("a", "A")], "other-account-id").is_err());
    }

    fn state(provider: ProviderType) -> AppState {
        let store = pebble_store::Store::open_in_memory().unwrap();
        store
            .insert_account(&pebble_core::Account {
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
    fn message(id: &str) -> Message {
        serde_json::from_value(json!({"id":id,"account_id":"a","remote_id":"5","message_id_header":"<move@example.invalid>","in_reply_to":null,"references_header":null,"thread_id":null,"subject":"Test","snippet":"","from_address":"test@example.invalid","from_name":"Test","to_list":[],"cc_list":[],"bcc_list":[],"body_text":"cached content","body_html_raw":"","has_attachments":false,"is_read":false,"is_starred":false,"is_draft":false,"date":1,"remote_version":null,"is_deleted":false,"deleted_at":null,"created_at":1,"updated_at":1})).unwrap()
    }
    fn seed(state: &AppState) {
        state
            .store
            .insert_folder(&folder("source", "Source"))
            .unwrap();
        state
            .store
            .insert_folder(&folder("target", "Target"))
            .unwrap();
        state
            .store
            .insert_message(&message("moving"), &["source".into()])
            .unwrap();
        let mut neighbor = message("neighbor");
        neighbor.message_id_header = Some("<neighbor@example.invalid>".into());
        if state.store.get_account("a").unwrap().unwrap().provider != ProviderType::Imap {
            neighbor.remote_id = "9".into();
        }
        state
            .store
            .insert_message(&neighbor, &["target".into()])
            .unwrap();
    }
    // Small loopback protocol fixture. Production credentials/network are never used.
    fn imap_fixture(
        state: &AppState,
        already_moved: bool,
        reject_create: bool,
    ) -> (std::thread::JoinHandle<Vec<String>>, u16) {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = json!({"host":"127.0.0.1","port":port,"username":"test","password":"fixture","security":"plain","accept_invalid_certs":false});
        super::super::encrypted_store::store_account_auth_data(
            &state.crypto,
            &state.store,
            "a",
            &serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(15)))
                .unwrap();
            stream
                .write_all(b"* OK IMAP4rev1 fixture ready\r\n")
                .unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut moved = already_moved;
            let mut selected = String::new();
            let mut requests = Vec::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let tag = line.split_whitespace().next().unwrap();
                let command = line.split_once(' ').unwrap().1.trim();
                if !command.starts_with("LOGIN") {
                    requests.push(command.to_owned());
                }
                let mut data = String::new();
                let mut status = "OK";
                if command.starts_with("SELECT") {
                    selected = command.to_owned();
                    data = "* 2 EXISTS\r\n* OK [UIDVALIDITY 1] stable\r\n".into();
                } else if command.starts_with("UID SEARCH") {
                    let hit = if selected.contains("Source") && !moved {
                        " 5"
                    } else if selected.contains("Target") && moved {
                        " 2"
                    } else {
                        ""
                    };
                    data = format!("* SEARCH{hit}\r\n");
                } else if command.starts_with("UID MOVE") {
                    moved = true;
                } else if command == "LIST \"\" \"\"" {
                    data = "* LIST (\\Noselect) \".\" \"\"\r\n".into();
                } else if command == "LIST \"\"" {
                    status = "BAD";
                } else if command.starts_with("LIST") {
                    data="* LIST () \".\" \"Source\"\r\n* LIST () \".\" \"Source.Child\"\r\n* LIST () \".\" \"Project Space\"\r\n* LIST () \".\" \"Target\"\r\n* LIST (\\Noselect) \".\" \"Container\"\r\n* LIST () \".\" \"Container.Child\"\r\n".into();
                } else if command.starts_with("CREATE") && reject_create {
                    status = "NO";
                } else if command.starts_with("LOGOUT") {
                    data = "* BYE logout\r\n".into();
                }
                stream
                    .write_all(format!("{data}{tag} {status} fixture\r\n").as_bytes())
                    .unwrap();
                if command.starts_with("LOGOUT") {
                    break;
                }
            }
            requests
        });
        (handle, port)
    }
    #[tokio::test]
    async fn move_command_tracks_destination_uid_without_editing_neighbor() {
        let state = state(ProviderType::Imap);
        seed(&state);
        let (server, _) = imap_fixture(&state, false, false);
        let result = move_messages_inner(
            &state,
            "a",
            vec!["moving".into()],
            Some("source".into()),
            "target".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.moved_ids, vec!["moving"]);
        assert!(result.failures.is_empty());
        assert_eq!(
            state
                .store
                .get_message("moving")
                .unwrap()
                .unwrap()
                .remote_id,
            "2"
        );
        assert_eq!(
            state
                .store
                .get_message("neighbor")
                .unwrap()
                .unwrap()
                .remote_id,
            "5"
        );
        assert_eq!(
            state.store.get_message_folder_ids("neighbor").unwrap(),
            vec!["target"]
        );
        assert_eq!(
            state
                .store
                .pending_mail_ops_summary(Some("a"))
                .unwrap()
                .total_active_count,
            0
        );
        let requests = server.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|s| s.starts_with("UID MOVE"))
                .count(),
            1
        );
    }
    #[tokio::test]
    async fn worker_replays_remote_success_with_new_uid_and_is_idempotent() {
        let state = state(ProviderType::Imap);
        seed(&state);
        let (server, _) = imap_fixture(&state, true, false);
        let payload = json!({"source_folder_id":"source","target_folder_id":"target","source_folder_remote_id":"Source","target_folder_remote_id":"Target"});
        state
            .store
            .insert_pending_mail_op(
                "a",
                "moving",
                "managed_move",
                &json!({"provider_account_id":"a","remote_id":"5","payload":payload}).to_string(),
            )
            .unwrap();
        assert_eq!(
            super::super::pending_mail_ops::process_pending_mail_ops(&state, None)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            state
                .store
                .get_message("moving")
                .unwrap()
                .unwrap()
                .remote_id,
            "2"
        );
        assert_eq!(
            state
                .store
                .get_message("neighbor")
                .unwrap()
                .unwrap()
                .remote_id,
            "5"
        );
        replay_managed_move(
            &state,
            &state.store.get_message("moving").unwrap().unwrap(),
            &payload,
        )
        .await
        .unwrap();
        assert_eq!(
            super::super::pending_mail_ops::process_pending_mail_ops(&state, None)
                .await
                .unwrap(),
            0
        );
        assert!(!server
            .join()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("UID MOVE")));
    }
    #[tokio::test]
    async fn batch_keeps_failures_and_rejects_cross_account_before_moving() {
        let state = state(ProviderType::Pop3);
        seed(&state);
        let mut draft = message("draft");
        draft.is_draft = true;
        draft.remote_id = "6".into();
        state
            .store
            .insert_message(&draft, &["source".into()])
            .unwrap();
        let result = move_messages_inner(
            &state,
            "a",
            vec!["moving".into(), "draft".into()],
            Some("source".into()),
            "target".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.moved_ids, vec!["moving"]);
        assert_eq!(result.failures[0].message_id, "draft");
        assert_eq!(
            state.store.get_message_folder_ids("draft").unwrap(),
            vec!["source"]
        );
        let mut foreign = state.store.get_account("a").unwrap().unwrap();
        foreign.id = "b".into();
        foreign.email = "other@example.invalid".into();
        state.store.insert_account(&foreign).unwrap();
        let mut other = folder("foreign", "Other");
        other.account_id = "b".into();
        state.store.insert_folder(&other).unwrap();
        let mut msg = message("foreign-message");
        msg.account_id = "b".into();
        state
            .store
            .insert_message(&msg, &["foreign".into()])
            .unwrap();
        assert!(move_messages_inner(
            &state,
            "a",
            vec!["draft".into(), "foreign-message".into()],
            None,
            "target".into()
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn root_create_uses_empty_quoted_namespace_and_modified_utf7() {
        let state = state(ProviderType::Imap);
        let (server, _) = imap_fixture(&state, false, false);
        let folder = create_folder_inner(&state, "a", "收件 项目", None, None)
            .await
            .unwrap();
        assert_eq!(
            folder.remote_id,
            encode_folder_segment("收件 项目", Some(".")).unwrap()
        );
        assert!(folder.parent_id.is_none());
        let requests = server.join().unwrap();
        assert!(requests.iter().any(|request| request == "LIST \"\" \"\""));
        assert!(requests
            .iter()
            .any(|request| request == &format!("CREATE \"{}\"", folder.remote_id)));
    }

    #[tokio::test]
    async fn spaced_mailbox_rename_quotes_the_exact_list_pattern() {
        let state = state(ProviderType::Imap);
        state
            .store
            .insert_folder(&folder("spaced", "Project Space"))
            .unwrap();
        let (server, _) = imap_fixture(&state, false, false);
        let folders = rename_folder_inner(&state, "a", "spaced", "Renamed Space")
            .await
            .unwrap();
        assert!(folders
            .iter()
            .any(|folder| folder.id == "spaced" && folder.remote_id == "Renamed Space"));
        assert!(server
            .join()
            .unwrap()
            .iter()
            .any(|request| request == "LIST \"\" \"Project Space\""));
    }

    #[tokio::test]
    async fn server_rejected_create_never_inserts_phantom_folder() {
        let state = state(ProviderType::Imap);
        let (server, _) = imap_fixture(&state, false, true);
        assert!(create_folder_inner(&state, "a", "New", None, None)
            .await
            .is_err());
        assert!(state.store.list_folders("a").unwrap().is_empty());
        server.join().unwrap();
    }
    #[tokio::test]
    async fn subtree_rename_keeps_cached_child_identity_and_body() {
        let state = state(ProviderType::Imap);
        seed(&state);
        let mut child = folder("child", "Source.Child");
        child.parent_id = Some("Source".into());
        state.store.insert_folder(&child).unwrap();
        let mut cached = message("child-message");
        cached.remote_id = "8".into();
        state
            .store
            .insert_message(&cached, &["child".into()])
            .unwrap();
        state
            .store
            .update_sync_state("a", |s| {
                s.selected_imap_folder_remote_ids = Some(vec!["Source.Child".into()])
            })
            .unwrap();
        let (server, _) = imap_fixture(&state, false, false);
        let folders = rename_folder_inner(&state, "a", "source", "Renamed")
            .await
            .unwrap();
        assert!(folders.iter().any(|f| f.id == "child"
            && f.remote_id == "Renamed.Child"
            && f.parent_id.as_deref() == Some("source")));
        assert_eq!(
            state
                .store
                .get_message("child-message")
                .unwrap()
                .unwrap()
                .body_text,
            "cached content"
        );
        assert_eq!(
            state
                .store
                .get_sync_state("a")
                .unwrap()
                .unwrap()
                .selected_imap_folder_remote_ids
                .unwrap(),
            vec!["Renamed.Child"]
        );
        assert!(server
            .join()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("RENAME")));
    }
    #[tokio::test]
    async fn nonselectable_namespace_can_contain_new_folders_and_rename_cached_children() {
        let state = state(ProviderType::Imap);
        let (server, _) = imap_fixture(&state, false, false);
        let created = create_folder_inner(&state, "a", "New", None, Some("Container"))
            .await
            .unwrap();
        assert_eq!(created.remote_id, "Container.New");
        assert_eq!(created.parent_id.as_deref(), Some("Container"));
        server.join().unwrap();
        let (server, _) = imap_fixture(&state, false, false);
        let folders = rename_namespace_inner(&state, "a", "Container", "Renamed")
            .await
            .unwrap();
        assert!(folders.iter().any(|f| f.id == created.id
            && f.remote_id == "Renamed.New"
            && f.parent_id.as_deref() == Some("Renamed")));
        server.join().unwrap();
    }
    #[test]
    fn local_boundary_system_destinations_and_noops_are_guarded() {
        let state = state(ProviderType::Gmail);
        let message = message("m");
        let remote = folder("remote", "INBOX");
        let local = folder("local", "__local_folder_custom");
        assert!(validate_move_route(&state, &message, &local, &remote).is_err());
        assert!(validate_move_route(&state, &message, &remote, &local).is_err());
        assert!(validate_move_route(&state, &message, &remote, &remote).is_err());
        let mut sent = folder("sent", "SENT");
        sent.is_system = true;
        sent.role = Some(pebble_core::FolderRole::Sent);
        assert!(validate_move_route(&state, &message, &remote, &sent).is_err());
    }
}
