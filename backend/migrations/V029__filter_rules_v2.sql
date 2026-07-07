-- Extend filter_rules with per-rule match mode, stop-processing flag, and
-- a multi-action JSON column. Existing rows are migrated in-place.
ALTER TABLE filter_rules ADD COLUMN match_mode TEXT NOT NULL DEFAULT 'all';
ALTER TABLE filter_rules ADD COLUMN stop_processing INTEGER NOT NULL DEFAULT 0;
ALTER TABLE filter_rules ADD COLUMN actions TEXT NOT NULL DEFAULT '[]';

-- Collapse the legacy action_type / action_value columns into the new JSON array.
UPDATE filter_rules
SET actions = json_array(
    json_object('action_type', action_type, 'action_value', action_value)
)
WHERE actions = '[]';
