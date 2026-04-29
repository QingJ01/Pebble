use crate::state::AppState;
use pebble_core::{new_id, now_timestamp, PebbleError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

const EMAIL_TEMPLATES_KEY: &str = "email_templates";
const EMAIL_SIGNATURES_KEY: &str = "email_signatures";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmailTemplate {
    pub id: String,
    pub name: String,
    pub subject: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveEmailTemplateRequest {
    pub name: String,
    pub subject: String,
    pub body: String,
}

fn decrypt_json<T: DeserializeOwned>(
    state: &AppState,
    key: &str,
) -> Result<Option<T>, PebbleError> {
    let Some(encrypted) = state.store.get_secure_user_data(key)? else {
        return Ok(None);
    };
    let decrypted = state.crypto.decrypt(&encrypted)?;
    serde_json::from_slice(&decrypted)
        .map(Some)
        .map_err(|e| PebbleError::Internal(format!("Invalid secure user data for {key}: {e}")))
}

fn encrypt_json<T: Serialize>(state: &AppState, key: &str, value: &T) -> Result<(), PebbleError> {
    let plaintext = serde_json::to_vec(value)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize secure user data: {e}")))?;
    let encrypted = state.crypto.encrypt(&plaintext)?;
    state.store.set_secure_user_data(key, &encrypted)
}

fn normalize_template_input(input: SaveEmailTemplateRequest) -> SaveEmailTemplateRequest {
    SaveEmailTemplateRequest {
        name: input.name.trim().to_string(),
        subject: input.subject,
        body: input.body,
    }
}

#[tauri::command]
pub async fn list_email_templates(
    state: State<'_, AppState>,
) -> Result<Vec<EmailTemplate>, PebbleError> {
    Ok(decrypt_json(&state, EMAIL_TEMPLATES_KEY)?.unwrap_or_default())
}

#[tauri::command]
pub async fn save_email_template(
    state: State<'_, AppState>,
    template: SaveEmailTemplateRequest,
) -> Result<EmailTemplate, PebbleError> {
    let template = normalize_template_input(template);
    if template.name.is_empty() {
        return Err(PebbleError::Validation(
            "Template name cannot be empty".to_string(),
        ));
    }

    let mut templates: Vec<EmailTemplate> =
        decrypt_json(&state, EMAIL_TEMPLATES_KEY)?.unwrap_or_default();
    let saved = EmailTemplate {
        id: new_id(),
        name: template.name,
        subject: template.subject,
        body: template.body,
        created_at: now_timestamp(),
    };
    templates.push(saved.clone());
    encrypt_json(&state, EMAIL_TEMPLATES_KEY, &templates)?;
    Ok(saved)
}

#[tauri::command]
pub async fn delete_email_template(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), PebbleError> {
    let mut templates: Vec<EmailTemplate> =
        decrypt_json(&state, EMAIL_TEMPLATES_KEY)?.unwrap_or_default();
    templates.retain(|template| template.id != id);
    encrypt_json(&state, EMAIL_TEMPLATES_KEY, &templates)
}

#[tauri::command]
pub async fn get_email_signature(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<String, PebbleError> {
    let signatures: HashMap<String, String> =
        decrypt_json(&state, EMAIL_SIGNATURES_KEY)?.unwrap_or_default();
    Ok(signatures.get(&account_id).cloned().unwrap_or_default())
}

#[tauri::command]
pub async fn set_email_signature(
    state: State<'_, AppState>,
    account_id: String,
    signature: String,
) -> Result<(), PebbleError> {
    let mut signatures: HashMap<String, String> =
        decrypt_json(&state, EMAIL_SIGNATURES_KEY)?.unwrap_or_default();
    if signature.trim().is_empty() {
        signatures.remove(&account_id);
    } else {
        signatures.insert(account_id, signature);
    }

    if signatures.is_empty() {
        state.store.delete_secure_user_data(EMAIL_SIGNATURES_KEY)
    } else {
        encrypt_json(&state, EMAIL_SIGNATURES_KEY, &signatures)
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_template_input, SaveEmailTemplateRequest};

    #[test]
    fn template_names_are_trimmed_before_storage() {
        let normalized = normalize_template_input(SaveEmailTemplateRequest {
            name: "  Intro  ".to_string(),
            subject: "Subject".to_string(),
            body: "Body".to_string(),
        });

        assert_eq!(normalized.name, "Intro");
        assert_eq!(normalized.subject, "Subject");
        assert_eq!(normalized.body, "Body");
    }
}
