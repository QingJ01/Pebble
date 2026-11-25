use crate::state::AppState;
use pebble_core::Result;
use pebble_crypto::CryptoService;
use pebble_store::Store;
use std::sync::Arc;
use tokio::sync::OwnedMutexGuard;

pub(crate) const ACCOUNT_AUTH_DATA_PURPOSE: &str = "accounts.auth_data";
pub(crate) const SECURE_USER_DATA_PURPOSE: &str = "secure_user_data.value";
pub(crate) const TRANSLATE_CONFIG_PURPOSE: &str = "translate_config.config";
pub(crate) const ACTIVE_TRANSLATE_CONFIG_ID: &str = "active";

pub(crate) async fn lock_secure_user_data_key(state: &AppState, key: &str) -> OwnedMutexGuard<()> {
    let key_lock = {
        let mut locks = state.secure_user_data_locks.lock().await;
        Arc::clone(
            locks
                .entry(key.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    };
    key_lock.lock_owned().await
}

pub(crate) fn encrypt_account_auth_data(
    crypto: &CryptoService,
    account_id: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    crypto.encrypt_for(ACCOUNT_AUTH_DATA_PURPOSE, account_id, plaintext)
}

pub(crate) fn decrypt_account_auth_data(
    crypto: &CryptoService,
    store: &Store,
    account_id: &str,
    encrypted: &[u8],
) -> Result<Vec<u8>> {
    let needs_migration = CryptoService::ciphertext_needs_migration(encrypted);
    let plaintext = crypto.decrypt_for(ACCOUNT_AUTH_DATA_PURPOSE, account_id, encrypted)?;
    if needs_migration {
        let replacement = encrypt_account_auth_data(crypto, account_id, &plaintext)?;
        store.compare_exchange_auth_data(account_id, encrypted, &replacement)?;
    }
    Ok(plaintext)
}

pub(crate) fn load_account_auth_data(
    crypto: &CryptoService,
    store: &Store,
    account_id: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(encrypted) = store.get_auth_data(account_id)? else {
        return Ok(None);
    };
    decrypt_account_auth_data(crypto, store, account_id, &encrypted).map(Some)
}

pub(crate) fn store_account_auth_data(
    crypto: &CryptoService,
    store: &Store,
    account_id: &str,
    plaintext: &[u8],
) -> Result<()> {
    let encrypted = encrypt_account_auth_data(crypto, account_id, plaintext)?;
    store.set_auth_data(account_id, &encrypted)
}

pub(crate) fn encrypt_secure_user_data(
    crypto: &CryptoService,
    key: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    crypto.encrypt_for(SECURE_USER_DATA_PURPOSE, key, plaintext)
}

pub(crate) fn decrypt_secure_user_data(
    crypto: &CryptoService,
    store: &Store,
    key: &str,
    encrypted: &[u8],
) -> Result<Vec<u8>> {
    let needs_migration = CryptoService::ciphertext_needs_migration(encrypted);
    let plaintext = crypto.decrypt_for(SECURE_USER_DATA_PURPOSE, key, encrypted)?;
    if needs_migration {
        let replacement = encrypt_secure_user_data(crypto, key, &plaintext)?;
        store.compare_exchange_secure_user_data(key, encrypted, &replacement)?;
    }
    Ok(plaintext)
}

pub(crate) fn load_secure_user_data(
    crypto: &CryptoService,
    store: &Store,
    key: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(encrypted) = store.get_secure_user_data(key)? else {
        return Ok(None);
    };
    decrypt_secure_user_data(crypto, store, key, &encrypted).map(Some)
}

pub(crate) fn store_secure_user_data(
    crypto: &CryptoService,
    store: &Store,
    key: &str,
    plaintext: &[u8],
) -> Result<()> {
    let encrypted = encrypt_secure_user_data(crypto, key, plaintext)?;
    store.set_secure_user_data(key, &encrypted)
}

/// `secure_user_data` key whose (v1-encrypted) presence records that every
/// legacy unversioned ciphertext has been re-encrypted. Encrypting the marker
/// with its own AAD means it cannot be copied from another record.
pub(crate) const AAD_MIGRATION_DONE_KEY: &str = "__pebble.aad_migration.v1";
const AAD_MIGRATION_DONE_VALUE: &[u8] = b"done";

/// Outcome of [`migrate_legacy_ciphertexts`].
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LegacyCiphertextMigration {
    /// Records re-encrypted from the legacy format during this run.
    pub migrated: usize,
    /// Legacy records that could not be decrypted (left untouched).
    pub failed: usize,
    /// Whether the migration is complete and legacy ciphertext is now rejected.
    pub completed: bool,
}

/// Whether a previous run already completed the migration.
pub(crate) fn legacy_ciphertext_migration_done(crypto: &CryptoService, store: &Store) -> bool {
    match store.get_secure_user_data(AAD_MIGRATION_DONE_KEY) {
        Ok(Some(marker)) => {
            !CryptoService::ciphertext_needs_migration(&marker)
                && crypto
                    .decrypt_for(SECURE_USER_DATA_PURPOSE, AAD_MIGRATION_DONE_KEY, &marker)
                    .is_ok_and(|value| value == AAD_MIGRATION_DONE_VALUE)
        }
        _ => false,
    }
}

fn count_legacy_records(store: &Store) -> Result<usize> {
    let mut legacy = 0;
    for key in store.list_secure_user_data_keys()? {
        if let Some(value) = store.get_secure_user_data(&key)? {
            legacy += usize::from(CryptoService::ciphertext_needs_migration(&value));
        }
    }
    for account_id in store.list_account_ids_with_auth_data()? {
        if let Some(value) = store.get_auth_data(&account_id)? {
            legacy += usize::from(CryptoService::ciphertext_needs_migration(&value));
        }
    }
    if let Some(config) = store.get_translate_config()? {
        let is_legacy = serde_json::from_str::<serde_json::Value>(&config.config).is_ok()
            || crate::commands::translate::hex_decode(&config.config)
                .map(|bytes| CryptoService::ciphertext_needs_migration(&bytes))
                .unwrap_or(false);
        legacy += usize::from(is_legacy);
    }
    for job in store.list_scheduled_sends()? {
        legacy += usize::from(CryptoService::ciphertext_needs_migration(&job.payload));
    }
    Ok(legacy)
}

/// One-time, idempotent re-encryption of every legacy (AAD-less) record in
/// `secure_user_data`, `accounts.auth_data` and `translate_config`.
///
/// Records are rewritten with compare-and-swap, so a concurrent writer always
/// wins (its value is already v1). When no legacy record remains, a marker is
/// stored and the crypto service stops accepting unversioned ciphertext; on
/// later startups the marker alone switches strict mode on.
pub(crate) fn migrate_legacy_ciphertexts(
    crypto: &CryptoService,
    store: &Store,
) -> Result<LegacyCiphertextMigration> {
    let mut outcome = LegacyCiphertextMigration::default();
    if legacy_ciphertext_migration_done(crypto, store) {
        crypto.reject_legacy_ciphertext();
        outcome.completed = true;
        return Ok(outcome);
    }

    for key in store.list_secure_user_data_keys()? {
        let Some(encrypted) = store.get_secure_user_data(&key)? else {
            continue;
        };
        if !CryptoService::ciphertext_needs_migration(&encrypted) {
            continue;
        }
        match decrypt_secure_user_data(crypto, store, &key, &encrypted) {
            Ok(_) => outcome.migrated += 1,
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!("Could not re-encrypt legacy secure user data {key}: {error}");
            }
        }
    }

    for account_id in store.list_account_ids_with_auth_data()? {
        let Some(encrypted) = store.get_auth_data(&account_id)? else {
            continue;
        };
        if !CryptoService::ciphertext_needs_migration(&encrypted) {
            continue;
        }
        match decrypt_account_auth_data(crypto, store, &account_id, &encrypted) {
            Ok(_) => outcome.migrated += 1,
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!("Could not re-encrypt legacy auth data for {account_id}: {error}");
            }
        }
    }

    if let Some(config) = store.get_translate_config()? {
        let before = config.config.clone();
        match crate::commands::translate::decrypt_config_with_store(crypto, store, &config.config) {
            Ok(_) => {
                let after = store.get_translate_config()?.map(|config| config.config);
                if after.as_deref() != Some(before.as_str()) {
                    outcome.migrated += 1;
                }
            }
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!("Could not re-encrypt legacy translate config: {error}");
            }
        }
    }

    // Re-scan: compare-and-swap losers and failures leave legacy rows behind.
    let remaining = count_legacy_records(store)?;
    if remaining == 0 {
        let marker =
            encrypt_secure_user_data(crypto, AAD_MIGRATION_DONE_KEY, AAD_MIGRATION_DONE_VALUE)?;
        store.set_secure_user_data(AAD_MIGRATION_DONE_KEY, &marker)?;
        crypto.reject_legacy_ciphertext();
        outcome.completed = true;
    } else {
        tracing::warn!(
            "{remaining} legacy ciphertext record(s) remain; legacy decryption stays enabled"
        );
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{now_timestamp, Account, ProviderType};

    fn insert_account(store: &Store, id: &str) {
        store
            .insert_account(&Account {
                account_label: None,
                provider_display_name: None,
                id: id.to_string(),
                email: format!("{id}@example.com"),
                display_name: id.to_string(),
                color: None,
                provider: ProviderType::Imap,
                created_at: now_timestamp(),
                updated_at: now_timestamp(),
            })
            .unwrap();
    }

    #[test]
    fn loading_legacy_account_auth_data_lazily_migrates_it_to_v1() {
        let crypto = CryptoService::from_key([9_u8; 32]);
        let store = Store::open_in_memory().unwrap();
        insert_account(&store, "account-1");
        let legacy = crypto.encrypt(b"legacy auth data").unwrap();
        store.set_auth_data("account-1", &legacy).unwrap();

        assert_eq!(
            load_account_auth_data(&crypto, &store, "account-1")
                .unwrap()
                .unwrap(),
            b"legacy auth data"
        );

        let migrated = store.get_auth_data("account-1").unwrap().unwrap();
        assert!(!CryptoService::ciphertext_needs_migration(&migrated));
        assert_eq!(
            crypto
                .decrypt_for(ACCOUNT_AUTH_DATA_PURPOSE, "account-1", &migrated)
                .unwrap(),
            b"legacy auth data"
        );
    }

    #[test]
    fn account_auth_data_cannot_be_moved_between_records() {
        let crypto = CryptoService::from_key([9_u8; 32]);
        let store = Store::open_in_memory().unwrap();
        insert_account(&store, "account-1");
        insert_account(&store, "account-2");
        let encrypted =
            encrypt_account_auth_data(&crypto, "account-1", b"account one secret").unwrap();
        store.set_auth_data("account-2", &encrypted).unwrap();

        assert!(load_account_auth_data(&crypto, &store, "account-2").is_err());
    }

    fn save_translate_config(store: &Store, config: &str) {
        store
            .save_translate_config(&pebble_core::TranslateConfig {
                id: ACTIVE_TRANSLATE_CONFIG_ID.to_string(),
                provider_type: "deepl".to_string(),
                config: config.to_string(),
                is_enabled: true,
                created_at: now_timestamp(),
                updated_at: now_timestamp(),
            })
            .unwrap();
    }

    #[test]
    fn startup_migration_reencrypts_every_legacy_record_then_rejects_legacy() {
        let crypto = CryptoService::from_key([9_u8; 32]);
        let store = Store::open_in_memory().unwrap();
        insert_account(&store, "account-1");
        insert_account(&store, "account-2");
        store
            .set_auth_data("account-1", &crypto.encrypt(b"auth one").unwrap())
            .unwrap();
        store
            .set_auth_data(
                "account-2",
                &encrypt_account_auth_data(&crypto, "account-2", b"auth two").unwrap(),
            )
            .unwrap();
        store
            .set_secure_user_data("templates", &crypto.encrypt(b"tpl").unwrap())
            .unwrap();
        store
            .set_secure_user_data("signatures", &crypto.encrypt(b"sig").unwrap())
            .unwrap();
        let translate_plaintext = r#"{"api_key":"secret"}"#;
        save_translate_config(
            &store,
            &crate::commands::translate::hex_encode(
                &crypto.encrypt(translate_plaintext.as_bytes()).unwrap(),
            ),
        );

        let outcome = migrate_legacy_ciphertexts(&crypto, &store).unwrap();
        assert_eq!(
            outcome,
            LegacyCiphertextMigration {
                migrated: 4,
                failed: 0,
                completed: true
            }
        );
        assert!(!crypto.legacy_ciphertext_allowed());
        assert_eq!(count_legacy_records(&store).unwrap(), 0);
        assert_eq!(
            load_account_auth_data(&crypto, &store, "account-1")
                .unwrap()
                .unwrap(),
            b"auth one"
        );
        assert_eq!(
            load_secure_user_data(&crypto, &store, "signatures")
                .unwrap()
                .unwrap(),
            b"sig"
        );
        let config = store.get_translate_config().unwrap().unwrap().config;
        assert_eq!(
            crate::commands::translate::decrypt_config_with_store(&crypto, &store, &config)
                .unwrap(),
            translate_plaintext
        );

        // Legacy ciphertext written after completion (e.g. tampering) is rejected.
        store
            .set_secure_user_data("templates", &crypto.encrypt(b"tpl").unwrap())
            .unwrap();
        assert!(load_secure_user_data(&crypto, &store, "templates").is_err());

        // A fresh process sees the marker and goes strict without rescanning.
        let restarted = CryptoService::from_key([9_u8; 32]);
        assert!(legacy_ciphertext_migration_done(&restarted, &store));
        let outcome = migrate_legacy_ciphertexts(&restarted, &store).unwrap();
        assert_eq!(outcome.migrated, 0);
        assert!(outcome.completed);
        assert!(!restarted.legacy_ciphertext_allowed());
    }

    #[test]
    fn startup_migration_stays_lenient_while_a_legacy_record_cannot_be_read() {
        let crypto = CryptoService::from_key([9_u8; 32]);
        let other_key = CryptoService::from_key([3_u8; 32]);
        let store = Store::open_in_memory().unwrap();
        store
            .set_secure_user_data("templates", &crypto.encrypt(b"tpl").unwrap())
            .unwrap();
        store
            .set_secure_user_data("broken", &other_key.encrypt(b"x").unwrap())
            .unwrap();

        let outcome = migrate_legacy_ciphertexts(&crypto, &store).unwrap();

        assert_eq!(outcome.migrated, 1);
        assert_eq!(outcome.failed, 1);
        assert!(!outcome.completed);
        assert!(crypto.legacy_ciphertext_allowed());
        assert!(!legacy_ciphertext_migration_done(&crypto, &store));
    }

    #[test]
    fn migration_marker_copied_from_another_key_is_not_trusted() {
        let crypto = CryptoService::from_key([9_u8; 32]);
        let store = Store::open_in_memory().unwrap();
        let forged = encrypt_secure_user_data(&crypto, "templates", b"done").unwrap();
        store
            .set_secure_user_data(AAD_MIGRATION_DONE_KEY, &forged)
            .unwrap();
        assert!(!legacy_ciphertext_migration_done(&crypto, &store));
    }

    #[test]
    fn loading_legacy_secure_user_data_lazily_migrates_it_to_v1() {
        let crypto = CryptoService::from_key([9_u8; 32]);
        let store = Store::open_in_memory().unwrap();
        let legacy = crypto.encrypt(b"legacy user data").unwrap();
        store.set_secure_user_data("templates", &legacy).unwrap();

        assert_eq!(
            load_secure_user_data(&crypto, &store, "templates")
                .unwrap()
                .unwrap(),
            b"legacy user data"
        );

        let migrated = store.get_secure_user_data("templates").unwrap().unwrap();
        assert!(!CryptoService::ciphertext_needs_migration(&migrated));
        assert_eq!(
            crypto
                .decrypt_for(SECURE_USER_DATA_PURPOSE, "templates", &migrated)
                .unwrap(),
            b"legacy user data"
        );
    }
}
