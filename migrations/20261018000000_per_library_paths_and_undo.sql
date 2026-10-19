CREATE TABLE tracks_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL,

    title TEXT,
    artist TEXT,
    album TEXT,
    album_artist TEXT,
    album_artists TEXT,
    composer TEXT,
    label TEXT,
    genre TEXT,
    comment TEXT,
    lyrics TEXT,

    track INTEGER,
    track_total INTEGER,
    disc INTEGER,
    disc_total INTEGER,

    release_year INTEGER,
    recording_date TEXT,
    original_release_date TEXT,

    release_type TEXT,
    compilation INTEGER,
    isrc TEXT,
    barcode TEXT,
    catalog_number TEXT,
    bpm INTEGER,
    language TEXT,
    script TEXT,
    mood TEXT,

    replay_gain_track_gain TEXT,
    replay_gain_track_peak TEXT,
    replay_gain_album_gain TEXT,
    replay_gain_album_peak TEXT,

    file_format TEXT,
    file_size INTEGER,
    duration INTEGER,
    bitrate INTEGER,
    sample_rate INTEGER,
    bit_depth INTEGER,
    channels INTEGER,

    acoustid TEXT,
    musicbrainz_recording_id TEXT,
    musicbrainz_track_id TEXT,
    musicbrainz_release_id TEXT,
    musicbrainz_release_group_id TEXT,
    musicbrainz_artist_id TEXT,
    musicbrainz_release_artist_id TEXT,
    musicbrainz_work_id TEXT,

    status TEXT NOT NULL DEFAULT 'pending',

    added_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_scanned_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    enriched_at TIMESTAMP,

    file_hash TEXT,
    marked_for_deletion INTEGER NOT NULL DEFAULT 0,
    health_issue TEXT,
    library_id INTEGER REFERENCES libraries(id),

    UNIQUE (library_id, file_path)
);

INSERT INTO tracks_new (
    id, file_path, title, artist, album, album_artist, album_artists, composer,
    label, genre, comment, lyrics, track, track_total, disc, disc_total,
    release_year, recording_date, original_release_date, release_type,
    compilation, isrc, barcode, catalog_number, bpm, language, script, mood,
    replay_gain_track_gain, replay_gain_track_peak, replay_gain_album_gain,
    replay_gain_album_peak, file_format, file_size, duration, bitrate,
    sample_rate, bit_depth, channels, acoustid, musicbrainz_recording_id,
    musicbrainz_track_id, musicbrainz_release_id, musicbrainz_release_group_id,
    musicbrainz_artist_id, musicbrainz_release_artist_id, musicbrainz_work_id,
    status, added_at, last_scanned_at, enriched_at, file_hash,
    marked_for_deletion, health_issue, library_id
)
SELECT
    id, file_path, title, artist, album, album_artist, album_artists, composer,
    label, genre, comment, lyrics, track, track_total, disc, disc_total,
    release_year, recording_date, original_release_date, release_type,
    compilation, isrc, barcode, catalog_number, bpm, language, script, mood,
    replay_gain_track_gain, replay_gain_track_peak, replay_gain_album_gain,
    replay_gain_album_peak, file_format, file_size, duration, bitrate,
    sample_rate, bit_depth, channels, acoustid, musicbrainz_recording_id,
    musicbrainz_track_id, musicbrainz_release_id, musicbrainz_release_group_id,
    musicbrainz_artist_id, musicbrainz_release_artist_id, musicbrainz_work_id,
    status, added_at, last_scanned_at, enriched_at, file_hash,
    marked_for_deletion, health_issue, library_id
FROM tracks;

-- Carry the AUTOINCREMENT high-water mark over, so ids of purged rows are
-- never handed out again (undoing a purge restores a row under its old id).
INSERT INTO sqlite_sequence (name, seq)
SELECT 'tracks_new', seq FROM sqlite_sequence
WHERE name = 'tracks' AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'tracks_new');
UPDATE sqlite_sequence
SET seq = MAX(seq, COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'tracks'), 0))
WHERE name = 'tracks_new';

DROP TABLE tracks;
ALTER TABLE tracks_new RENAME TO tracks;

CREATE INDEX IF NOT EXISTS idx_tracks_file_hash ON tracks (file_hash);
CREATE INDEX IF NOT EXISTS idx_tracks_isrc ON tracks (isrc);
CREATE INDEX IF NOT EXISTS idx_tracks_status ON tracks (status);
CREATE INDEX IF NOT EXISTS idx_tracks_marked ON tracks (marked_for_deletion);
CREATE INDEX IF NOT EXISTS idx_tracks_health ON tracks (health_issue);
CREATE INDEX IF NOT EXISTS idx_tracks_library ON tracks (library_id);

CREATE TRIGGER IF NOT EXISTS tracks_fts_ai AFTER INSERT ON tracks BEGIN
    INSERT INTO tracks_fts(rowid, title, artist, album, album_artist, genre)
    VALUES (new.id, new.title, new.artist, new.album, new.album_artist, new.genre);
END;

CREATE TRIGGER IF NOT EXISTS tracks_fts_ad AFTER DELETE ON tracks BEGIN
    INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album, album_artist, genre)
    VALUES ('delete', old.id, old.title, old.artist, old.album, old.album_artist, old.genre);
END;

CREATE TRIGGER IF NOT EXISTS tracks_fts_au AFTER UPDATE ON tracks BEGIN
    INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album, album_artist, genre)
    VALUES ('delete', old.id, old.title, old.artist, old.album, old.album_artist, old.genre);
    INSERT INTO tracks_fts(rowid, title, artist, album, album_artist, genre)
    VALUES (new.id, new.title, new.artist, new.album, new.album_artist, new.genre);
END;

ALTER TABLE action_log ADD COLUMN library_id INTEGER REFERENCES libraries(id);
CREATE INDEX IF NOT EXISTS idx_action_log_library ON action_log (library_id, undone, id);
