//! The AI model the assistant uses (#107): an OpenAI-compatible endpoint
//! (cloud or local, e.g. Ollama / LM Studio), a model name and an API key.
//!
//! Stored encrypted in `secure_user_data` (never in cloud backups). The
//! webview only learns whether a key is saved, never the key itself.

use std::time::{Duration, Instant};

use crate::commands::encrypted_store::{load_secure_user_data, store_secure_user_data};
use crate::commands::network::get_global_proxy_raw;
use crate::state::AppState;
use pebble_core::PebbleError;
use pebble_translate::TranslateService;
use serde::{Deserialize, Serialize};
use tauri::State;

type CmdResult<T> = std::result::Result<T, PebbleError>;

const STORE_KEY: &str = "agent_model";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentModelConfig {
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

/// What the webview may see.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelView {
    pub configured: bool,
    pub endpoint: String,
    pub model: String,
    pub has_api_key: bool,
    /// The endpoint is on this machine (mail never leaves the device).
    pub local: bool,
}

/// Normalise the endpoint and return the Chat Completions URL. Accepts a
/// base URL (`https://api.example.com`), one ending in `/v1`, or the full
/// `…/chat/completions` URL.
pub fn chat_completions_url(endpoint: &str) -> CmdResult<String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    let parsed = url::Url::parse(trimmed).map_err(|_| {
        PebbleError::Validation("Enter the endpoint as a URL, e.g. https://api.openai.com".into())
    })?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(PebbleError::Validation(
            "The endpoint must start with https:// or http://".into(),
        ));
    }
    if parsed.scheme() == "http" && !is_local_host(parsed.host_str().unwrap_or_default()) {
        return Err(PebbleError::Validation(
            "Use https:// for remote endpoints (http:// is only allowed for this machine)".into(),
        ));
    }
    Ok(if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    })
}

fn is_local_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".localhost")
}

fn is_local_endpoint(endpoint: &str) -> bool {
    url::Url::parse(endpoint.trim())
        .ok()
        .and_then(|url| url.host_str().map(is_local_host))
        .unwrap_or(false)
}

pub(crate) fn load_config(state: &AppState) -> CmdResult<Option<AgentModelConfig>> {
    let Some(bytes) = load_secure_user_data(&state.crypto, &state.store, STORE_KEY)? else {
        return Ok(None);
    };
    Ok(serde_json::from_slice::<AgentModelConfig>(&bytes).ok())
}

fn save_config(state: &AppState, config: &AgentModelConfig) -> CmdResult<()> {
    let bytes = serde_json::to_vec(config).map_err(|e| PebbleError::Internal(e.to_string()))?;
    store_secure_user_data(&state.crypto, &state.store, STORE_KEY, &bytes)
}

pub fn view_of(config: Option<&AgentModelConfig>) -> AgentModelView {
    match config {
        Some(config) => AgentModelView {
            configured: !config.endpoint.is_empty() && !config.model.is_empty(),
            endpoint: config.endpoint.clone(),
            model: config.model.clone(),
            has_api_key: !config.api_key.is_empty(),
            local: is_local_endpoint(&config.endpoint),
        },
        None => AgentModelView {
            configured: false,
            endpoint: String::new(),
            model: String::new(),
            has_api_key: false,
            local: false,
        },
    }
}

#[tauri::command]
pub async fn get_agent_model(state: State<'_, AppState>) -> CmdResult<AgentModelView> {
    Ok(view_of(load_config(&state)?.as_ref()))
}

/// Merge a submission with the saved config: an empty `api_key` keeps the
/// saved key unless `clear_api_key` is set.
fn merge(
    saved: Option<AgentModelConfig>,
    endpoint: String,
    model: String,
    api_key: Option<String>,
    clear_api_key: bool,
) -> AgentModelConfig {
    let saved_key = saved.map(|config| config.api_key).unwrap_or_default();
    let api_key = match api_key.map(|key| key.trim().to_string()) {
        Some(key) if !key.is_empty() => key,
        _ if clear_api_key => String::new(),
        _ => saved_key,
    };
    AgentModelConfig {
        endpoint: endpoint.trim().trim_end_matches('/').to_string(),
        model: model.trim().to_string(),
        api_key,
    }
}

#[tauri::command]
pub async fn set_agent_model(
    state: State<'_, AppState>,
    endpoint: String,
    model: String,
    api_key: Option<String>,
    clear_api_key: Option<bool>,
) -> CmdResult<AgentModelView> {
    let config = merge(
        load_config(&state)?,
        endpoint,
        model,
        api_key,
        clear_api_key.unwrap_or(false),
    );
    chat_completions_url(&config.endpoint)?;
    if config.model.is_empty() {
        return Err(PebbleError::Validation("Enter a model name".into()));
    }
    save_config(&state, &config)?;
    Ok(view_of(Some(&config)))
}

