pub mod aes;
pub mod keystore;

use pebble_core::Result;
use zeroize::Zeroizing;

/// Service that manages encryption/decryption using a DEK from the OS keystore.
pub struct CryptoService {
    dek: Zeroizing<[u8; 32]>,
}

impl CryptoService {
    /// Initialize by loading (or creating) the DEK from the OS credential store.
    pub fn init() -> Result<Self> {
        let dek = keystore::KeyStore::get_or_create_dek()?;
        Ok(Self { dek })
    }

    /// Encrypt plaintext bytes.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        aes::encrypt(&*self.dek, plaintext)
    }

    /// Decrypt ciphertext bytes.
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        aes::decrypt(&*self.dek, ciphertext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
