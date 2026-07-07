-- TOTP credential (one per user, AES-GCM encrypted secret)
CREATE TABLE IF NOT EXISTS mfa_totp (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    encrypted_secret BLOB    NOT NULL,
    nonce            BLOB    NOT NULL,
    created_at       INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Consumed TOTP time steps for replay prevention.
-- Step numbers older than 5 minutes are pruned on each verify call.
CREATE TABLE IF NOT EXISTS mfa_totp_used_steps (
    step    INTEGER NOT NULL PRIMARY KEY,
    used_at INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Single-row lockout state per user.
-- Locked after 5 consecutive failures; clears on success or after 15 minutes.
CREATE TABLE IF NOT EXISTS mfa_lockout (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    failed_count INTEGER NOT NULL DEFAULT 0,
    locked_until INTEGER,
    last_failure INTEGER
);

INSERT OR IGNORE INTO mfa_lockout (id, failed_count) VALUES (1, 0);
