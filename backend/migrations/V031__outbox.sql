-- Server-side send queue. A message written here is not sent immediately;
-- a background worker releases it after send_after (the undo window) and
-- performs the actual SMTP send + Sent-folder APPEND + draft cleanup.
CREATE TABLE outbox (
    id               TEXT    PRIMARY KEY,
    -- Draft this send originated from, if any. Cleaned up (IMAP expunge +
    -- staging delete + attachment files) only once the send actually
    -- succeeds, not at enqueue time, so undo can still reopen the draft.
    draft_id         TEXT,
    to_addrs         TEXT    NOT NULL,
    cc_addrs         TEXT    NOT NULL DEFAULT '[]',
    bcc_addrs        TEXT    NOT NULL DEFAULT '[]',
    subject          TEXT    NOT NULL DEFAULT '',
    text_body        TEXT    NOT NULL DEFAULT '',
    html_body        TEXT,
    in_reply_to      TEXT,
    references_hdr   TEXT,
    from_identity_id INTEGER,
    -- JSON-encoded PGP send params (mode/signature/ciphertext/micalg), if any.
    pgp_json         TEXT,
    state            TEXT    NOT NULL DEFAULT 'scheduled',
    fail_reason      TEXT,
    attempt_count    INTEGER NOT NULL DEFAULT 0,
    created_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    -- Undo deadline. The worker won't attempt a send before this time.
    send_after       TEXT    NOT NULL
);

CREATE INDEX outbox_state_idx ON outbox(state);
CREATE INDEX outbox_send_after_idx ON outbox(send_after);
