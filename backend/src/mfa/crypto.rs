use std::fs;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;

/// Wraps the server-side AES-256-GCM key used to encrypt TOTP secrets at rest.
///
/// One key per Rav instance, stored as 32 raw bytes in `{data_dir}/.mfa_key`
/// with mode 600. Generated on first use if the file does not exist.
/// Losing this file invalidates all enrolled TOTP secrets.
pub struct MfaCrypto {
    cipher: Aes256Gcm,
}

impl MfaCrypto {
    /// Load the key from `{data_dir}/.mfa_key`, creating it if absent.
    pub fn from_data_dir(data_dir: &str) -> Result<Self, String> {
        let key_path = Path::new(data_dir).join(".mfa_key");

        let key_bytes: Vec<u8> = if key_path.exists() {
            let bytes = fs::read(&key_path)
                .map_err(|e| format!("Failed to read MFA key file: {e}"))?;
            if bytes.len() != 32 {
                return Err(format!(
                    "MFA key file has wrong length (got {}, expected 32)",
                    bytes.len()
                ));
            }
            bytes
        } else {
            let mut bytes = vec![0u8; 32];
            rand::rng().fill_bytes(&mut bytes);

            fs::create_dir_all(data_dir)
                .map_err(|e| format!("Failed to create data dir: {e}"))?;
            fs::write(&key_path, &bytes)
                .map_err(|e| format!("Failed to write MFA key file: {e}"))?;

            // Set mode 600 on Unix.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))
                    .map_err(|e| format!("Failed to set MFA key file permissions: {e}"))?;
            }

            bytes
        };

        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        Ok(Self {
            cipher: Aes256Gcm::new(key),
        })
    }

    /// Encrypt `plaintext` with a fresh random 12-byte nonce.
    /// Returns `(ciphertext_with_tag, nonce)`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| format!("AES-GCM encrypt failed: {e}"))?;

        Ok((ciphertext, nonce_bytes.to_vec()))
    }

    /// Decrypt `ciphertext` (including auth tag) with the given `nonce`.
    pub fn decrypt(&self, ciphertext: &[u8], nonce_bytes: &[u8]) -> Result<Vec<u8>, String> {
        if nonce_bytes.len() != 12 {
            return Err("Invalid nonce length".to_string());
        }
        let nonce = Nonce::from_slice(nonce_bytes);

        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| "AES-GCM decrypt failed (wrong key or corrupted data)".to_string())
    }
}
