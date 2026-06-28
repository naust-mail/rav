-- Undo-send delay in seconds. 0 = disabled (send immediately).
ALTER TABLE display_preferences ADD COLUMN undo_send_delay INTEGER NOT NULL DEFAULT 5;
