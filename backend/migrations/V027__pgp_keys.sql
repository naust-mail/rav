CREATE TABLE IF NOT EXISTS pgp_keys (
    id              TEXT    PRIMARY KEY,
    identity_id     INTEGER REFERENCES identities(id) ON DELETE SET NULL,
    fingerprint     TEXT    NOT NULL UNIQUE,
    public_key      TEXT    NOT NULL,
    private_key_enc TEXT    NOT NULL,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS pgp_keys_identity_idx ON pgp_keys(identity_id);
