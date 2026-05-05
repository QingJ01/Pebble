use pebble_core::{PebbleError, Result};
use rand::RngCore;
use tracing::{info, warn};
use zeroize::Zeroizing;

const SERVICE_NAME: &str = "com.pebble.email";
const KEY_ENTRY: &str = "master-dek";
const DEK_LEN: usize = 32;

pub struct KeyStore;

impl KeyStore {
    /// Get or create the Data Encryption Key from the OS credential store.
    ///
    /// The raw 32-byte key is hex-encoded before storing so it can round-trip
    /// safely through string-based keychain backends and survive kernel-keyring
    /// serialisation.
    pub fn get_or_create_dek() -> Result<Zeroizing<[u8; DEK_LEN]>> {
        let entry = keyring::Entry::new(SERVICE_NAME, KEY_ENTRY)
            .map_err(|e| PebbleError::Auth(format!("Keyring entry error: {e}")))?;

        match entry.get_secret() {
            Ok(hex_secret) => {
                let hex_secret = Zeroizing::new(hex_secret);
                match decode_hex(&hex_secret) {
                    Ok(key) => {
                        // Re-encode on read so stale binary-format entries get
                        // transparently migrated to hex next time.
                        if hex_secret.len() != DEK_LEN * 2 {
                            let _ = entry.set_secret(hex::encode(&*key).as_bytes());
                        }
                        return Ok(key);
                    }
                    Err(_) => {
                        warn!(
                            "DEK stored with unexpected format (len={}), regenerating",
                            hex_secret.len()
                        );
                        let _ = entry.delete_credential();
                    }
                }
            }
            Err(keyring::Error::NoEntry) => {
                // expected first-run path
            }
            Err(e) => return Err(PebbleError::Auth(format!("Keyring read error: {e}"))),
        }

        info!("No usable DEK found, generating new one");
        let mut key = Zeroizing::new([0u8; DEK_LEN]);
        rand::thread_rng().fill_bytes(&mut *key);
        let hex_key = hex::encode(&*key);
        entry
            .set_secret(hex_key.as_bytes())
            .map_err(|e| PebbleError::Auth(format!("Failed to store DEK: {e}")))?;
        Ok(key)
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

/// Decode a 32-byte key from its hex representation.
fn decode_hex(hex_data: &[u8]) -> std::result::Result<Zeroizing<[u8; DEK_LEN]>, ()> {
    let hex_str =
        std::str::from_utf8(hex_data).map_err(|_| ())?;
    let bytes = hex::decode(hex_str).map_err(|_| ())?;
    if bytes.len() != DEK_LEN {
        return Err(());
    }
    let mut key = Zeroizing::new([0u8; DEK_LEN]);
    key.copy_from_slice(&bytes);
    Ok(key)
}
