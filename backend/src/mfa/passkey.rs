use std::collections::HashMap;
use std::time::{Duration, Instant};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use dashmap::DashMap;
use rand::RngCore;
use webauthn_rs::prelude::{
    AuthenticationResult, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, Webauthn, WebauthnBuilder,
};

use crate::config::AppConfig;

/// How long a pending ceremony nonce stays valid before we discard it.
const CEREMONY_TTL: Duration = Duration::from_secs(300);

/// In-flight registration ceremony state, keyed by client nonce.
pub struct PendingRegistration {
    pub state: PasskeyRegistration,
    /// 32-byte random salt sent to authenticator as PRF input.
    pub prf_salt: Vec<u8>,
    pub key_name: String,
    pub created_at: Instant,
}

/// In-flight authentication ceremony state, keyed by client nonce.
pub struct PendingAuthentication {
    /// None for anti-enumeration fake challenges - finish_authentication rejects these immediately.
    pub state: Option<PasskeyAuthentication>,
    pub email: String,
    /// Maps base64url credential ID -> prf_salt for this user's enrolled keys.
    pub cred_salts: HashMap<String, Vec<u8>>,
    pub created_at: Instant,
}

/// Central passkey service: holds the WebAuthn instance and pending ceremony maps.
///
/// `webauthn` is `None` when the operator has not configured RP_ID / RP_ORIGIN,
/// in which case all passkey routes return 503.
pub struct PasskeyService {
    pub webauthn: Option<Webauthn>,
    pub pending_reg: DashMap<String, PendingRegistration>,
    pub pending_auth: DashMap<String, PendingAuthentication>,
}

impl PasskeyService {
    pub fn from_config(config: &AppConfig) -> Result<Self, String> {
        let webauthn = match (&config.webauthn_rp_id, &config.webauthn_rp_origin) {
            (Some(rp_id), Some(rp_origin)) => {
                let origin = url::Url::parse(rp_origin)
                    .map_err(|e| format!("Invalid WEBAUTHN_RP_ORIGIN: {e}"))?;
                let wbn = WebauthnBuilder::new(rp_id, &origin)
                    .map_err(|e| format!("WebauthnBuilder error: {e}"))?
                    .rp_name("oxi Mail")
                    .build()
                    .map_err(|e| format!("Webauthn build error: {e}"))?;
                Some(wbn)
            }
            _ => None,
        };
        Ok(Self {
            webauthn,
            pending_reg: DashMap::new(),
            pending_auth: DashMap::new(),
        })
    }

    fn generate_nonce() -> String {
        let mut bytes = [0u8; 24];
        rand::rng().fill_bytes(&mut bytes);
        URL_SAFE_NO_PAD.encode(bytes)
    }

