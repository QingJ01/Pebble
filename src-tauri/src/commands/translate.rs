use super::network::get_global_proxy_raw;
use crate::commands::encrypted_store::{ACTIVE_TRANSLATE_CONFIG_ID, TRANSLATE_CONFIG_PURPOSE};
use crate::state::AppState;
use pebble_core::{now_timestamp, PebbleError, TranslateConfig};
use pebble_crypto::CryptoService;
use pebble_store::Store;
use pebble_translate::types::{TranslateProviderConfig, TranslateResult};
use pebble_translate::TranslateService;
use tauri::State;

/// Decode a hex string to bytes.
pub(crate) fn hex_decode(s: &str) -> std::result::Result<Vec<u8>, PebbleError> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(PebbleError::Internal(
            "Invalid hex string length".to_string(),
        ));
    }
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = hex_nibble(pair[0])
                .ok_or_else(|| PebbleError::Internal("Invalid hex digit".to_string()))?;
            let low = hex_nibble(pair[1])
                .ok_or_else(|| PebbleError::Internal("Invalid hex digit".to_string()))?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Encode bytes to a hex string.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decrypt the config field of a TranslateConfig using the app's crypto service.
/// If the stored value is legacy plaintext JSON, migrates it to encrypted form in-place.
pub(crate) fn decrypt_config(
    state: &AppState,
    stored: &str,
) -> std::result::Result<String, PebbleError> {
    decrypt_config_with_store(&state.crypto, &state.store, stored)
}

pub(crate) fn decrypt_config_with_store(
    crypto: &CryptoService,
    store: &Store,
    stored: &str,
) -> std::result::Result<String, PebbleError> {
    if serde_json::from_str::<serde_json::Value>(stored).is_ok() {
        // Legacy plaintext config written by old versions — migrate it to
        // encrypted form in-place, but only if it is a well-formed provider
        // config with a safe endpoint. Backup restores never write plaintext
        // configs any more, so this path only serves genuine legacy rows.
        let provider_config: TranslateProviderConfig = serde_json::from_str(stored)
            .map_err(|e| PebbleError::Translate(format!("Invalid config: {e}")))?;
        validate_provider_config(&provider_config)?;
        let encrypted = encrypt_config_with_crypto(crypto, stored)?;
        store.compare_exchange_translate_config_blob(stored, &encrypted)?;
        return Ok(stored.to_string());
    }
    let bytes = hex_decode(stored)?;
    let needs_migration = pebble_crypto::CryptoService::ciphertext_needs_migration(&bytes);
    let decrypted =
        crypto.decrypt_for(TRANSLATE_CONFIG_PURPOSE, ACTIVE_TRANSLATE_CONFIG_ID, &bytes)?;
    let plaintext = String::from_utf8(decrypted)
        .map_err(|e| PebbleError::Internal(format!("Invalid UTF-8 in decrypted config: {e}")))?;
    if needs_migration {
        let encrypted = encrypt_config_with_crypto(crypto, &plaintext)?;
        store.compare_exchange_translate_config_blob(stored, &encrypted)?;
    }
    Ok(plaintext)
}

/// Encrypt a plaintext config string for storage.
pub(crate) fn encrypt_config(
    state: &AppState,
    plaintext: &str,
) -> std::result::Result<String, PebbleError> {
    encrypt_config_with_crypto(&state.crypto, plaintext)
}

fn encrypt_config_with_crypto(
    crypto: &CryptoService,
    plaintext: &str,
) -> std::result::Result<String, PebbleError> {
    let encrypted = crypto.encrypt_for(
        TRANSLATE_CONFIG_PURPOSE,
        ACTIVE_TRANSLATE_CONFIG_ID,
        plaintext.as_bytes(),
    )?;
    Ok(hex_encode(&encrypted))
}

