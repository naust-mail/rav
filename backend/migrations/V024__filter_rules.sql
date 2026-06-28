-- Email filter rules evaluated on every new incoming message.
-- conditions: JSON array of {field, op, value} objects (AND logic).
-- action_type: "move" | "tag" | "mark_read" | "delete"
-- action_value: target folder (move) or tag id (tag); null for mark_read/delete.
CREATE TABLE IF NOT EXISTS filter_rules (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    priority     INTEGER NOT NULL DEFAULT 0,
    conditions   TEXT NOT NULL DEFAULT '[]',
    action_type  TEXT NOT NULL,
    action_value TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_filter_rules_priority ON filter_rules(enabled, priority ASC);
