use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;

use crate::error::AppError;

/// Encrypts and decrypts IMAP folder names using AES-256-GCM with a
/// session-derived key. IDs are opaque, tamper-proof, and expire with
/// the session.
pub struct FolderCipher {
    cipher: Aes256Gcm,
}

impl FolderCipher {
    pub fn new(key: &[u8; 32]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(key);
        Self { cipher: Aes256Gcm::new(key) }
    }

    /// Derive a cipher directly from a session token. Same derivation as in
    /// `auth::session`. Useful in tests where only the token is available.
    #[cfg(test)]
    pub fn from_session_token(token: &str) -> Self {
        use hmac::{KeyInit, Mac, SimpleHmac};
        use sha2::Sha256;
        let mut mac = SimpleHmac::<Sha256>::new_from_slice(token.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(b"folder-ids");
        let result = mac.finalize().into_bytes();
        let mut key = [0u8; 32];
        key.copy_from_slice(&result);
        Self::new(&key)
    }

    /// Encrypts a folder name. Returns `base64url(nonce || ciphertext+tag)`.
    /// A fresh 12-byte nonce is generated per call so the same folder name
    /// produces a different ID each session.
    pub fn encrypt(&self, folder_name: &str) -> String {
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self.cipher
            .encrypt(nonce, folder_name.as_bytes())
            .expect("AES-GCM encryption cannot fail");
        let mut buf = Vec::with_capacity(12 + ciphertext.len());
        buf.extend_from_slice(&nonce_bytes);
        buf.extend_from_slice(&ciphertext);
        URL_SAFE_NO_PAD.encode(&buf)
    }

    /// Decrypts a folder ID back to the IMAP folder name.
    /// Returns `BadRequest` if the ID is malformed or the GCM tag fails
    /// (invalid, tampered, or from a different session).
    pub fn decrypt(&self, folder_id: &str) -> Result<String, AppError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(folder_id)
            .map_err(|_| AppError::BadRequest("Invalid folder ID".to_string()))?;
        // nonce(12) + ciphertext(>=1) + tag(16) = minimum 29 bytes
        if bytes.len() < 29 {
            return Err(AppError::BadRequest("Invalid folder ID".to_string()));
        }
        let (nonce_bytes, ciphertext) = bytes.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| AppError::BadRequest("Invalid folder ID".to_string()))?;
        String::from_utf8(plaintext)
            .map_err(|_| AppError::BadRequest("Invalid folder ID".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = [42u8; 32];
        let cipher = FolderCipher::new(&key);
        let id = cipher.encrypt("INBOX");
        assert_eq!(cipher.decrypt(&id).unwrap(), "INBOX");
    }

    #[test]
    fn different_nonce_each_call() {
        let key = [7u8; 32];
        let cipher = FolderCipher::new(&key);
        let a = cipher.encrypt("Sent");
        let b = cipher.encrypt("Sent");
        assert_ne!(a, b);
        assert_eq!(cipher.decrypt(&a).unwrap(), "Sent");
        assert_eq!(cipher.decrypt(&b).unwrap(), "Sent");
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let key = [1u8; 32];
        let cipher = FolderCipher::new(&key);
        let mut bytes = URL_SAFE_NO_PAD.decode(cipher.encrypt("INBOX")).unwrap();
        bytes[20] ^= 0xFF; // flip bits in ciphertext
        let bad = URL_SAFE_NO_PAD.encode(&bytes);
        assert!(cipher.decrypt(&bad).is_err());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let cipher_a = FolderCipher::new(&[1u8; 32]);
        let cipher_b = FolderCipher::new(&[2u8; 32]);
        let id = cipher_a.encrypt("INBOX");
        assert!(cipher_b.decrypt(&id).is_err());
    }

    #[test]
    fn too_short_input_is_rejected() {
        let cipher = FolderCipher::new(&[0u8; 32]);
        assert!(cipher.decrypt("short").is_err());
        assert!(cipher.decrypt("").is_err());
    }

    #[test]
    fn from_session_token_matches_new_with_derived_key() {
        let token = "test-session-token-abc123";
        let cipher_a = FolderCipher::from_session_token(token);
        let id = cipher_a.encrypt("Drafts");
        // Same token -> same key -> decrypts correctly
        let cipher_b = FolderCipher::from_session_token(token);
        assert_eq!(cipher_b.decrypt(&id).unwrap(), "Drafts");
    }

    #[test]
    fn different_tokens_produce_different_keys() {
        let cipher_a = FolderCipher::from_session_token("token-one");
        let cipher_b = FolderCipher::from_session_token("token-two");
        let id = cipher_a.encrypt("INBOX");
        assert!(cipher_b.decrypt(&id).is_err());
    }
}
