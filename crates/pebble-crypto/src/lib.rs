pub mod aes;
pub mod keystore;
pub mod passphrase;

use pebble_core::{PebbleError, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use zeroize::{Zeroize, Zeroizing};

const CONTEXT_AAD_DOMAIN: &[u8] = b"pebble.crypto.context.v1\0";

/// Service that manages encryption/decryption using a DEK from the OS keystore.
pub struct CryptoService {
    dek: Zeroizing<[u8; 32]>,
    /// Whether `decrypt_for` still accepts legacy unversioned (AAD-less)
    /// ciphertext. Cleared once the one-time startup migration has
    /// re-encrypted every stored record.
    legacy_ciphertext_allowed: AtomicBool,
}

impl CryptoService {
    /// Initialize by loading (or creating) the DEK from the OS credential store.
    pub fn init() -> Result<Self> {
        let dek = keystore::KeyStore::get_or_create_dek()?;
        Ok(Self::with_dek(dek))
    }

    fn with_dek(dek: Zeroizing<[u8; 32]>) -> Self {
        Self {
            dek,
            legacy_ciphertext_allowed: AtomicBool::new(true),
        }
    }

    /// Construct the service from an existing data-encryption key.
    pub fn from_key(mut dek: [u8; 32]) -> Self {
        let protected_dek = Zeroizing::new(dek);
        dek.zeroize();
        Self::with_dek(protected_dek)
    }

    /// Encrypt plaintext bytes.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        aes::encrypt(&self.dek, plaintext)
    }

    /// Decrypt ciphertext bytes.
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        aes::decrypt(&self.dek, ciphertext)
    }

    /// Encrypt a value for one storage purpose and record identity.
    pub fn encrypt_for(&self, purpose: &str, record_id: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
        let aad = context_aad(purpose, record_id);
        aes::encrypt_enveloped(&self.dek, plaintext, &aad)
    }

    /// Decrypt a scoped v1 ciphertext, or read a legacy unscoped ciphertext for migration.
    pub fn decrypt_for(
        &self,
        purpose: &str,
        record_id: &str,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>> {
        if aes::envelope_needs_migration(ciphertext) && !self.legacy_ciphertext_allowed() {
            return Err(PebbleError::Auth(
                "Rejected unversioned ciphertext: legacy records were already migrated".to_string(),
            ));
        }
        let aad = context_aad(purpose, record_id);
        Ok(aes::decrypt_enveloped(&self.dek, ciphertext, &aad)?.plaintext)
    }

    /// Stop accepting legacy unversioned ciphertext in `decrypt_for`.
    /// Called once every stored legacy record has been re-encrypted.
    pub fn reject_legacy_ciphertext(&self) {
        self.legacy_ciphertext_allowed
            .store(false, Ordering::SeqCst);
    }

    /// Whether `decrypt_for` still falls back to legacy unversioned ciphertext.
    pub fn legacy_ciphertext_allowed(&self) -> bool {
        self.legacy_ciphertext_allowed.load(Ordering::SeqCst)
    }

    /// Whether ciphertext is legacy and should be re-encrypted after a successful read.
    pub fn ciphertext_needs_migration(ciphertext: &[u8]) -> bool {
        aes::envelope_needs_migration(ciphertext)
    }
}