#[tauri::command]
pub async fn clear_agent_model(state: State<'_, AppState>) -> CmdResult<()> {
    state.store.delete_secure_user_data(STORE_KEY)
}

/// The translation settings' LLM provider, if it is one, as a starting point.
#[tauri::command]
pub async fn import_agent_model_from_translate(
    state: State<'_, AppState>,
) -> CmdResult<Option<AgentModelView>> {
    let Some(pebble_translate::types::TranslateProviderConfig::LLM {
        endpoint,
        api_key,
        model,
        ..
    }) = crate::commands::translate::load_stored_provider_config(&state)?
    else {
        return Ok(None);
    };
    let config = AgentModelConfig {
        endpoint,
        model,
        api_key,
    };
    save_config(&state, &config)?;
    Ok(Some(view_of(Some(&config))))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelTest {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
}

/// Send a one-line request to check the endpoint, model and key. Fields
/// left empty use the saved values.
#[tauri::command]
pub async fn test_agent_model(
    state: State<'_, AppState>,
    endpoint: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
) -> CmdResult<AgentModelTest> {
    let saved = load_config(&state)?.unwrap_or_default();
    let config = merge(
        Some(saved.clone()),
        endpoint
            .filter(|e| !e.trim().is_empty())
            .unwrap_or(saved.endpoint),
        model
            .filter(|m| !m.trim().is_empty())
            .unwrap_or(saved.model),
        api_key,
        false,
    );
    let url = chat_completions_url(&config.endpoint)?;
    let proxy = get_global_proxy_raw(&state.crypto, &state.store)?;
    let client = TranslateService::http_client_with_proxy(proxy.as_ref())?;
    let started = Instant::now();
    let mut request = client
        .post(&url)
        .timeout(Duration::from_secs(30))
        .json(&serde_json::json!({
            "model": config.model,
            "messages": [{ "role": "user", "content": "Reply with the single word: OK" }],
            "max_tokens": 5,
            "temperature": 0,
        }));
    if !config.api_key.is_empty() {
        request = request.bearer_auth(&config.api_key);
    }
    let result = request.send().await;
    let latency_ms = started.elapsed().as_millis() as u64;
    Ok(match result {
        Ok(response) if response.status().is_success() => AgentModelTest {
            ok: true,
            latency_ms,
            message: String::new(),
        },
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail: String = body.chars().take(200).collect();
            AgentModelTest {
                ok: false,
                latency_ms,
                message: format!("HTTP {status}: {detail}"),
            }
        }
        Err(error) => AgentModelTest {
            ok: false,
            latency_ms,
            message: if error.is_timeout() {
                "The model didn't answer within 30 seconds".into()
            } else {
                error.to_string()
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_urls_are_normalised() {
        assert_eq!(
            chat_completions_url("https://api.example.com").unwrap(),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://openrouter.ai/api/v1/chat/completions").unwrap(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("http://localhost:11434").unwrap(),
            "http://localhost:11434/v1/chat/completions"
        );
        assert!(
            chat_completions_url("http://api.example.com").is_err(),
            "plain http only for this machine"
        );
        assert!(chat_completions_url("ftp://x").is_err());
        assert!(chat_completions_url("not a url").is_err());
    }

    #[test]
    fn saved_key_is_kept_unless_replaced_or_cleared() {
        let saved = AgentModelConfig {
            endpoint: "https://a".into(),
            model: "m".into(),
            api_key: "sk-old".into(),
        };
        assert_eq!(
            merge(
                Some(saved.clone()),
                "https://b/".into(),
                " m2 ".into(),
                None,
                false
            )
            .api_key,
            "sk-old"
        );
        assert_eq!(
            merge(
                Some(saved.clone()),
                "https://b".into(),
                "m2".into(),
                Some("  ".into()),
                false
            )
            .api_key,
            "sk-old"
        );
        assert_eq!(
            merge(
                Some(saved.clone()),
                "https://b".into(),
                "m2".into(),
                Some("sk-new".into()),
                false
            )
            .api_key,
            "sk-new"
        );
        assert_eq!(
            merge(Some(saved), "https://b".into(), "m2".into(), None, true).api_key,
            ""
        );
        let merged = merge(None, "https://b/".into(), " m2 ".into(), None, false);
        assert_eq!(
            (merged.endpoint.as_str(), merged.model.as_str()),
            ("https://b", "m2")
        );
    }

    #[test]
    fn the_view_never_carries_the_key() {
        let view = view_of(Some(&AgentModelConfig {
            endpoint: "http://localhost:11434".into(),
            model: "qwen2.5".into(),
            api_key: "sk-secret".into(),
        }));
        assert!(view.configured && view.has_api_key && view.local);
        assert!(!serde_json::to_string(&view).unwrap().contains("sk-secret"));
        assert!(!view_of(None).configured);
    }
}
