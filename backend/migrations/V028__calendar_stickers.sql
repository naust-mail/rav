CREATE TABLE calendar_stickers (
    date TEXT NOT NULL PRIMARY KEY, -- ISO date: YYYY-MM-DD
    sticker_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