#[tauri::command]
pub async fn translate_text(
    state: State<'_, AppState>,
    text: String,
    from_lang: String,
    to_lang: String,
) -> std::result::Result<TranslateResult, PebbleError> {
    let config = state
        .store
        .get_translate_config()?
        .ok_or_else(|| PebbleError::Translate("No translate engine configured".to_string()))?;

    if !config.is_enabled {
        return Err(PebbleError::Translate(
            "Translation is disabled".to_string(),
        ));
    }

    // Decrypt config before parsing
    let decrypted = decrypt_config(&state, &config.config)?;
    let provider_config: TranslateProviderConfig = serde_json::from_str(&decrypted)
        .map_err(|e| PebbleError::Translate(format!("Invalid config: {e}")))?;

    validate_provider_config(&provider_config)?;

    let proxy = get_global_proxy_raw(&state.crypto, &state.store)?;

    TranslateService::translate_with_proxy(
        &provider_config,
        proxy.as_ref(),
        &text,
        &from_lang,
        &to_lang,
    )
    .await
}

/// Validate a translate config restored from passphrase-encrypted backup
/// secrets. Returns the provider type tag and the normalised config JSON.
pub(crate) fn validate_restored_translate_config(
    config: &str,
) -> std::result::Result<(String, String), PebbleError> {
    let provider_config: TranslateProviderConfig = serde_json::from_str(config)
        .map_err(|e| PebbleError::Validation(format!("Invalid restored translate config: {e}")))?;
    validate_provider_config(&provider_config)?;
    let normalized = serde_json::to_string(&provider_config)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize config: {e}")))?;
    Ok((provider_type_tag(&provider_config).to_string(), normalized))
}

fn provider_type_tag(config: &TranslateProviderConfig) -> &'static str {
    match config {
        TranslateProviderConfig::DeepLX { .. } => "deeplx",
        TranslateProviderConfig::DeepL { .. } => "deepl",
        TranslateProviderConfig::GenericApi { .. } => "generic_api",
        TranslateProviderConfig::LLM { .. } => "llm",
    }
}

fn provider_api_key(config: &TranslateProviderConfig) -> Option<&str> {
    match config {
        TranslateProviderConfig::DeepLX { .. } => None,
        TranslateProviderConfig::DeepL { api_key, .. } => Some(api_key.as_str()),
        TranslateProviderConfig::GenericApi { api_key, .. } => api_key.as_deref(),
        TranslateProviderConfig::LLM { api_key, .. } => Some(api_key.as_str()),
    }
    .filter(|key| !key.is_empty())
}

fn provider_endpoint(config: &TranslateProviderConfig) -> Option<&str> {
    match config {
        TranslateProviderConfig::DeepLX { endpoint }
        | TranslateProviderConfig::GenericApi { endpoint, .. }
        | TranslateProviderConfig::LLM { endpoint, .. } => Some(endpoint.as_str()),
        TranslateProviderConfig::DeepL { .. } => None,
    }
}

/// Build the config JSON returned to the webview: the API key is blanked and
/// replaced by a `has_api_key` flag.
fn mask_provider_config(config: &str) -> std::result::Result<String, PebbleError> {
    let provider_config: TranslateProviderConfig = serde_json::from_str(config)
        .map_err(|e| PebbleError::Translate(format!("Invalid config: {e}")))?;
    let has_api_key = provider_api_key(&provider_config).is_some();
    let mut value = serde_json::to_value(&provider_config)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize config: {e}")))?;
    if let Some(object) = value.as_object_mut() {
        if object.contains_key("api_key") {
            object.insert(
                "api_key".to_string(),
                serde_json::Value::String(String::new()),
            );
        }
        object.insert(
            "has_api_key".to_string(),
            serde_json::Value::Bool(has_api_key),
        );
    }
    serde_json::to_string(&value)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize config: {e}")))
}

