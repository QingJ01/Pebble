//! Chat with the assistant (#107, M3).
//!
//! `agent_chat_send` stores the question, returns a run id at once and
//! answers in the background, emitting `agent:chat-progress` while it
//! works and `agent:chat-changed` when the conversation changes. Messages
//! are encrypted at rest.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::agent::capabilities::{Capabilities, DraftProposal, FollowUpStarted, MailRef};
use crate::agent::chat::{run_chat, system_prompt, ChatError, Progress};
use crate::agent::llm::{ChatMessage, OpenAiCompatible};
use crate::commands::agent_model::{chat_completions_url, load_config};
use crate::commands::network::get_global_proxy_raw;
use crate::events;
use crate::state::AppState;
use pebble_core::{new_id, now_timestamp, PebbleError};
use pebble_store::agent_chat::AgentChatRow;
use pebble_translate::TranslateService;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::warn;

type CmdResult<T> = std::result::Result<T, PebbleError>;

const PURPOSE: &str = "agent_chat.payload";
/// Earlier messages given to the model as context.
const HISTORY_MESSAGES: u32 = 12;
pub const CHAT_PROGRESS_EVENT: &str = "agent:chat-progress";
pub const CHAT_CHANGED_EVENT: &str = "agent:chat-changed";

/// What a chat message holds (stored encrypted).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatPayload {
    pub text: String,
    /// Accounts the question was asked about.
    pub scope: Vec<String>,
    pub used: Vec<MailRef>,
    pub drafts: Vec<DraftProposal>,
    pub follow_ups: Vec<FollowUpStarted>,
    pub error: Option<String>,
    pub fallback: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEntry {
    pub id: String,
    pub role: String,
    pub run_id: Option<String>,
    pub status: String,
    pub created_at: i64,
    #[serde(flatten)]
    pub payload: ChatPayload,
}

fn runs() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static RUNS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn encrypt(state: &AppState, id: &str, payload: &ChatPayload) -> CmdResult<Vec<u8>> {
    let json = serde_json::to_vec(payload).map_err(|e| PebbleError::Internal(e.to_string()))?;
    state.crypto.encrypt_for(PURPOSE, id, &json)
}

fn decrypt(state: &AppState, row: &AgentChatRow) -> Option<ChatPayload> {
    let bytes = state
        .crypto
        .decrypt_for(PURPOSE, &row.id, &row.payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save(
    state: &AppState,
    role: &str,
    run_id: Option<&str>,
    status: &str,
    payload: &ChatPayload,
) -> CmdResult<String> {
    let id = new_id();
    state.store.insert_agent_chat(&AgentChatRow {
        id: id.clone(),
        role: role.into(),
        run_id: run_id.map(str::to_string),
        status: status.into(),
        payload: encrypt(state, &id, payload)?,
        created_at: now_timestamp(),
    })?;
    Ok(id)
}

fn today() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d (%A), UTC%:z")
        .to_string()
}

#[tauri::command]
pub async fn agent_chat_list(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> CmdResult<Vec<ChatEntry>> {
    let rows = state.store.list_agent_chat(limit.unwrap_or(200))?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let payload = decrypt(&state, &row)?;
            Some(ChatEntry {
                id: row.id,
                role: row.role,
                run_id: row.run_id,
                status: row.status,
                created_at: row.created_at,
                payload,
            })
        })
        .collect())
}

