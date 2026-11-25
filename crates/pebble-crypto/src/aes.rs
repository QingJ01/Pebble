use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use pebble_core::{PebbleError, Result};
use rand::RngCore;

const NONCE_SIZE: usize = 12;
const ENVELOPE_MAGIC: &[u8; 7] = b"PEBBLE\0";
const ENVELOPE_V1: u8 = 1;
const ENVELOPE_V1_HEADER: &[u8; 8] = b"PEBBLE\0\x01";

#[derive(Debug, PartialEq, Eq)]
pub struct DecryptedEnvelope {
    pub plaintext: Vec<u8>,
    pub needs_migration: bool,
}

/// Encrypt plaintext with AES-256-GCM.
/// Returns nonce (12 bytes) || ciphertext || tag.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    encrypt_with_aad(key, plaintext, &[])
}

/// Encrypt plaintext with AES-256-GCM and bind it to additional authenticated data.
/// Returns nonce (12 bytes) || ciphertext || tag.
pub fn encrypt_with_aad(key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| PebbleError::Auth(format!("Invalid key: {e}")))?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| PebbleError::Auth(format!("Encryption failed: {e}")))?;

    let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypt data produced by `encrypt`.
/// Expects nonce (12 bytes) || ciphertext || tag.
pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    decrypt_with_aad(key, data, &[])
}

/// Decrypt data produced by `encrypt_with_aad` using the same authenticated data.
pub fn decrypt_with_aad(key: &[u8; 32], data: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    if data.len() < NONCE_SIZE + 16 {
        return Err(PebbleError::Auth("Ciphertext too short".to_string()));
    }

    let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| PebbleError::Auth(format!("Invalid key: {e}")))?;

    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|e| PebbleError::Auth(format!("Decryption failed: {e}")))
}

/// Encrypt with AAD and prefix the authenticated payload with the v1 envelope header.
pub fn encrypt_enveloped(key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let authenticated_aad = envelope_aad(aad);
    let encrypted = encrypt_with_aad(key, plaintext, &authenticated_aad)?;
    let mut envelope = Vec::with_capacity(ENVELOPE_V1_HEADER.len() + encrypted.len());
    envelope.extend_from_slice(ENVELOPE_V1_HEADER);
    envelope.extend_from_slice(&encrypted);
    Ok(envelope)
}

/// Decrypt a v1 envelope or a legacy unversioned ciphertext.
///
/// Once the envelope magic is present, any version or authentication error is final. It must not
/// fall back to legacy decryption because doing so would bypass the envelope's authenticated
/// context.
pub fn decrypt_enveloped(key: &[u8; 32], data: &[u8], aad: &[u8]) -> Result<DecryptedEnvelope> {
    if data.starts_with(ENVELOPE_MAGIC) {
        let version = data
            .get(ENVELOPE_MAGIC.len())
            .copied()
            .ok_or_else(|| PebbleError::Auth("Truncated ciphertext envelope header".to_string()))?;
        if version != ENVELOPE_V1 {
            return Err(PebbleError::Auth(format!(
                "Unsupported ciphertext envelope version: {version}"
            )));
        }

        let authenticated_aad = envelope_aad(aad);
        let plaintext =
            decrypt_with_aad(key, &data[ENVELOPE_V1_HEADER.len()..], &authenticated_aad)?;
        return Ok(DecryptedEnvelope {
            plaintext,
            needs_migration: false,
        });
    }

    // Legacy ciphertext without AAD. Callers re-encrypt it lazily after a
    // successful read; keep the fallback but make it visible in the logs.
    let plaintext = decrypt(key, data)?;
    tracing::warn!(
        "decrypted legacy ciphertext without authenticated context; it should be re-encrypted"
    );
    Ok(DecryptedEnvelope {
        plaintext,
        needs_migration: true,
    })
}

pub fn envelope_needs_migration(data: &[u8]) -> bool {
    !data.starts_with(ENVELOPE_MAGIC)
}

fn envelope_aad(aad: &[u8]) -> Vec<u8> {
    let mut authenticated_aad = Vec::with_capacity(ENVELOPE_V1_HEADER.len() + aad.len());
    authenticated_aad.extend_from_slice(ENVELOPE_V1_HEADER);
    authenticated_aad.extend_from_slice(aad);
    authenticated_aad
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        key
    }

    #[test]
    fn test_encrypt_decrypt_round_trip() {
        let key = test_key();
        let plaintext = b"hello world, this is a secret!";
        let encrypted = encrypt(&key, plaintext).unwrap();
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_wrong_key_fails() {
        let key1 = test_key();
        let key2 = test_key();
        let encrypted = encrypt(&key1, b"secret data").unwrap();
        assert!(decrypt(&key2, &encrypted).is_err());
    }

    #[test]
    fn test_decrypt_truncated_data_fails() {
        let key = test_key();
        assert!(decrypt(&key, &[0u8; 10]).is_err());
    }

    #[test]
    fn test_encrypt_empty_plaintext() {
        let key = test_key();
        let encrypted = encrypt(&key, b"").unwrap();
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, b"");
    }

    #[test]
    fn aad_must_match_for_decryption() {
        let key = test_key();
        let encrypted = encrypt_with_aad(&key, b"secret", b"accounts.auth_data/account-1").unwrap();

        assert_eq!(
            decrypt_with_aad(&key, &encrypted, b"accounts.auth_data/account-1").unwrap(),
            b"secret"
        );
        assert!(decrypt_with_aad(&key, &encrypted, b"accounts.auth_data/account-2").is_err());
    }

    #[test]
    fn envelope_reads_legacy_ciphertext_and_marks_it_for_migration() {
        let key = test_key();
        let legacy = encrypt(&key, b"legacy secret").unwrap();

        let decrypted = decrypt_enveloped(&key, &legacy, b"new context").unwrap();

        assert_eq!(decrypted.plaintext, b"legacy secret");
        assert!(decrypted.needs_migration);
    }

    #[test]
    fn v1_envelope_authentication_failure_never_falls_back_to_legacy() {
        let key = test_key();
        let nonce_bytes = *b"PEBBLE\0\x01ABCD";
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let legacy_ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                b"legacy plaintext".as_ref(),
            )
            .unwrap();
        let mut crafted_legacy = nonce_bytes.to_vec();
        crafted_legacy.extend_from_slice(&legacy_ciphertext);

        assert_eq!(decrypt(&key, &crafted_legacy).unwrap(), b"legacy plaintext");
        assert!(decrypt_enveloped(&key, &crafted_legacy, b"v1 context").is_err());
    }

    #[test]
    fn envelope_rejects_unknown_versions() {
        let key = test_key();
        let mut encrypted = encrypt_enveloped(&key, b"secret", b"context").unwrap();
        encrypted[7] = 2;

        let error = decrypt_enveloped(&key, &encrypted, b"context")
            .unwrap_err()
            .to_string();

        assert!(error.contains("Unsupported ciphertext envelope version"));
    }
}
