-- Multi-library support.
--
-- Each library is a named, path-pinned collection. All libraries share this one
-- database and are distinguished by `tracks.library_id`. The column is added
-- nullable (purely additive, no table rebuild) and backfilled in Rust at
-- startup via `db::ensure_default_library`, which can pick a sensible default
-- path/name from the running config. Going forward every insert sets it.

CREATE TABLE IF NOT EXISTS libraries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL UNIQUE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_opened_at TIMESTAMP
);

ALTER TABLE tracks ADD COLUMN library_id INTEGER REFERENCES libraries(id);

CREATE INDEX IF NOT EXISTS idx_tracks_library ON tracks(library_id);
