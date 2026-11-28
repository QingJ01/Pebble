//! Model-assisted notifications (#107 §2.7).
//!
//! The sync stage records the rule decision for every new message at once
//! (so dedupe and the digest work as usual) and holds back the
//! notification of messages the rules couldn't place when the account has
//! model assist on. This module then asks the configured model about those
//! messages, in one request per batch, and settles each one:
//!
//! - the model may only choose between interrupting and the digest, never
//!   silence (see [`pebble_notify::apply_model_verdict`]);
//! - no model, a slow or failing one, a malformed answer, too many messages
//!   at once or the daily limit: the rule decision stands, logged as
//!   `model_fallback`;
//! - the model's "why" line is kept with the decision and shown as AI text.

use std::time::Duration;

use pebble_core::{now_timestamp, Message};
use pebble_notify::triage_model::{
    parse_response, system_prompt, user_prompt, Correction, TriageAnswer, TriageItem, MAX_BATCH,
    MIN_CONFIDENCE,
};
use pebble_notify::{apply_model_verdict, Decision, Reason, Tier};
use pebble_store::Store;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

use crate::agent::capabilities::redact_codes;
use crate::agent::llm::{ChatMessage, Llm, OpenAiCompatible};
use crate::commands::agent_model::{chat_completions_url, load_config};
use crate::commands::indexing::{send_planned_notifications, NotifyItem};
use crate::commands::network::get_global_proxy_raw;
use crate::events;
use crate::smart_notify::{local_minute_of_day, tier_code, NotifyPrefsRecord, Triaged};
use crate::state::AppState;
use pebble_translate::TranslateService;

/// How long a batch may wait for the model.
const MODEL_TIMEOUT: Duration = Duration::from_secs(8);
/// Model decisions per day; beyond it the rules decide until midnight.
const DAILY_LIMIT: u32 = 300;
/// Characters of body sent per message.
const BODY_CHARS: usize = 600;
/// Corrections included as examples.
const EXAMPLES: u32 = 8;

/// A message waiting for the model.
#[derive(Debug, Clone)]
pub struct ModelCandidate {
    /// The rule decision, already logged.
    pub triaged: Triaged,
    pub prefs: NotifyPrefsRecord,
    pub message: Message,
    pub to_me: bool,
    pub known_contact: bool,
    pub times_user_wrote: u32,
    pub automated: bool,
    /// What the notification shows if it interrupts.
    pub notify: NotifyItem,
    /// The rules already found a deadline in it.
    pub has_deadline: bool,
}

fn body_for_model(message: &Message) -> String {
    let text = if message.body_text.trim().is_empty() {
        pebble_core::strip_html_for_snippet(&message.body_html_raw)
    } else {
        message.body_text.clone()
    };
    let stripped = pebble_notify::awaiting::strip_quoted(&text);
    let mut out: String = stripped.trim().chars().take(BODY_CHARS).collect();
    if stripped.trim().chars().count() > BODY_CHARS {
        out.push('…');
    }
    redact_codes(&out)
}

pub fn triage_items(candidates: &[ModelCandidate]) -> Vec<TriageItem> {
    candidates
        .iter()
        .enumerate()
        .map(|(i, c)| TriageItem {
            id: (i + 1).to_string(),
            from: c.message.from_address.clone(),
            from_name: c.message.from_name.clone(),
            subject: c.message.subject.clone(),
            to_me: c.to_me,
            known_contact: c.known_contact,
            times_user_wrote: c.times_user_wrote,
            automated: c.automated,
            body: c.prefs.model_send_body.then(|| body_for_model(&c.message)),
        })
        .collect()
}

/// The decision for one candidate given the model's answer (if any).
pub fn settle(candidate: &ModelCandidate, answer: Option<&TriageAnswer>, minute: u16) -> Decision {
    match answer {
        Some(answer) => apply_model_verdict(
            &candidate.triaged.decision,
            &answer.verdict,
            &candidate.prefs.to_prefs(),
            minute,
            MIN_CONFIDENCE,
        ),
        None => Decision {
            reason: Reason::ModelFallback,
            model_candidate: false,
            ..candidate.triaged.decision.clone()
        },
    }
}

fn local_midnight_ts() -> i64 {
    let midnight = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default();
    crate::deadlines::local_ts(midnight).unwrap_or_else(|| now_timestamp() - 86_400)
}

fn corrections(store: &Store) -> Vec<Correction> {
    store
        .recent_notify_feedback(EXAMPLES)
        .unwrap_or_default()
        .into_iter()
        .map(|(sender, subject, important)| Correction {
            sender,
            subject: subject.chars().take(80).collect(),
            important,
        })
        .collect()
}

/// Ask the model. `Err` means "use the rules" (with the reason logged).
async fn ask_model(
    state: &AppState,
    candidates: &[ModelCandidate],
) -> Result<std::collections::HashMap<String, TriageAnswer>, String> {
    if candidates.len() > MAX_BATCH {
        return Err(format!("{} messages at once", candidates.len()));
    }
    let used = state
        .store
        .count_model_decisions_since(local_midnight_ts())
        .unwrap_or(0);
    if used + candidates.len() as u32 > DAILY_LIMIT {
        return Err("daily limit reached".into());
    }
    let config = load_config(state)
        .map_err(|e| e.to_string())?
        .filter(|c| !c.endpoint.is_empty() && !c.model.is_empty())
        .ok_or("no model set up")?;
    let url = chat_completions_url(&config.endpoint).map_err(|e| e.to_string())?;
    let proxy = get_global_proxy_raw(&state.crypto, &state.store).map_err(|e| e.to_string())?;
    let client =
        TranslateService::http_client_with_proxy(proxy.as_ref()).map_err(|e| e.to_string())?;
    let llm = OpenAiCompatible {
        client,
        url,
        model: config.model,
        api_key: config.api_key,
    };
    let items = triage_items(candidates);
    let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
    let now_local = chrono::Local::now()
        .format("%Y-%m-%d %H:%M (%A)")
        .to_string();
    let messages = vec![
        ChatMessage::system(system_prompt(&now_local)),
        ChatMessage::user(user_prompt(&items, &corrections(&state.store))),
    ];
    let reply = tokio::time::timeout(MODEL_TIMEOUT, llm.complete(&messages, None))
        .await
        .map_err(|_| "the model didn't answer in time".to_string())?
        .map_err(|e| e.to_string())?;
    parse_response(reply.content.as_deref().unwrap_or_default(), &ids)
}

