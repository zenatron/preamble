-- Tracks duplicate groups that the user has manually dismissed (keep all).
-- Duplicates are computed views (GROUP BY hash/ISRC) so we can't mark the
-- group itself; instead we record their key+kind so the query filters them.
CREATE TABLE IF NOT EXISTS duplicates_skipped (
    key    TEXT NOT NULL,
    kind   TEXT NOT NULL CHECK (kind IN ('hash', 'isrc')),
    library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    skipped_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (key, kind, library_id)
);
