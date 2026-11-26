use argon2::{Algorithm, Argon2, Params, Version};
use pebble_core::{PebbleError, Result};
use rand::RngCore;
use ring::pbkdf2;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use zeroize::Zeroizing;

const SALT_SIZE: usize = 16;
const AES_GCM_MIN_PAYLOAD_SIZE: usize = 12 + 16;
const ALGORITHM: &str = "aes-256-gcm";
const BACKUP_SECRET_AAD: &[u8] = b"pebble.backup.secrets.v1";

/// Minimum length (in Unicode scalar values) for new backup passphrases.
pub const MIN_PASSPHRASE_CHARS: usize = 12;

/// Legacy KDF: still accepted for decryption of existing backups.
const KDF_PBKDF2: &str = "pbkdf2-hmac-sha256";
const PBKDF2_ITERATIONS: u32 = 210_000;

/// Current KDF for new encryptions.
const KDF_ARGON2ID: &str = "argon2id";
const ARGON2_MEMORY_KIB: u32 = 64 * 1024;
const ARGON2_TIME_COST: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PassphraseEncryptedBlob {
    pub algorithm: String,
    pub kdf: String,
    /// PBKDF2 iteration count, or the Argon2 time cost.
    pub iterations: u32,
    /// Argon2 memory cost in KiB (absent for PBKDF2 blobs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_kib: Option<u32>,
    /// Argon2 parallelism (absent for PBKDF2 blobs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
    pub salt_hex: String,
    pub ciphertext_hex: String,
}

/// Encrypt with a passphrase the user is setting now: the passphrase must
/// satisfy the minimum-length policy.
pub fn encrypt_with_passphrase(
    plaintext: &[u8],
    passphrase: &str,
) -> Result<PassphraseEncryptedBlob> {
    validate_new_passphrase(passphrase)?;
    encrypt_with_stored_passphrase(plaintext, passphrase)
}

/// Encrypt with a passphrase that was already saved before the minimum-length
/// policy existed (e.g. the auto-backup passphrase). Only a non-empty
/// passphrase is required, so existing configurations keep working.
pub fn encrypt_with_stored_passphrase(
    plaintext: &[u8],
    passphrase: &str,
) -> Result<PassphraseEncryptedBlob> {
    validate_passphrase_present(passphrase)?;

    let mut salt = [0u8; SALT_SIZE];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = derive_argon2id_key(passphrase, &salt)?;
    let ciphertext = crate::aes::encrypt_enveloped(&key, plaintext, BACKUP_SECRET_AAD)?;

    Ok(PassphraseEncryptedBlob {
        algorithm: ALGORITHM.to_string(),
        kdf: KDF_ARGON2ID.to_string(),
        iterations: ARGON2_TIME_COST,
        memory_kib: Some(ARGON2_MEMORY_KIB),
        parallelism: Some(ARGON2_PARALLELISM),
        salt_hex: hex_encode(&salt),
        ciphertext_hex: hex_encode(&ciphertext),
    })
}

pub fn decrypt_with_passphrase(
    blob: &PassphraseEncryptedBlob,
    passphrase: &str,
) -> Result<Vec<u8>> {
    // Old backups may have been created before the minimum length existed,
    // so decryption only requires a non-empty passphrase.
    validate_passphrase_present(passphrase)?;
    validate_blob(blob)?;

    let salt = hex_decode(&blob.salt_hex)?;
    if salt.len() != SALT_SIZE {
        return Err(PebbleError::Validation(format!(
            "Invalid backup secret salt length: expected {SALT_SIZE} bytes, got {}",
            salt.len()
        )));
    }
    let ciphertext = hex_decode(&blob.ciphertext_hex)?;
    if ciphertext.len() < AES_GCM_MIN_PAYLOAD_SIZE {
        return Err(PebbleError::Validation(
            "Invalid backup secret ciphertext length".to_string(),
        ));
    }
    let key = if blob.kdf == KDF_ARGON2ID {
        derive_argon2id_key(passphrase, &salt)?
    } else {
        derive_pbkdf2_key(passphrase, &salt, blob.iterations)?
    };

    Ok(crate::aes::decrypt_enveloped(&key, &ciphertext, BACKUP_SECRET_AAD)?.plaintext)
}

/// Validate a passphrase that will be used to create a new backup.
pub fn validate_new_passphrase(passphrase: &str) -> Result<()> {
    validate_passphrase_present(passphrase)?;
    if passphrase.trim().chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(PebbleError::Validation(format!(
            "Backup encryption passphrase must be at least {MIN_PASSPHRASE_CHARS} characters"
        )));
    }
    Ok(())
}

fn validate_passphrase_present(passphrase: &str) -> Result<()> {
    if passphrase.trim().is_empty() {
        return Err(PebbleError::Validation(
            "Backup encryption passphrase is required".to_string(),
        ));
    }
    Ok(())
}

