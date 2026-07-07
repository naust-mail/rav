-- Passkey credentials (WebAuthn, one per enrolled authenticator).
-- passkey_json is the full serialized webauthn-rs Passkey struct (sign count included).
-- prf_salt is a per-credential random 32-byte value used to request PRF during auth.
-- encrypted_imap / imap_nonce hold the AES-256-GCM ciphertext of the user's IMAP password,
-- encrypted with the PRF output as the AES key at enrollment time.
-- imap_host/port/tls/smtp_* are captured from the session at enrollment so passkey login
-- works even in custom-server mode.
CREATE TABLE IF NOT EXISTS mfa_passkeys (
    credential_id    TEXT    PRIMARY KEY,
    passkey_json     TEXT    NOT NULL,
    prf_salt         BLOB    NOT NULL,
    encrypted_imap   BLOB    NOT NULL,
    imap_nonce       BLOB    NOT NULL,
    name             TEXT    NOT NULL DEFAULT '',
    imap_host        TEXT    NOT NULL DEFAULT '',
    imap_port        INTEGER NOT NULL DEFAULT 993,
    imap_tls         INTEGER NOT NULL DEFAULT 1,
    smtp_host        TEXT    NOT NULL DEFAULT '',
    smtp_port        INTEGER NOT NULL DEFAULT 587,
    smtp_tls         INTEGER NOT NULL DEFAULT 1,
    created_at       INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Per-user MFA settings.
CREATE TABLE IF NOT EXISTS mfa_settings (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    passkey_only  INTEGER NOT NULL DEFAULT 0
);

INSERT OR IGNORE INTO mfa_settings (id, passkey_only) VALUES (1, 0);
