-- Replace the old drafts table (content stored in SQLite) with a lightweight
-- staging table. IMAP is the source of truth for draft content; this table
-- only tracks local UI state that IMAP doesn't need to know about.
DROP TABLE IF EXISTS drafts;

CREATE TABLE draft_staging (
    uuid             TEXT    PRIMARY KEY,
    imap_uid         INTEGER,
    -- Message-ID of the message being replied to. Used to reconstruct the quoted
    -- content when the draft is reopened. Stored as Message-ID (not UID+folder)
    -- so the reference survives the original message being moved between folders.
    -- UNIQUE enforces one reply draft per original message.
    reply_message_id TEXT UNIQUE
);

-- Recreate draft_attachments with a FK to draft_staging instead of drafts.
DROP TABLE IF EXISTS draft_attachments;

CREATE TABLE draft_attachments (
    id           TEXT    PRIMARY KEY,
    draft_uuid   TEXT    NOT NULL REFERENCES draft_staging(uuid) ON DELETE CASCADE,
    filename     TEXT    NOT NULL,
    content_type TEXT    NOT NULL,
    size         INTEGER NOT NULL,
    file_path    TEXT    NOT NULL,
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