fn validate_blob(blob: &PassphraseEncryptedBlob) -> Result<()> {
    if blob.algorithm != ALGORITHM {
        return Err(PebbleError::Validation(format!(
            "Unsupported backup secret encryption algorithm: {}",
            blob.algorithm
        )));
    }
    // Only the exact parameters Pebble writes are accepted, so a tampered
    // backup cannot weaken the KDF or force huge memory/time costs.
    match blob.kdf.as_str() {
        KDF_PBKDF2 => {
            if blob.iterations != PBKDF2_ITERATIONS {
                return Err(PebbleError::Validation(format!(
                    "Unsupported backup secret KDF iterations: {}",
                    blob.iterations
                )));
            }
        }
        KDF_ARGON2ID => {
            if blob.iterations != ARGON2_TIME_COST
                || blob.memory_kib != Some(ARGON2_MEMORY_KIB)
                || blob.parallelism != Some(ARGON2_PARALLELISM)
            {
                return Err(PebbleError::Validation(
                    "Unsupported backup secret KDF parameters".to_string(),
                ));
            }
        }
        other => {
            return Err(PebbleError::Validation(format!(
                "Unsupported backup secret KDF: {other}"
            )));
        }
    }
    Ok(())
}

fn derive_argon2id_key(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_TIME_COST,
        ARGON2_PARALLELISM,
        Some(32),
    )
    .map_err(|error| PebbleError::Internal(format!("Invalid Argon2 parameters: {error}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut())
        .map_err(|error| PebbleError::Internal(format!("Argon2 key derivation failed: {error}")))?;
    Ok(key)
}

fn derive_pbkdf2_key(
    passphrase: &str,
    salt: &[u8],
    iterations: u32,
) -> Result<Zeroizing<[u8; 32]>> {
    let iterations = NonZeroU32::new(iterations).ok_or_else(|| {
        PebbleError::Validation(
            "Backup secret KDF iterations must be greater than zero".to_string(),
        )
    })?;
    let mut key = Zeroizing::new([0u8; 32]);
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        passphrase.as_bytes(),
        key.as_mut(),
    );
    Ok(key)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    hex::decode(s)
        .map_err(|error| PebbleError::Validation(format!("Invalid backup secret hex: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "correct horse battery";

    #[test]
    fn passphrase_encryption_emits_v1_envelope_with_argon2id() {
        let encrypted = encrypt_with_passphrase(b"backup secrets", PASS).unwrap();
        let ciphertext = hex_decode(&encrypted.ciphertext_hex).unwrap();

        assert!(ciphertext.starts_with(b"PEBBLE\0\x01"));
        assert_eq!(encrypted.kdf, KDF_ARGON2ID);
        assert_eq!(encrypted.memory_kib, Some(ARGON2_MEMORY_KIB));
        assert_eq!(
            decrypt_with_passphrase(&encrypted, PASS).unwrap(),
            b"backup secrets"
        );
    }

    #[test]
    fn new_passphrases_must_have_minimum_length() {
        let error = encrypt_with_passphrase(b"x", "short pass")
            .unwrap_err()
            .to_string();
        assert!(error.contains("at least 12 characters"), "{error}");
        assert!(validate_new_passphrase("            x").is_err());
        assert!(validate_new_passphrase("twelve chars").is_ok());
    }

    fn legacy_pbkdf2_blob(
        plaintext: &[u8],
        passphrase: &str,
        enveloped: bool,
    ) -> PassphraseEncryptedBlob {
        let salt = [9u8; SALT_SIZE];
        let key = derive_pbkdf2_key(passphrase, &salt, PBKDF2_ITERATIONS).unwrap();
        let ciphertext = if enveloped {
            crate::aes::encrypt_enveloped(&key, plaintext, BACKUP_SECRET_AAD).unwrap()
        } else {
            crate::aes::encrypt(&key, plaintext).unwrap()
        };
        PassphraseEncryptedBlob {
            algorithm: ALGORITHM.to_string(),
            kdf: KDF_PBKDF2.to_string(),
            iterations: PBKDF2_ITERATIONS,
            memory_kib: None,
            parallelism: None,
            salt_hex: hex_encode(&salt),
            ciphertext_hex: hex_encode(&ciphertext),
        }
    }

    #[test]
    fn decrypts_legacy_pbkdf2_blobs_with_short_passphrases() {
        let blob = legacy_pbkdf2_blob(b"old backup", "passphrase", true);
        // Legacy JSON (without the Argon2 fields) still deserialises.
        let json = serde_json::to_string(&blob).unwrap();
        assert!(!json.contains("memory_kib"));
        let blob: PassphraseEncryptedBlob = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decrypt_with_passphrase(&blob, "passphrase").unwrap(),
            b"old backup"
        );
    }

    #[test]
    fn passphrase_decryption_reads_legacy_ciphertext() {
        let blob = legacy_pbkdf2_blob(b"legacy backup secrets", "passphrase", false);
        assert_eq!(
            decrypt_with_passphrase(&blob, "passphrase").unwrap(),
            b"legacy backup secrets"
        );
    }

    #[test]
    fn stored_short_passphrases_can_still_encrypt() {
        let encrypted = encrypt_with_stored_passphrase(b"secret", "short").unwrap();
        assert_eq!(
            decrypt_with_passphrase(&encrypted, "short").unwrap(),
            b"secret"
        );
        assert!(encrypt_with_stored_passphrase(b"secret", "  ").is_err());
    }

    #[test]
    fn rejects_tampered_argon2_parameters() {
        let mut encrypted = encrypt_with_passphrase(b"secret", PASS).unwrap();
        encrypted.memory_kib = Some(8);
        let error = decrypt_with_passphrase(&encrypted, PASS)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Unsupported backup secret KDF parameters"));
    }
}
