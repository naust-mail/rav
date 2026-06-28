-- Add thread_id column for grouping related messages in the list view.
-- thread_id is the canonical root message-id of the thread: the first entry
-- in References (oldest ancestor), falling back to In-Reply-To, then own Message-ID.
ALTER TABLE messages ADD COLUMN thread_id TEXT;

-- Backfill existing rows. SUBSTR/INSTR extracts the first space-delimited token
-- from references_header (the oldest ancestor per RFC 2822).
UPDATE messages SET thread_id = CASE
  WHEN references_header IS NOT NULL AND TRIM(references_header) != ''
    THEN TRIM(SUBSTR(
      TRIM(references_header),
      1,
      CASE
        WHEN INSTR(TRIM(references_header), ' ') > 0
        THEN INSTR(TRIM(references_header), ' ') - 1
        ELSE LENGTH(TRIM(references_header))
      END
    ))
  WHEN in_reply_to IS NOT NULL AND TRIM(in_reply_to) != ''
    THEN TRIM(in_reply_to)
  WHEN message_id IS NOT NULL AND TRIM(message_id) != ''
    THEN TRIM(message_id)
  ELSE NULL
END;

CREATE INDEX IF NOT EXISTS idx_messages_thread_id ON messages(thread_id);