/// A blank API key from the webview means "keep the stored key". The stored
/// key is only reused for the same provider and the same endpoint, so a
/// changed endpoint can never receive the previously saved secret.
fn merge_stored_api_key(
    mut submitted: TranslateProviderConfig,
    stored: Option<&TranslateProviderConfig>,
) -> TranslateProviderConfig {
    if provider_api_key(&submitted).is_some() {
        return submitted;
    }
    let Some(stored) = stored else {
        return submitted;
    };
    if provider_type_tag(stored) != provider_type_tag(&submitted)
        || provider_endpoint(stored) != provider_endpoint(&submitted)
    {
        return submitted;
    }
    let Some(stored_key) = provider_api_key(stored).map(str::to_string) else {
        return submitted;
    };
    match &mut submitted {
        TranslateProviderConfig::DeepL { api_key, .. }
        | TranslateProviderConfig::LLM { api_key, .. } => *api_key = stored_key,
        TranslateProviderConfig::GenericApi { api_key, .. } => *api_key = Some(stored_key),
        TranslateProviderConfig::DeepLX { .. } => {}
    }
    submitted
}

fn load_stored_provider_config(
    state: &AppState,
) -> std::result::Result<Option<TranslateProviderConfig>, PebbleError> {
    let Some(stored) = state.store.get_translate_config()? else {
        return Ok(None);
    };
    let decrypted = decrypt_config(state, &stored.config)?;
    Ok(serde_json::from_str(&decrypted).ok())
}

/// Parse a config submitted by the webview, fill in the stored API key when
/// it was left blank, and validate the endpoint.
fn resolve_submitted_config(
    state: &AppState,
    config: &str,
) -> std::result::Result<TranslateProviderConfig, PebbleError> {
    let submitted: TranslateProviderConfig = serde_json::from_str(config)
        .map_err(|e| PebbleError::Translate(format!("Invalid config: {e}")))?;
    let stored = if provider_api_key(&submitted).is_none() {
        load_stored_provider_config(state)?
    } else {
        None
    };
    let merged = merge_stored_api_key(submitted, stored.as_ref());
    validate_provider_config(&merged)?;
    Ok(merged)
}