/// Runs still answering (the UI shows them as in progress after a reload).
#[tauri::command]
pub async fn agent_chat_running() -> CmdResult<Vec<String>> {
    Ok(runs()
        .lock()
        .map(|runs| runs.keys().cloned().collect())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn agent_chat_cancel(run_id: String) -> CmdResult<bool> {
    let flag = runs()
        .lock()
        .ok()
        .and_then(|runs| runs.get(&run_id).cloned());
    Ok(match flag {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    })
}

#[tauri::command]
pub async fn agent_chat_clear(state: State<'_, AppState>, app: AppHandle) -> CmdResult<()> {
    state.store.clear_agent_chat()?;
    let _ = app.emit(CHAT_CHANGED_EVENT, ());
    Ok(())
}

/// Ask the assistant. `account_ids` limits what it may read (all accounts
/// when absent). Returns the run id; the answer arrives as a new message.
#[tauri::command]
pub async fn agent_chat_send(
    state: State<'_, AppState>,
    app: AppHandle,
    text: String,
    account_ids: Option<Vec<String>>,
    from_account_id: Option<String>,
) -> CmdResult<String> {
    let question = text.trim().to_string();
    if question.is_empty() {
        return Err(PebbleError::Validation("Type a question".into()));
    }
    let config = load_config(&state)?
        .filter(|config| !config.endpoint.is_empty() && !config.model.is_empty())
        .ok_or_else(|| {
            PebbleError::Validation("Set up an AI model in Settings → AI assistant first".into())
        })?;
    let url = chat_completions_url(&config.endpoint)?;
    let accounts = state.store.list_accounts()?;
    let scope: HashMap<String, String> = accounts
        .into_iter()
        .filter(|account| {
            account_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&account.id))
        })
        .map(|account| (account.id, account.email))
        .collect();
    if scope.is_empty() {
        return Err(PebbleError::Validation(
            "Choose at least one account".into(),
        ));
    }

    // Context: the conversation so far (before this question), only
    // questions that got an answer, so the model never sees two questions
    // in a row.
    let recent = state.store.list_agent_chat(HISTORY_MESSAGES)?;
    let answered: std::collections::HashSet<&str> = recent
        .iter()
        .filter(|row| row.role == "assistant" && row.status == "done")
        .filter_map(|row| row.run_id.as_deref())
        .collect();
    let history: Vec<ChatMessage> = recent
        .iter()
        .filter(|row| row.status == "done")
        .filter(|row| {
            row.role == "assistant"
                || row
                    .run_id
                    .as_deref()
                    .is_some_and(|id| answered.contains(id))
        })
        .filter_map(|row| {
            let payload = decrypt(&state, row)?;
            Some(if row.role == "user" {
                ChatMessage::user(payload.text)
            } else {
                ChatMessage::assistant(payload.text)
            })
        })
        .collect();

    // Everything that can fail happens before the run exists, so a failed
    // send never leaves a run without an answer.
    let proxy = get_global_proxy_raw(&state.crypto, &state.store)?;
    let client = TranslateService::http_client_with_proxy(proxy.as_ref())?;
    let llm = OpenAiCompatible {
        client,
        url,
        model: config.model,
        api_key: config.api_key,
    };

    let run_id = new_id();
    save(
        &state,
        "user",
        Some(&run_id),
        "done",
        &ChatPayload {
            text: question.clone(),
            scope: scope.keys().cloned().collect(),
            ..Default::default()
        },
    )?;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut runs) = runs().lock() {
        runs.insert(run_id.clone(), Arc::clone(&cancel));
    }
    let _ = app.emit(CHAT_CHANGED_EVENT, ());

    let task_run_id = run_id.clone();
    let task_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let app = task_app;
        let run_id = task_run_id;
        // Ends the run however the task ends (a panic included): stores an
        // error answer if none was stored, and forgets the run.
        let mut guard = RunGuard {
            app: app.clone(),
            run_id: run_id.clone(),
            answered: false,
        };
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let store = Arc::clone(&state.store);
        let search = Arc::clone(&state.search);
        let mut caps = Capabilities::new(&store, &search, scope);
        caps.preferred_account = from_account_id;
        let system = system_prompt(&today(), &caps.scope_emails());
        let progress_app = app.clone();
        let progress_run = run_id.clone();
        let progress = move |p: Progress| {
            let _ = progress_app.emit(
                CHAT_PROGRESS_EVENT,
                serde_json::json!({ "runId": progress_run, "step": p.step, "detail": p.detail }),
            );
        };
        let result = run_chat(
            &llm, &mut caps, system, history, question, &progress, &cancel,
        )
        .await;
        let (status, payload) = match result {
            Ok(outcome) => (
                "done",
                ChatPayload {
                    text: outcome.text,
                    used: outcome.used,
                    drafts: outcome.drafts,
                    follow_ups: outcome.follow_ups,
                    fallback: outcome.fallback,
                    ..Default::default()
                },
            ),
            // What was already done (follow-ups exist, drafts were made)
            // stays visible even when the run stops early.
            Err(ChatError::Cancelled) => (
                "cancelled",
                ChatPayload {
                    used: caps.used.clone(),
                    drafts: caps.drafts.clone(),
                    follow_ups: caps.follow_ups.clone(),
                    ..Default::default()
                },
            ),
            Err(ChatError::Model(message)) => {
                warn!("Assistant run failed: {message}");
                (
                    "error",
                    ChatPayload {
                        error: Some(message),
                        used: caps.used.clone(),
                        drafts: caps.drafts.clone(),
                        follow_ups: caps.follow_ups.clone(),
                        ..Default::default()
                    },
                )
            }
        };
        let started_follow_ups = !caps.follow_ups.is_empty();
        if let Err(e) = save(&state, "assistant", Some(&run_id), status, &payload) {
            warn!("Failed to store the assistant's answer: {e}");
        }
        guard.answered = true;
        drop(guard);
        if started_follow_ups {
            let _ = app.emit(events::AGENT_EVENTS_CHANGED, ());
        }
    });
    Ok(run_id)
}

/// Ends a chat run when its task finishes, whatever the reason.
struct RunGuard {
    app: AppHandle,
    run_id: String,
    answered: bool,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if !self.answered {
            if let Some(state) = self.app.try_state::<AppState>() {
                let payload = ChatPayload {
                    error: Some("The assistant stopped unexpectedly".into()),
                    ..Default::default()
                };
                if let Err(e) = save(&state, "assistant", Some(&self.run_id), "error", &payload) {
                    warn!("Failed to store the assistant's error: {e}");
                }
            }
        }
        if let Ok(mut runs) = runs().lock() {
            runs.remove(&self.run_id);
        }
        let _ = self.app.emit(CHAT_CHANGED_EVENT, ());
    }
}
