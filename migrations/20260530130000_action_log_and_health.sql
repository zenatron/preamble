-- Reversible action history for the undo stack. Each row records one mutating
-- action and enough JSON payload to reverse it (e.g. previous tag values, or a
-- full serialized track + quarantine path for a purge).
CREATE TABLE IF NOT EXISTS action_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    kind TEXT NOT NULL,          -- flag | unflag | keep | purge | edit | status | retry
    summary TEXT NOT NULL,       -- human-readable description for the history view
    payload TEXT NOT NULL,       -- JSON describing how to undo
    undone INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_action_log_undone ON action_log (undone, id);

-- Integrity/health problems found by the health-check scan. NULL = healthy or
-- not yet checked. Values: zero_byte | decode_error | hash_mismatch | low_bitrate | missing_file
ALTER TABLE tracks ADD COLUMN health_issue TEXT;

CREATE INDEX IF NOT EXISTS idx_tracks_health ON tracks (health_issue);