    pub fn generate_prf_salt() -> Vec<u8> {
        let mut bytes = vec![0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        bytes
    }

    /// Cleans out stale pending ceremonies older than CEREMONY_TTL.
    pub fn purge_stale(&self) {
        let cutoff = Instant::now() - CEREMONY_TTL;
        self.pending_reg.retain(|_, v| v.created_at > cutoff);
        self.pending_auth.retain(|_, v| v.created_at > cutoff);
    }

    /// Begin passkey registration ceremony.
    ///
    /// Returns `(nonce, creation_options_json)`. The caller injects PRF extension
    /// into the JSON before sending to the client.
    pub fn begin_registration(
        &self,
        user_id: uuid::Uuid,
        email: &str,
        key_name: String,
        exclude_cred_ids: Vec<String>,
    ) -> Result<(String, serde_json::Value), String> {
        let wbn = self.webauthn.as_ref().ok_or("Passkeys not configured")?;

        let exclude: Vec<webauthn_rs::prelude::CredentialID> = exclude_cred_ids
            .into_iter()
            .filter_map(|id| {
                URL_SAFE_NO_PAD
                    .decode(id.as_bytes())
                    .ok()
                    .map(webauthn_rs::prelude::CredentialID::from)
            })
            .collect();

        let (ccr, state) = wbn
            .start_passkey_registration(
                user_id,
                email,
                email,
                if exclude.is_empty() { None } else { Some(exclude) },
            )
            .map_err(|e| format!("start_passkey_registration: {e}"))?;

        let prf_salt = Self::generate_prf_salt();
        let prf_salt_b64 = URL_SAFE_NO_PAD.encode(&prf_salt);

        let mut options = serde_json::to_value(&ccr)
            .map_err(|e| format!("serialize creation options: {e}"))?;

        // Inject PRF extension so the browser requests a PRF output from the authenticator.
        options["publicKey"]["extensions"]["prf"] = serde_json::json!({
            "eval": {
                "first": prf_salt_b64
            }
        });

        let nonce = Self::generate_nonce();
        self.pending_reg.insert(nonce.clone(), PendingRegistration {
            state,
            prf_salt,
            key_name,
            created_at: Instant::now(),
        });

        Ok((nonce, options))
    }

    /// Finish passkey registration. Returns the completed `Passkey` and the `prf_salt`
    /// that was used. Caller must check that a PRF output was present in the response
    /// and encrypt the IMAP password before storing.
    pub fn finish_registration(
        &self,
        nonce: &str,
        credential_json: serde_json::Value,
    ) -> Result<(Passkey, Vec<u8>, String), String> {
        let pending = self
            .pending_reg
            .remove(nonce)
            .ok_or("Ceremony expired or nonce invalid")?
            .1;

        let wbn = self.webauthn.as_ref().ok_or("Passkeys not configured")?;

        let reg_cred: RegisterPublicKeyCredential =
            serde_json::from_value(credential_json)
                .map_err(|e| format!("Invalid credential JSON: {e}"))?;

        let passkey = wbn
            .finish_passkey_registration(&reg_cred, &pending.state)
            .map_err(|e| format!("finish_passkey_registration: {e}"))?;

        Ok((passkey, pending.prf_salt, pending.key_name))
    }

    /// Begin passkey authentication ceremony.
    ///
    /// `passkeys` is the list of enrolled Passkey structs deserialized from the DB.
    /// `cred_salts` maps credential_id (base64url) -> prf_salt for each enrolled key.
    /// Returns `(nonce, request_options_json)` with PRF salts injected per-credential.
    pub fn begin_authentication(
        &self,
        email: String,
        passkeys: &[Passkey],
        cred_salts: HashMap<String, Vec<u8>>,
    ) -> Result<(String, serde_json::Value), String> {
        let wbn = self.webauthn.as_ref().ok_or("Passkeys not configured")?;

        let (rcr, state) = wbn
            .start_passkey_authentication(passkeys)
            .map_err(|e| format!("start_passkey_authentication: {e}"))?;

        let mut options = serde_json::to_value(&rcr)
            .map_err(|e| format!("serialize request options: {e}"))?;

        // Inject per-credential PRF salts via evalByCredential.
        let by_cred: serde_json::Map<String, serde_json::Value> = cred_salts
            .iter()
            .map(|(cred_id, salt)| {
                (
                    cred_id.clone(),
                    serde_json::json!({ "first": URL_SAFE_NO_PAD.encode(salt) }),
                )
            })
            .collect();

        options["publicKey"]["extensions"]["prf"] = serde_json::json!({
            "evalByCredential": by_cred
        });

        let nonce = Self::generate_nonce();
        self.pending_auth.insert(nonce.clone(), PendingAuthentication {
            state: Some(state),
            email,
            cred_salts,
            created_at: Instant::now(),
        });

        Ok((nonce, options))
    }

    /// Begin a fake authentication ceremony for anti-enumeration purposes.
    ///
    /// Returns options with the same JSON shape as a real ceremony so callers
    /// cannot distinguish accounts with passkeys from those without. The stored
    /// state has `state: None`, so `finish_authentication` always rejects it.
    ///
    /// A synthetic credential ID and PRF salt are generated so that
    /// `allowCredentials` and `evalByCredential` are non-empty - identical in
    /// shape to a real response. Without this an attacker could trivially
    /// distinguish "no passkeys" from "has passkeys" by checking the field.
    pub fn begin_authentication_fake(
        &self,
        email: String,
        rp_id: &str,
    ) -> (String, serde_json::Value) {
        let mut challenge = [0u8; 32];
        rand::rng().fill_bytes(&mut challenge);

        let mut fake_cred_id = [0u8; 32];
        rand::rng().fill_bytes(&mut fake_cred_id);
        let fake_cred_id_b64 = URL_SAFE_NO_PAD.encode(fake_cred_id);

        let mut fake_salt = [0u8; 32];
        rand::rng().fill_bytes(&mut fake_salt);

        let options = serde_json::json!({
            "publicKey": {
                "challenge": URL_SAFE_NO_PAD.encode(challenge),
                "timeout": 60000,
                "rpId": rp_id,
                "allowCredentials": [{ "type": "public-key", "id": fake_cred_id_b64 }],
                "userVerification": "required",
                "extensions": {
                    "prf": {
                        "evalByCredential": {
                            fake_cred_id_b64: { "first": URL_SAFE_NO_PAD.encode(fake_salt) }
                        }
                    }
                }
            }
        });

        let nonce = Self::generate_nonce();
        self.pending_auth.insert(nonce.clone(), PendingAuthentication {
            state: None,
            email,
            cred_salts: HashMap::new(),
            created_at: Instant::now(),
        });

        (nonce, options)
    }

    /// Finish passkey authentication.
    ///
    /// Returns `(AuthenticationResult, prf_salt)` where `prf_salt` is the salt for
    /// the credential that was used, so the caller can derive the AES key.
    pub fn finish_authentication(
        &self,
        nonce: &str,
        credential_json: serde_json::Value,
    ) -> Result<(AuthenticationResult, Vec<u8>, String), String> {
        let pending = self
            .pending_auth
            .remove(nonce)
            .ok_or("Ceremony expired or nonce invalid")?
            .1;

        // Fake challenge: reject immediately without revealing why.
        let state = pending.state.ok_or("Authentication failed")?;

        let wbn = self.webauthn.as_ref().ok_or("Passkeys not configured")?;

        let pub_key_cred: PublicKeyCredential =
            serde_json::from_value(credential_json)
                .map_err(|e| format!("Invalid credential JSON: {e}"))?;

        let auth_result = wbn
            .finish_passkey_authentication(&pub_key_cred, &state)
            .map_err(|e| format!("finish_passkey_authentication: {e}"))?;

        let cred_id_b64 = URL_SAFE_NO_PAD.encode(auth_result.cred_id().as_ref());
        let salt = pending
            .cred_salts
            .get(&cred_id_b64)
            .cloned()
            .ok_or("No PRF salt found for this credential")?;

        Ok((auth_result, salt, pending.email))
    }
}

/// Encrypt `plaintext` using `prf_output` (32 bytes) as the AES-256-GCM key.
/// Returns `(ciphertext_with_tag, nonce_12_bytes)`.
pub fn encrypt_with_prf(prf_output: &[u8], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    if prf_output.len() != 32 {
        return Err(format!("PRF output must be 32 bytes, got {}", prf_output.len()));
    }
    let key = Key::<Aes256Gcm>::from_slice(prf_output);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("AES-GCM encrypt failed: {e}"))?;