fn context_aad(purpose: &str, record_id: &str) -> Vec<u8> {
    let mut aad =
        Vec::with_capacity(CONTEXT_AAD_DOMAIN.len() + 16 + purpose.len() + record_id.len());
    aad.extend_from_slice(CONTEXT_AAD_DOMAIN);
    aad.extend_from_slice(&(purpose.len() as u64).to_be_bytes());
    aad.extend_from_slice(purpose.as_bytes());
    aad.extend_from_slice(&(record_id.len() as u64).to_be_bytes());
    aad.extend_from_slice(record_id.as_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_crypto_service() -> CryptoService {
        CryptoService::with_dek(Zeroizing::new([7_u8; 32]))
    }

    #[test]
    fn scoped_ciphertext_is_bound_to_purpose_and_record_id() {
        let service = test_crypto_service();
        let encrypted = service
            .encrypt_for("accounts.auth_data", "account-1", b"secret")
            .unwrap();

        assert_eq!(
            service
                .decrypt_for("accounts.auth_data", "account-1", &encrypted)
                .unwrap(),
            b"secret"
        );
        assert!(service
            .decrypt_for("accounts.auth_data", "account-2", &encrypted)
            .is_err());
        assert!(service
            .decrypt_for("secure_user_data.value", "account-1", &encrypted)
            .is_err());
    }

    #[test]
    fn scoped_decryption_reads_legacy_ciphertext_for_lazy_migration() {
        let service = test_crypto_service();
        let legacy = service.encrypt(b"legacy secret").unwrap();

        assert!(CryptoService::ciphertext_needs_migration(&legacy));
        assert_eq!(
            service
                .decrypt_for("accounts.auth_data", "account-1", &legacy)
                .unwrap(),
            b"legacy secret"
        );

        let v1 = service
            .encrypt_for("accounts.auth_data", "account-1", b"new secret")
            .unwrap();
        assert!(!CryptoService::ciphertext_needs_migration(&v1));
    }

    #[test]
    fn legacy_ciphertext_is_rejected_once_migration_completed() {
        let service = test_crypto_service();
        let legacy = service.encrypt(b"legacy secret").unwrap();
        let v1 = service
            .encrypt_for("accounts.auth_data", "account-1", b"new secret")
            .unwrap();
        assert!(service.legacy_ciphertext_allowed());

        service.reject_legacy_ciphertext();

        assert!(!service.legacy_ciphertext_allowed());
        assert!(service
            .decrypt_for("accounts.auth_data", "account-1", &legacy)
            .is_err());
        assert_eq!(
            service
                .decrypt_for("accounts.auth_data", "account-1", &v1)
                .unwrap(),
            b"new secret"
        );
    }

    #[test]
    fn passphrase_encrypted_blob_round_trips() {
        let plaintext = br#"{"accounts":[{"id":"a1","password":"secret"}]}"#;

        let encrypted =
            passphrase::encrypt_with_passphrase(plaintext, "correct horse battery staple").unwrap();
        let decrypted =
            passphrase::decrypt_with_passphrase(&encrypted, "correct horse battery staple")
                .unwrap();

        assert_eq!(decrypted, plaintext);
        assert_ne!(encrypted.ciphertext_hex, String::from_utf8_lossy(plaintext));
    }

    #[test]
    fn passphrase_encrypted_blob_rejects_wrong_passphrase() {
        let encrypted =
            passphrase::encrypt_with_passphrase(b"secret", "right passphrase!").unwrap();

        let err = passphrase::decrypt_with_passphrase(&encrypted, "wrong passphrase")
            .unwrap_err()
            .to_string();

        assert!(err.contains("Decryption failed"));
    }

    #[test]
    fn passphrase_encrypted_blob_rejects_empty_passphrase() {
        let err = passphrase::encrypt_with_passphrase(b"secret", "")
            .unwrap_err()
            .to_string();

        assert!(err.contains("passphrase is required"));
    }

    #[test]
    fn passphrase_encrypted_blob_rejects_unsupported_iterations_before_decrypting() {
        let mut encrypted =
            passphrase::encrypt_with_passphrase(b"secret", "right passphrase!").unwrap();
        encrypted.iterations = 1;

        let err = passphrase::decrypt_with_passphrase(&encrypted, "right passphrase!")
            .unwrap_err()
            .to_string();

        assert!(err.contains("Unsupported backup secret KDF parameters"));
    }

    #[test]
    fn passphrase_encrypted_blob_rejects_non_ascii_hex_without_panicking() {
        let mut encrypted =
            passphrase::encrypt_with_passphrase(b"secret", "right passphrase!").unwrap();
        encrypted.salt_hex = "aéx".to_string();

        let result = std::panic::catch_unwind(|| {
            passphrase::decrypt_with_passphrase(&encrypted, "right passphrase!")
        });

        assert!(result.is_ok(), "invalid backup hex must not panic");
        let error = result.unwrap().unwrap_err().to_string();
        assert!(error.contains("Invalid backup secret hex"));
    }

    #[test]
    #[ignore] // Requires OS credential store access
    fn test_crypto_service_init() {
        let service = CryptoService::init();
        assert!(service.is_ok());
    }

    #[test]
    #[ignore] // Requires OS credential store access
    fn test_crypto_service_round_trip() {
        let service = CryptoService::init().unwrap();
        let plaintext = b"test credentials json";
        let encrypted = service.encrypt(plaintext).unwrap();
        let decrypted = service.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
