-- Vacation / out-of-office auto-responder settings (singleton row per user DB).
CREATE TABLE IF NOT EXISTS vacation_responder (
    id                   INTEGER PRIMARY KEY CHECK (id = 1),
    enabled              INTEGER NOT NULL DEFAULT 0,
    subject              TEXT NOT NULL DEFAULT 'Out of office',
    body                 TEXT NOT NULL DEFAULT '',
    start_date           TEXT,
    end_date             TEXT,
    reply_interval_hours INTEGER NOT NULL DEFAULT 24,
    updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Tracks which senders have already received a vacation reply, to avoid
-- sending duplicates within reply_interval_hours.
CREATE TABLE IF NOT EXISTS vacation_replies (
    sender_email TEXT NOT NULL PRIMARY KEY,
    replied_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