/// Settle the held messages of one flush and show the ones that interrupt.
pub async fn refine(app: AppHandle, candidates: Vec<ModelCandidate>) {
    if candidates.is_empty() {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let answers = match ask_model(&state, &candidates).await {
        Ok(answers) => answers,
        Err(reason) => {
            info!(
                "Model-assisted notifications fell back to the rules: {reason} ({} messages)",
                candidates.len()
            );
            Default::default()
        }
    };
    let store = std::sync::Arc::clone(&state.store);
    let app_for_task = app.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let minute = local_minute_of_day();
        let now = now_timestamp();
        let mut notify = Vec::new();
        let mut deadlines = false;
        for (i, candidate) in candidates.iter().enumerate() {
            let answer = answers.get(&(i + 1).to_string());
            let decision = settle(candidate, answer, minute);
            let why = answer.and_then(|a| a.verdict.why.clone());
            if let Err(e) = store.update_notification_decision(
                &candidate.triaged.account_id,
                &candidate.triaged.message_key,
                tier_code(decision.tier),
                &decision.reason.code(),
                why.as_deref(),
                answer.map(|a| a.verdict.confidence),
            ) {
                warn!("Failed to store model decision: {e}");
            }
            if decision.tier == Tier::Interrupt {
                let mut item = candidate.notify.clone();
                if let Some(why) = &why {
                    item.body = format!("{}\n✦ {why}", item.body);
                }
                notify.push(item);
            }
            if let (false, Some(due)) = (candidate.has_deadline, answer.and_then(|a| a.deadline)) {
                let received = crate::deadlines::local_naive(candidate.message.date);
                let sane =
                    due.date() >= received.date() && due - received <= chrono::Duration::days(120);
                if sane {
                    deadlines |= crate::deadlines::record(
                        &store,
                        &candidate.message,
                        due,
                        false,
                        None,
                        "model",
                        now,
                    );
                }
            }
        }
        let _ = app_for_task.emit(events::NOTIFY_CHANGED, ());
        if deadlines {
            let _ = app_for_task.emit(events::AGENT_EVENTS_CHANGED, ());
        }
        send_planned_notifications(&app_for_task, notify);
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_notify::{Mode, ModelLabel, ModelVerdict};

    fn candidate(tier: Tier, send_body: bool) -> ModelCandidate {
        let message = Message {
            id: "m1".into(),
            account_id: "a1".into(),
            remote_id: "r1".into(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Q".into(),
            snippet: String::new(),
            from_address: "x@y.example".into(),
            from_name: "X".into(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "Your code is 123456.\n\n> old quoted text".into(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 0,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 0,
            updated_at: 0,
        };
        let prefs = NotifyPrefsRecord {
            mode: Mode::Smart,
            model_assist: true,
            model_send_body: send_body,
            ..Default::default()
        };
        ModelCandidate {
            triaged: Triaged {
                account_id: "a1".into(),
                message_id: "m1".into(),
                message_key: "k1".into(),
                decision: Decision {
                    tier,
                    reason: Reason::Direct,
                    base_tier: tier,
                    model_candidate: true,
                },
            },
            prefs,
            message,
            to_me: true,
            known_contact: false,
            times_user_wrote: 2,
            automated: false,
            notify: NotifyItem {
                account_id: "a1".into(),
                message_id: "m1".into(),
                message_key: "k1".into(),
                body: "X: Q".into(),
                date: 0,
            },
            has_deadline: false,
        }
    }

    #[test]
    fn items_mask_codes_strip_quotes_and_respect_metadata_only() {
        let items = triage_items(&[
            candidate(Tier::Digest, true),
            candidate(Tier::Digest, false),
        ]);
        let body = items[0].body.as_deref().unwrap();
        assert!(!body.contains("123456") && !body.contains("old quoted"));
        assert_eq!(items[0].id, "1");
        assert!(items[1].body.is_none());
    }

    #[test]
    fn model_can_promote_or_demote_but_failure_keeps_the_rules() {
        let c = candidate(Tier::Interrupt, true);
        let fallback = settle(&c, None, 600);
        assert_eq!(fallback.tier, Tier::Interrupt);
        assert_eq!(fallback.reason.code(), "model_fallback");

        let answer = |interrupt, confidence| TriageAnswer {
            verdict: ModelVerdict {
                interrupt,
                label: ModelLabel::Newsletter,
                confidence,
                why: None,
            },
            deadline: None,
        };
        let demoted = settle(&c, Some(&answer(false, 0.9)), 600);
        assert_eq!(demoted.tier, Tier::Digest);
        assert_eq!(demoted.reason.code(), "model:newsletter");
        let unsure = settle(
            &candidate(Tier::Digest, true),
            Some(&answer(true, 0.3)),
            600,
        );
        assert_eq!(
            unsure.tier,
            Tier::Digest,
            "low confidence goes to the digest"
        );
        let promoted = settle(
            &candidate(Tier::Digest, true),
            Some(&answer(true, 0.9)),
            600,
        );
        assert_eq!(promoted.tier, Tier::Interrupt);
    }
}
