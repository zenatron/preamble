-- Staging flag for the Trash workflow: tracks flagged here are reviewed
-- before they are purged from disk + database. Independent of `status`, so a
-- track keeps its real pipeline state (duplicate, missing, ...) while flagged.
ALTER TABLE tracks ADD COLUMN marked_for_deletion INTEGER NOT NULL DEFAULT 0;

-- Speeds up duplicate grouping, which scans for repeated hashes / ISRCs.
CREATE INDEX IF NOT EXISTS idx_tracks_file_hash ON tracks (file_hash);
CREATE INDEX IF NOT EXISTS idx_tracks_isrc ON tracks (isrc);

-- Speeds up the per-tab status filters and the Trash query.
CREATE INDEX IF NOT EXISTS idx_tracks_status ON tracks (status);
CREATE INDEX IF NOT EXISTS idx_tracks_marked ON tracks (marked_for_deletion);
