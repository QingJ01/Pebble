use pebble_core::{PebbleError, Result};
use rand::RngCore;
use tracing::info;
use zeroize::Zeroizing;

const SERVICE_NAME: &str = "com.pebble.email";
const KEY_ENTRY: &str = "master-dek";

pub struct KeyStore;

impl KeyStore {
    /// Get or create the Data Encryption Key from the OS credential store.
    pub fn get_or_create_dek() -> Result<Zeroizing<[u8; 32]>> {
        let entry = keyring::Entry::new(SERVICE_NAME, KEY_ENTRY)
            .map_err(|e| PebbleError::Auth(format!("Keyring entry error: {e}")))?;

        match entry.get_secret() {
            Ok(secret) => {
                if secret.len() != 32 {
                    return Err(PebbleError::Auth(format!(
                        "Invalid DEK length: expected 32, got {}",
                        secret.len()
                    )));
                }
                let mut key = Zeroizing::new([0u8; 32]);
                key.copy_from_slice(&secret);
                Ok(key)
            }
            Err(keyring::Error::NoEntry) => {
                info!("No DEK found, generating new one");
                let mut key = Zeroizing::new([0u8; 32]);
                rand::thread_rng().fill_bytes(&mut *key);
                entry
                    .set_secret(&*key)
                    .map_err(|e| PebbleError::Auth(format!("Failed to store DEK: {e}")))?;
                Ok(key)
            }
            Err(e) => Err(PebbleError::Auth(format!("Keyring read error: {e}"))),
        }
    }

    /// Delete the DEK from the OS credential store.
    pub fn delete_dek() -> Result<()> {
        let entry = keyring::Entry::new(SERVICE_NAME, KEY_ENTRY)
            .map_err(|e| PebbleError::Auth(format!("Keyring entry error: {e}")))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // Already gone
            Err(e) => Err(PebbleError::Auth(format!("Failed to delete DEK: {e}"))),
        }
    }
}
