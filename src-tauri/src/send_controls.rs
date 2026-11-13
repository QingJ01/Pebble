use crate::commands::compose::{
    dispatch_send_email, stage_compose_attachment_bytes, validate_attachment_paths,
    SendDispatchOutcome,
};
use crate::state::AppState;
use base64::{engine::general_purpose::STANDARD, Engine};
use pebble_core::{now_timestamp, EmailAddress, PebbleError, Result};
use pebble_store::{scheduled_sends::ScheduledSend, Store};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, sync::Mutex, time::Duration};
use tauri::{Emitter, Manager, State};

const PURPOSE: &str = "scheduled_sends.payload";
const MAX_BYTES: usize = 25 * 1024 * 1024;
pub const CHANGED: &str = "send-controls-changed";
pub const EXIT_REQUESTED: &str = "send-controls-exit-requested";

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotAttachment {
    pub name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SendSnapshot {
    pub account_id: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
    pub editor_mode: String,
    pub compose_mode: String,
    pub raw_source: String,
    pub rich_text_html: String,
    pub quoted_reply_html: String,
    pub attachments: Vec<SnapshotAttachment>,
}

#[derive(Serialize, Deserialize, PartialEq)]
struct SealedAttachment {
    name: String,
    data: String,
}

#[derive(Serialize, Deserialize, PartialEq)]
struct SealedSend {
    snapshot: SendSnapshot,
    attachments: Vec<SealedAttachment>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendSummary {
    id: String,
    account_id: String,
    subject: String,
    due_at: i64,
    state: String,
    kind: String,
    revision: i64,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum ExitGate {
    #[default]
    Running,
    Prompting,
    Approved,
}

#[derive(Default)]
pub struct SendControlRuntime {
    gate: Mutex<ExitGate>,
}

fn conflict() -> PebbleError {
    PebbleError::Validation(
        "Message changed or is already sending; refresh and check Sent before trying again".into(),
    )
}
fn invalid_data(_: impl std::fmt::Display) -> PebbleError {
    PebbleError::Internal("Could not read or persist the scheduled message".into())
}
fn ensure_running(gate: &ExitGate) -> Result<()> {
    if *gate != ExitGate::Running {
        return Err(PebbleError::Validation(
            "An exit confirmation is pending".into(),
        ));
    }
    Ok(())
}

fn seal(state: &AppState, id: &str, mut snapshot: SendSnapshot) -> Result<Vec<u8>> {
    if !matches!(snapshot.editor_mode.as_str(), "rich" | "markdown" | "html")
        || !matches!(
            snapshot.compose_mode.as_str(),
            "new" | "reply" | "reply-all" | "forward"
        )
        || snapshot.to.is_empty()
        || snapshot.attachments.len() > 64
    {
        return Err(PebbleError::Validation("Invalid send snapshot".into()));
    }
    if state.store.get_account(&snapshot.account_id)?.is_none() {
        return Err(PebbleError::Validation(
            "Sender account no longer exists".into(),
        ));
    }
    for address in snapshot.to.iter().chain(&snapshot.cc).chain(&snapshot.bcc) {
        pebble_mail::sender::sender_mailbox(&EmailAddress {
            name: None,
            address: address.clone(),
        })?;
    }
    let paths = validate_attachment_paths(
        &snapshot
            .attachments
            .iter()
            .map(|a| a.path.clone())
            .collect::<Vec<_>>(),
        &state.attachments_dir,
    )?;
    let mut bytes_total = serde_json::to_vec(&snapshot).map_err(invalid_data)?.len();
    if bytes_total > MAX_BYTES {
        return Err(PebbleError::Validation(
            "Scheduled message is too large (25 MiB limit)".into(),
        ));
    }
    let mut attachments = Vec::new();
    for (path, attachment) in paths.iter().zip(&snapshot.attachments) {
        let file = std::fs::File::open(path).map_err(invalid_data)?;
        if !file.metadata().map_err(invalid_data)?.is_file() {
            return Err(PebbleError::Validation(
                "Attachment must be a regular file".into(),
            ));
        }
        let mut bytes = Vec::new();
        file.take((MAX_BYTES - bytes_total + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(invalid_data)?;
        bytes_total += bytes.len();
        if bytes_total > MAX_BYTES {
            return Err(PebbleError::Validation(
                "Scheduled message is too large (25 MiB limit)".into(),
            ));
        }
        attachments.push(SealedAttachment {
            name: attachment.name.clone(),
            data: STANDARD.encode(bytes),
        });
    }
    snapshot.attachments.clear();
    state.crypto.encrypt_for(
        PURPOSE,
        id,
        &serde_json::to_vec(&SealedSend {
            snapshot,
            attachments,
        })
        .map_err(invalid_data)?,
    )
}

fn unseal(state: &AppState, job: &ScheduledSend) -> Result<SealedSend> {
    let bytes = state.crypto.decrypt_for(PURPOSE, &job.id, &job.payload)?;
    let sealed: SealedSend = serde_json::from_slice(&bytes).map_err(invalid_data)?;
    if sealed.snapshot.account_id != job.account_id {
        return Err(PebbleError::Validation(
            "Scheduled sender does not match".into(),
        ));
    }
    Ok(sealed)
}

fn summary(state: &AppState, job: &ScheduledSend) -> Result<SendSummary> {
    let sealed = unseal(state, job)?;
    Ok(SendSummary {
        id: job.id.clone(),
        account_id: job.account_id.clone(),
        subject: sealed.snapshot.subject,
        due_at: job.due_at,
        state: job.state.clone(),
        kind: job.kind.clone(),
        revision: job.revision,
    })
}

fn stage_snapshot(root: &Path, mut sealed: SealedSend) -> Result<SendSnapshot> {
    let mut paths = Vec::new();
    let result = (|| {
        for attachment in sealed.attachments {
            let bytes = STANDARD
                .decode(attachment.data)
                .map_err(|_| PebbleError::Validation("Invalid encrypted attachment".into()))?;
            let path = stage_compose_attachment_bytes(root, &attachment.name, &bytes)?;
            paths.push(path.clone());
            sealed.snapshot.attachments.push(SnapshotAttachment {
                name: attachment.name,
                path: path.to_string_lossy().into_owned(),
                size: bytes.len() as u64,
            });
        }
        Ok(sealed.snapshot)
    })();
    if result.is_err() {
        cleanup_paths(root, &paths);
    }
    result
}

fn cleanup_paths(root: &Path, paths: &[std::path::PathBuf]) {
    for path in paths {
        let _ = crate::commands::compose::cleanup_staged_compose_attachment_path(root, path);
    }
}

#[tauri::command]
pub async fn queue_controlled_send(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    runtime: State<'_, SendControlRuntime>,
    snapshot: SendSnapshot,
    request_id: String,
    due_at: Option<i64>,
    undo_seconds: Option<u32>,
) -> Result<SendSummary> {
    let result = queue_snapshot(
        &state,
        &runtime,
        snapshot,
        request_id,
        due_at,
        undo_seconds,
        now_timestamp(),
    )?;
    let _ = app.emit(CHANGED, ());
    Ok(result)
}

fn queue_snapshot(
    state: &AppState,
    runtime: &SendControlRuntime,
    snapshot: SendSnapshot,
    request_id: String,
    due_at: Option<i64>,
    undo_seconds: Option<u32>,
    now: i64,
) -> Result<SendSummary> {
    let (kind, due_at) = match (due_at, undo_seconds) {
        (Some(due), None) => ("scheduled", due),
        (None, Some(seconds)) if (1..=60).contains(&seconds) => ("undo", now + i64::from(seconds)),
        _ => {
            return Err(PebbleError::Validation(
                "Choose a schedule or a 1–60 second Undo Send delay".into(),
            ))
        }
    };
    pebble_store::scheduled_sends::validate_future(due_at, now)?;
    if request_id.len() != 36
        || !request_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-')
    {
        return Err(PebbleError::Validation("Invalid send request ID".into()));
    }
    let id = request_id;
    let account_id = snapshot.account_id.clone();
    let payload = seal(state, &id, snapshot)?;
    let gate = runtime.gate.lock().unwrap();
    ensure_running(&gate)?;
    if let Some(existing) = state.store.get_scheduled_send(&id)? {
        // A lost IPC reply must never create another delivery, even if the user edited meanwhile.
        let candidate: SealedSend =
            serde_json::from_slice(&state.crypto.decrypt_for(PURPOSE, &id, &payload)?)
                .map_err(invalid_data)?;
        if unseal(state, &existing)? != candidate
            || (kind == "scheduled" && existing.due_at != due_at)
        {
            return Err(conflict());
        }
        return summary(state, &existing);
    }
    let job = ScheduledSend {
        id,
        account_id,
        kind: kind.into(),
        due_at,
        state: "pending".into(),
        payload,
        revision: 0,
    };
    state.store.insert_scheduled_send(&job, now)?;
    summary(state, &job)
}

#[tauri::command]
pub async fn list_controlled_sends(state: State<'_, AppState>) -> Result<Vec<SendSummary>> {
    state
        .store
        .list_scheduled_sends()?
        .iter()
        .map(|job| summary(&state, job))
        .collect()
}

#[tauri::command]
pub async fn restore_controlled_send(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    runtime: State<'_, SendControlRuntime>,
    id: String,
    revision: i64,
) -> Result<SendSnapshot> {
    let gate = runtime.gate.lock().unwrap();
    ensure_running(&gate)?;
    let job = state.store.get_scheduled_send(&id)?.ok_or_else(conflict)?;
    if job.revision != revision {
        return Err(conflict());
    }
    let sealed = unseal(&state, &job)?;
    if !state
        .store
        .restore_scheduled_send(&id, revision, now_timestamp())?
    {
        return Err(conflict());
    }
    let _ = app.emit(CHANGED, ());
    // The encrypted backup remains in state=draft if staging fails or the app exits.
    stage_snapshot(&state.attachments_dir, sealed)
}

#[tauri::command]
pub async fn reschedule_controlled_send(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    runtime: State<'_, SendControlRuntime>,
    id: String,
    revision: i64,
    due_at: i64,
) -> Result<()> {
    let gate = runtime.gate.lock().unwrap();
    ensure_running(&gate)?;
    if !state
        .store
        .reschedule_send(&id, revision, due_at, now_timestamp())?
    {
        return Err(conflict());
    }
    let _ = app.emit(CHANGED, ());
    Ok(())
}

#[tauri::command]
pub async fn delete_controlled_send(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    revision: i64,
) -> Result<()> {
    if !state.store.delete_scheduled_send(&id, revision)? {
        return Err(conflict());
    }
    let _ = app.emit(CHANGED, ());
    Ok(())
}

async fn deliver(
    app: &tauri::AppHandle,
    state: &AppState,
    job: &ScheduledSend,
) -> Result<SendDispatchOutcome> {
    let snapshot = stage_snapshot(&state.attachments_dir, unseal(state, job)?)?;
    let paths: Vec<_> = snapshot
        .attachments
        .iter()
        .map(|a| std::path::PathBuf::from(&a.path))
        .collect();
    let result = dispatch_send_email(
        app,
        state,
        snapshot.account_id,
        snapshot.to,
        snapshot.cc,
        snapshot.bcc,
        snapshot.subject,
        snapshot.body_text,
        snapshot.body_html,
        snapshot.in_reply_to,
        paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    )
    .await;
    cleanup_paths(&state.attachments_dir, &paths);
    result
}

async fn process_job<F, Fut>(
    store: &Store,
    runtime: &SendControlRuntime,
    job: &ScheduledSend,
    now: i64,
    send: F,
) -> Result<bool>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<SendDispatchOutcome>>,
{
    let claimed = {
        let gate = runtime.gate.lock().unwrap();
        *gate == ExitGate::Running && store.claim_scheduled_send(&job.id, job.revision, now)?
    };
    if !claimed {
        return Ok(false);
    }
    let sent = matches!(send().await, Ok(SendDispatchOutcome::Accepted));
    store.finish_scheduled_send(&job.id, job.revision + 1, sent, now_timestamp())?;
    Ok(true)
}

/// Call exactly once, after AppState is managed and before showing the main window.
pub fn setup(app: &tauri::AppHandle) -> Result<()> {
    if !app.manage(SendControlRuntime::default()) {
        return Err(PebbleError::Internal(
            "Send controls already initialized".into(),
        ));
    }
    let state = app.state::<AppState>();
    state.store.recover_scheduled_sends(now_timestamp())?;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let state = app.state::<AppState>();
            let runtime = app.state::<SendControlRuntime>();
            let jobs = match state.store.due_scheduled_sends(now_timestamp()) {
                Ok(jobs) => jobs,
                Err(_) => continue,
            };
            for job in jobs
                .into_iter()
                .filter(|j| j.state == "pending" && j.due_at <= now_timestamp())
            {
                // Any failure (including preflight/storage errors) needs an explicit decision.
                let _ = process_job(&state.store, &runtime, &job, now_timestamp(), || {
                    deliver(&app, &state, &job)
                })
                .await;
                let _ = app.emit(CHANGED, ());
                let _ = app.emit(crate::events::MAIL_PENDING_OPS_CHANGED, ());
            }
        }
    });
    Ok(())
}

fn guard_exit(gate: &mut ExitGate, count: Result<usize>) -> bool {
    if *gate == ExitGate::Approved {
        return false;
    }
    if matches!(count, Ok(0)) {
        *gate = ExitGate::Approved;
        return false;
    }
    *gate = ExitGate::Prompting;
    true
}

/// Native exit guard for tray Quit, Cmd-Q and application exit. Returns true to prevent exit.
/// Also use on window close when background mode is disabled, BEFORE the window is destroyed.
pub fn prevent_exit(app: &tauri::AppHandle) -> bool {
    let Some(runtime) = app.try_state::<SendControlRuntime>() else {
        return true;
    };
    let state = app.state::<AppState>();
    let prevented = guard_exit(
        &mut runtime.gate.lock().unwrap(),
        state.store.scheduled_send_exit_count(),
    );
    if prevented {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        let _ = app.emit(EXIT_REQUESTED, ());
    }
    prevented
}

#[tauri::command]
pub async fn send_control_exit_pending(runtime: State<'_, SendControlRuntime>) -> Result<bool> {
    Ok(*runtime.gate.lock().unwrap() == ExitGate::Prompting)
}

#[tauri::command]
pub async fn resolve_send_control_exit(
    app: tauri::AppHandle,
    runtime: State<'_, SendControlRuntime>,
    confirm: bool,
) -> Result<()> {
    {
        let mut gate = runtime.gate.lock().unwrap();
        if *gate != ExitGate::Prompting {
            return Err(PebbleError::Validation("No exit request pending".into()));
        }
        *gate = if confirm {
            ExitGate::Approved
        } else {
            ExitGate::Running
        };
    }
    if confirm {
        app.exit(0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::new_id;
    fn fixture() -> (AppState, std::path::PathBuf) {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_account(&pebble_core::Account {
                id: "a".into(),
                email: "from@example.com".into(),
                display_name: "From".into(),
                account_label: None,
                provider_display_name: None,
                color: None,
                provider: pebble_core::ProviderType::Imap,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        let root = std::env::temp_dir().join(format!("pebble-send-control-{}", new_id()));
        std::fs::create_dir_all(&root).unwrap();
        let (tx, _) = std::sync::mpsc::channel();
        (
            AppState::new(
                store,
                pebble_search::TantivySearch::open_in_memory().unwrap(),
                pebble_crypto::CryptoService::from_key([5; 32]),
                tx,
                root.clone(),
            ),
            root,
        )
    }
    fn snapshot() -> SendSnapshot {
        SendSnapshot {
            account_id: "a".into(),
            to: vec!["to@example.com".into()],
            cc: vec![],
            bcc: vec!["hidden@example.com".into()],
            subject: "Private subject".into(),
            body_text: "Private body".into(),
            body_html: Some("<p>Private body</p>".into()),
            in_reply_to: Some("<parent@example.com>".into()),
            editor_mode: "markdown".into(),
            compose_mode: "reply-all".into(),
            raw_source: "**Private body**".into(),
            rich_text_html: "<p>Private body</p>".into(),
            quoted_reply_html: "<blockquote>quote</blockquote>".into(),
            attachments: vec![],
        }
    }
    fn job(state: &AppState) -> ScheduledSend {
        ScheduledSend {
            id: "job".into(),
            account_id: "a".into(),
            kind: "undo".into(),
            due_at: 200,
            state: "pending".into(),
            payload: seal(state, "job", snapshot()).unwrap(),
            revision: 0,
        }
    }
    #[test]
    fn every_exit_source_shares_fail_closed_gate_and_confirmation_pauses_claims() {
        let (state, root) = fixture();
        let mut gate = ExitGate::Running;
        assert!(!guard_exit(
            &mut gate,
            state.store.scheduled_send_exit_count()
        ));
        assert!(ensure_running(&gate).is_err());
        gate = ExitGate::Running;
        state
            .store
            .insert_scheduled_send(&job(&state), 100)
            .unwrap();
        for _source in ["tray", "Cmd-Q", "window", "app exit"] {
            assert!(guard_exit(
                &mut gate,
                state.store.scheduled_send_exit_count()
            ));
            assert_eq!(gate, ExitGate::Prompting);
            assert!(ensure_running(&gate).is_err());
        }
        gate = ExitGate::Running;
        assert!(ensure_running(&gate).is_ok());
        gate = ExitGate::Approved;
        assert!(!guard_exit(
            &mut gate,
            state.store.scheduled_send_exit_count()
        ));
        gate = ExitGate::Running;
        assert!(guard_exit(
            &mut gate,
            Err(PebbleError::Storage("test storage failure".into()))
        ));
        assert_eq!(gate, ExitGate::Prompting);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn encrypted_snapshot_survives_source_deletion_and_rejects_record_swapping() {
        let (state, root) = fixture();
        let source = root.join("report.txt");
        std::fs::write(&source, b"private attachment").unwrap();
        let mut input = snapshot();
        input.attachments.push(SnapshotAttachment {
            name: "../report.txt".into(),
            path: source.to_string_lossy().into_owned(),
            size: 18,
        });
        let mut job = job(&state);
        job.payload = seal(&state, &job.id, input).unwrap();
        assert!(!String::from_utf8_lossy(&job.payload).contains("Private"));
        std::fs::remove_file(&source).unwrap();
        let restored = stage_snapshot(&root, unseal(&state, &job).unwrap()).unwrap();
        assert_eq!(restored.editor_mode, "markdown");
        assert_eq!(restored.compose_mode, "reply-all");
        assert_eq!(restored.raw_source, "**Private body**");
        assert_eq!(restored.bcc, vec!["hidden@example.com"]);
        let path = Path::new(&restored.attachments[0].path);
        assert!(path.starts_with(root.canonicalize().unwrap().join("compose_staging")));
        assert_eq!(std::fs::read(path).unwrap(), b"private attachment");
        job.id = "other".into();
        assert!(unseal(&state, &job).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_recipients_mode_and_missing_attachments_fail_before_queueing() {
        let (state, root) = fixture();
        let mut input = snapshot();
        input.to = vec!["bad\r\nBcc: injected@example.com".into()];
        assert!(seal(&state, "job", input).is_err());
        let mut input = snapshot();
        input.editor_mode = "other".into();
        assert!(seal(&state, "job", input).is_err());
        let mut input = snapshot();
        input.attachments.push(SnapshotAttachment {
            name: "missing".into(),
            path: root.join("missing").to_string_lossy().into_owned(),
            size: 0,
        });
        assert!(seal(&state, "job", input).is_err());
        assert!(state.store.list_scheduled_sends().unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn encrypted_full_snapshot_and_attachment_survive_database_close_and_restart() {
        let (state, root) = fixture();
        let path = root.join("persistent.sqlite");
        let source = root.join("source.txt");
        std::fs::write(&source, b"persistent attachment").unwrap();
        let mut expected = snapshot();
        expected.attachments.push(SnapshotAttachment {
            name: "source.txt".into(),
            path: source.to_string_lossy().into(),
            size: 21,
        });
        let mut queued = job(&state);
        queued.payload = seal(&state, &queued.id, expected.clone()).unwrap();
        {
            let store = Store::open(&path).unwrap();
            store
                .insert_account(&state.store.get_account("a").unwrap().unwrap())
                .unwrap();
            store.insert_scheduled_send(&queued, 100).unwrap();
        }
        std::fs::remove_file(source).unwrap();
        drop(state);
        let (tx, _) = std::sync::mpsc::channel();
        let restarted = AppState::new(
            Store::open(&path).unwrap(),
            pebble_search::TantivySearch::open_in_memory().unwrap(),
            pebble_crypto::CryptoService::from_key([5; 32]),
            tx,
            root.clone(),
        );
        assert_eq!(restarted.store.recover_scheduled_sends(300).unwrap(), 1);
        let saved = restarted
            .store
            .get_scheduled_send(&queued.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.state, "review");
        assert_eq!(saved.due_at, 200);
        let mut restored = stage_snapshot(
            &restarted.attachments_dir,
            unseal(&restarted, &saved).unwrap(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&restored.attachments[0].path).unwrap(),
            b"persistent attachment"
        );
        restored.attachments.clear();
        expected.attachments.clear();
        assert!(restored == expected);
        drop(restarted);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn idempotent_queue_receipt_never_duplicates_or_overwrites_an_edited_snapshot() {
        let (state, root) = fixture();
        let runtime = SendControlRuntime::default();
        let id = new_id();
        let first = queue_snapshot(
            &state,
            &runtime,
            snapshot(),
            id.clone(),
            None,
            Some(10),
            100,
        )
        .unwrap();
        let replay = queue_snapshot(
            &state,
            &runtime,
            snapshot(),
            id.clone(),
            None,
            Some(10),
            102,
        )
        .unwrap();
        assert_eq!(first.id, replay.id);
        assert_eq!(replay.due_at, 110);
        let mut edited = snapshot();
        edited.subject = "Changed after lost reply".into();
        assert!(queue_snapshot(&state, &runtime, edited, id, None, Some(10), 103).is_err());
        assert_eq!(state.store.list_scheduled_sends().unwrap().len(), 1);
        assert!(
            queue_snapshot(&state, &runtime, snapshot(), new_id(), Some(100), None, 100).is_err()
        );
        assert!(
            queue_snapshot(&state, &runtime, snapshot(), new_id(), None, Some(0), 100).is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn only_one_worker_can_dispatch_and_no_write_transaction_spans_transport() {
        let (state, root) = fixture();
        let runtime = SendControlRuntime::default();
        let job = job(&state);
        state.store.insert_scheduled_send(&job, 100).unwrap();
        let (first, second) = tokio::join!(
            process_job(&state.store, &runtime, &job, 200, || async {
                assert!(!state.store.restore_scheduled_send(&job.id, 1, 200).unwrap());
                // A write succeeds during delivery, so no write connection/transaction is held.
                state
                    .store
                    .set_secure_user_data("concurrent-test", b"write")
                    .unwrap();
                tokio::task::yield_now().await;
                Ok(SendDispatchOutcome::Accepted)
            }),
            process_job(&state.store, &runtime, &job, 200, || async {
                panic!("second worker invoked provider")
            })
        );
        assert!(first.unwrap());
        assert!(!second.unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn cancelled_paused_and_future_jobs_never_invoke_transport() {
        let (state, root) = fixture();
        let runtime = SendControlRuntime::default();
        let job = job(&state);
        state.store.insert_scheduled_send(&job, 100).unwrap();
        assert!(!process_job(&state.store, &runtime, &job, 199, || async {
            panic!("provider called early")
        })
        .await
        .unwrap());
        *runtime.gate.lock().unwrap() = ExitGate::Prompting;
        assert!(!process_job(&state.store, &runtime, &job, 200, || async {
            panic!("provider called during exit")
        })
        .await
        .unwrap());
        *runtime.gate.lock().unwrap() = ExitGate::Running;
        assert!(state.store.restore_scheduled_send(&job.id, 0, 199).unwrap());
        assert!(!process_job(&state.store, &runtime, &job, 200, || async {
            panic!("provider called after Undo")
        })
        .await
        .unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn accepted_ambiguous_and_definite_failures_are_never_automatically_retried() {
        for outcome in [
            Ok(SendDispatchOutcome::Accepted),
            Ok(SendDispatchOutcome::Review),
            Err(PebbleError::Network("test ambiguity".into())),
            Err(PebbleError::Auth("test rejection".into())),
        ] {
            let (state, root) = fixture();
            let runtime = SendControlRuntime::default();
            let job = job(&state);
            let accepted = matches!(outcome, Ok(SendDispatchOutcome::Accepted));
            state.store.insert_scheduled_send(&job, 100).unwrap();
            assert!(
                process_job(&state.store, &runtime, &job, 200, || async { outcome })
                    .await
                    .unwrap()
            );
            assert_eq!(
                state
                    .store
                    .get_scheduled_send(&job.id)
                    .unwrap()
                    .unwrap()
                    .state,
                if accepted { "sent" } else { "review" }
            );
            assert!(!process_job(&state.store, &runtime, &job, 300, || async {
                panic!("duplicate provider call")
            })
            .await
            .unwrap());
            state.store.recover_scheduled_sends(400).unwrap();
            assert!(!process_job(&state.store, &runtime, &job, 400, || async {
                panic!("duplicate after restart")
            })
            .await
            .unwrap());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