    Ok((ct, nonce_bytes.to_vec()))
}

/// Decrypt `ciphertext` using `prf_output` (32 bytes) as the AES-256-GCM key.
pub fn decrypt_with_prf(prf_output: &[u8], ciphertext: &[u8], nonce_bytes: &[u8]) -> Result<Vec<u8>, String> {
    if prf_output.len() != 32 {
        return Err(format!("PRF output must be 32 bytes, got {}", prf_output.len()));
    }
    if nonce_bytes.len() != 12 {
        return Err("Nonce must be 12 bytes".to_string());
    }
    let key = Key::<Aes256Gcm>::from_slice(prf_output);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "AES-GCM decrypt failed (wrong key or corrupted data)".to_string())
}

/// Extract PRF output from a WebAuthn credential's clientExtensionResults.
/// Returns `None` if absent (PRF not supported by this authenticator/browser).
pub fn extract_prf_output(credential_json: &serde_json::Value) -> Option<Vec<u8>> {
    let b64 = credential_json
        .get("clientExtensionResults")?
        .get("prf")?
        .get("results")?
        .get("first")?
        .as_str()?;
    URL_SAFE_NO_PAD.decode(b64).ok()
}

/// Serialise a `Passkey` to a JSON string for DB storage.
pub fn serialize_passkey(pk: &Passkey) -> Result<String, String> {
    serde_json::to_string(pk).map_err(|e| format!("Failed to serialize passkey: {e}"))
}

/// Deserialise a `Passkey` from a stored JSON string.
pub fn deserialize_passkey(json: &str) -> Result<Passkey, String> {
    serde_json::from_str(json).map_err(|e| format!("Failed to deserialize passkey: {e}"))
}

/// Get the base64url credential ID from a `Passkey`.
pub fn passkey_cred_id(pk: &Passkey) -> String {
    URL_SAFE_NO_PAD.encode(pk.cred_id().as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prf_key() -> Vec<u8> {
        vec![0x42u8; 32]
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let plaintext = b"secret-imap-password";
        let (ct, nonce) = encrypt_with_prf(&prf_key(), plaintext).unwrap();
        let recovered = decrypt_with_prf(&prf_key(), &ct, &nonce).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn encrypt_wrong_key_length_returns_err() {
        let short_key = vec![0u8; 16];
        let result = encrypt_with_prf(&short_key, b"data");
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_wrong_key_returns_err() {
        let (ct, nonce) = encrypt_with_prf(&prf_key(), b"data").unwrap();
        let wrong_key = vec![0x99u8; 32];
        let result = decrypt_with_prf(&wrong_key, &ct, &nonce);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_bad_nonce_length_returns_err() {
        let (ct, _) = encrypt_with_prf(&prf_key(), b"data").unwrap();
        let result = decrypt_with_prf(&prf_key(), &ct, &[0u8; 8]);
        assert!(result.is_err());
    }

    #[test]
    fn extract_prf_output_present() {
        let raw = b"12345678901234567890123456789012";
        let b64 = URL_SAFE_NO_PAD.encode(raw);
        let json = serde_json::json!({
            "clientExtensionResults": {
                "prf": {
                    "results": {
                        "first": b64
                    }
                }
            }
        });
        let result = extract_prf_output(&json).unwrap();
        assert_eq!(result, raw);
    }

    #[test]
    fn extract_prf_output_absent_returns_none() {
        let json = serde_json::json!({ "clientExtensionResults": {} });
        assert!(extract_prf_output(&json).is_none());
    }

    #[test]
    fn extract_prf_output_missing_field_returns_none() {
        let json = serde_json::json!({});
        assert!(extract_prf_output(&json).is_none());
    }
}