#[tauri::command]
pub async fn get_translate_config(
    state: State<'_, AppState>,
) -> std::result::Result<Option<TranslateConfig>, PebbleError> {
    let config = state.store.get_translate_config()?;
    // Never hand the decrypted API key to the webview.
    match config {
        Some(mut tc) => {
            let decrypted = decrypt_config(&state, &tc.config)?;
            tc.config = mask_provider_config(&decrypted)?;
            Ok(Some(tc))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn save_translate_config(
    state: State<'_, AppState>,
    provider_type: String,
    config: String,
    is_enabled: bool,
) -> std::result::Result<(), PebbleError> {
    let provider_config = resolve_submitted_config(&state, &config)?;
    // The provider type is derived from the validated config itself rather
    // than trusted from the caller.
    let _ = provider_type;
    let normalized = serde_json::to_string(&provider_config)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize config: {e}")))?;

    let now = now_timestamp();
    // Encrypt config before storing
    let encrypted = encrypt_config(&state, &normalized)?;
    let tc = TranslateConfig {
        id: "active".to_string(),
        provider_type: provider_type_tag(&provider_config).to_string(),
        config: encrypted,
        is_enabled,
        created_at: now,
        updated_at: now,
    };
    state.store.save_translate_config(&tc)
}

/// Validate URL(s) in a TranslateProviderConfig.
fn validate_provider_config(
    provider_config: &TranslateProviderConfig,
) -> std::result::Result<(), PebbleError> {
    match provider_config {
        TranslateProviderConfig::DeepLX { endpoint } => validate_translate_url(endpoint),
        TranslateProviderConfig::GenericApi { endpoint, .. } => validate_translate_url(endpoint),
        TranslateProviderConfig::LLM { endpoint, .. } => validate_translate_url(endpoint),
        TranslateProviderConfig::DeepL { .. } => Ok(()), // uses official API, no custom URL
    }
}

/// Validate that a translate endpoint URL is safe: HTTPS is required, plain
/// HTTP is only allowed for real loopback hosts.
fn validate_translate_url(raw: &str) -> std::result::Result<(), PebbleError> {
    let url = url::Url::parse(raw.trim())
        .map_err(|_| PebbleError::Validation("Invalid translate endpoint URL".into()))?;
    if url.host().is_none() {
        return Err(PebbleError::Validation(
            "Translate endpoint URL must include a host".into(),
        ));
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" => {
            if !url.username().is_empty() || url.password().is_some() {
                return Err(PebbleError::Validation(
                    "Credentials are not allowed in plain HTTP endpoint URLs".into(),
                ));
            }
            let is_loopback = match url.host() {
                Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
                Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                None => false,
            };
            if is_loopback {
                Ok(())
            } else {
                Err(PebbleError::Validation(
                    "Only HTTPS URLs are allowed for remote services".into(),
                ))
            }
        }
        _ => Err(PebbleError::Validation("Unsupported URL scheme".into())),
    }
}

#[tauri::command]
pub async fn test_translate_connection(
    state: State<'_, AppState>,
    config: String,
) -> std::result::Result<String, PebbleError> {
    // Fills a blank API key from the stored config (same provider/endpoint
    // only) and validates endpoint URLs before making any requests.
    let provider_config = resolve_submitted_config(&state, &config)?;

    let proxy = get_global_proxy_raw(&state.crypto, &state.store)?;
    let result = TranslateService::translate_with_proxy(
        &provider_config,
        proxy.as_ref(),
        "Hello",
        "en",
        "zh",
    )
    .await?;
    Ok(result.translated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_store::Store;

    fn save_config(store: &Store, config: &str) {
        let now = now_timestamp();
        store
            .save_translate_config(&TranslateConfig {
                id: ACTIVE_TRANSLATE_CONFIG_ID.to_string(),
                provider_type: "deepl".to_string(),
                config: config.to_string(),
                is_enabled: true,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    #[test]
    fn legacy_encrypted_translate_config_is_lazily_migrated_to_v1() {
        let crypto = pebble_crypto::CryptoService::from_key([11_u8; 32]);
        let store = Store::open_in_memory().unwrap();
        let plaintext = r#"{"api_key":"secret"}"#;
        let legacy = hex_encode(&crypto.encrypt(plaintext.as_bytes()).unwrap());
        save_config(&store, &legacy);

        assert_eq!(
            decrypt_config_with_store(&crypto, &store, &legacy).unwrap(),
            plaintext
        );

        let migrated_hex = store.get_translate_config().unwrap().unwrap().config;
        let migrated = hex_decode(&migrated_hex).unwrap();
        assert!(!pebble_crypto::CryptoService::ciphertext_needs_migration(
            &migrated
        ));
        assert_eq!(
            crypto
                .decrypt_for(
                    TRANSLATE_CONFIG_PURPOSE,
                    ACTIVE_TRANSLATE_CONFIG_ID,
                    &migrated,
                )
                .unwrap(),
            plaintext.as_bytes()
        );
    }

    #[test]
    fn plaintext_translate_config_is_lazily_migrated_to_v1() {
        let crypto = pebble_crypto::CryptoService::from_key([11_u8; 32]);
        let store = Store::open_in_memory().unwrap();
        let plaintext = r#"{"type":"deepl","api_key":"secret","use_free_api":false}"#;
        save_config(&store, plaintext);

        assert_eq!(
            decrypt_config_with_store(&crypto, &store, plaintext).unwrap(),
            plaintext
        );

        let migrated_hex = store.get_translate_config().unwrap().unwrap().config;
        let migrated = hex_decode(&migrated_hex).unwrap();
        assert!(!pebble_crypto::CryptoService::ciphertext_needs_migration(
            &migrated
        ));
    }

    #[test]
    fn translate_url_validation_uses_real_host_parsing() {
        for ok in [
            "https://api.example.com/translate",
            "http://localhost:1188/translate",
            "http://127.0.0.1:1188/translate",
            "http://127.1.2.3/translate",
            "http://[::1]:1188/translate",
            "http://LOCALHOST/translate",
        ] {
            assert!(validate_translate_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://localhost:1@evil.com/",
            "http://localhost@evil.com/",
            "http://127.0.0.1.evil.com/",
            "http://evil.com/localhost",
            "http://localhost.evil.com/",
            "ftp://localhost/",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert!(validate_translate_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn masked_translate_config_hides_api_key() {
        let masked =
            mask_provider_config(r#"{"type":"llm","endpoint":"https://api.example.com","api_key":"sk-secret","model":"m","mode":"completions"}"#)
                .unwrap();
        assert!(!masked.contains("sk-secret"));
        let value: serde_json::Value = serde_json::from_str(&masked).unwrap();
        assert_eq!(value["has_api_key"], true);
        assert_eq!(value["api_key"], "");

        let masked =
            mask_provider_config(r#"{"type":"deeplx","endpoint":"http://localhost:1188"}"#)
                .unwrap();
        let value: serde_json::Value = serde_json::from_str(&masked).unwrap();
        assert_eq!(value["has_api_key"], false);
    }

    #[test]
    fn blank_api_key_reuses_stored_key_only_for_same_endpoint() {
        let stored: TranslateProviderConfig = serde_json::from_str(
            r#"{"type":"llm","endpoint":"https://api.example.com","api_key":"sk-secret","model":"m","mode":"completions"}"#,
        )
        .unwrap();
        let same: TranslateProviderConfig = serde_json::from_str(
            r#"{"type":"llm","endpoint":"https://api.example.com","api_key":"","model":"m2","mode":"completions"}"#,
        )
        .unwrap();
        let moved: TranslateProviderConfig = serde_json::from_str(
            r#"{"type":"llm","endpoint":"https://attacker.example","api_key":"","model":"m","mode":"completions"}"#,
        )
        .unwrap();

        assert_eq!(
            provider_api_key(&merge_stored_api_key(same, Some(&stored))),
            Some("sk-secret")
        );
        assert_eq!(
            provider_api_key(&merge_stored_api_key(moved, Some(&stored))),
            None
        );
    }

    #[test]
    fn restored_translate_config_is_validated() {
        let (provider, normalized) = validate_restored_translate_config(
            r#"{"type":"deepl","api_key":"k","use_free_api":true}"#,
        )
        .unwrap();
        assert_eq!(provider, "deepl");
        assert!(normalized.contains("\"api_key\":\"k\""));
        assert!(validate_restored_translate_config(
            r#"{"type":"deeplx","endpoint":"http://evil.example/x"}"#
        )
        .is_err());
        assert!(validate_restored_translate_config(r#"{"anything":true}"#).is_err());
    }

    #[test]
    fn invalid_legacy_plaintext_config_is_not_migrated() {
        let crypto = pebble_crypto::CryptoService::from_key([11_u8; 32]);
        let store = Store::open_in_memory().unwrap();
        let plaintext = r#"{"type":"deeplx","endpoint":"http://evil.example/x"}"#;
        save_config(&store, plaintext);

        assert!(decrypt_config_with_store(&crypto, &store, plaintext).is_err());
        assert_eq!(
            store.get_translate_config().unwrap().unwrap().config,
            plaintext
        );
    }

    #[test]
    fn translate_config_rejects_non_ascii_hex_without_panicking() {
        let result = std::panic::catch_unwind(|| hex_decode("aéx"));

        assert!(
            result.is_ok(),
            "invalid translate config hex must not panic"
        );
        let error = result.unwrap().unwrap_err().to_string();
        assert!(error.contains("Invalid hex"));
    }
}
