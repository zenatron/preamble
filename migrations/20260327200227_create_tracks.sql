CREATE TABLE IF NOT EXISTS tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL UNIQUE,

    -- Core tags
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

    -- Numbering
    track INTEGER,
    track_total INTEGER,
    disc INTEGER,
    disc_total INTEGER,

    -- Dates
    release_year INTEGER,
    recording_date TEXT,
    original_release_date TEXT,

    -- Release metadata
    release_type TEXT,
    compilation INTEGER,
    isrc TEXT,
    barcode TEXT,
    catalog_number TEXT,
    bpm INTEGER,
    language TEXT,
    script TEXT,
    mood TEXT,

    -- Replay gain
    replay_gain_track_gain TEXT,
    replay_gain_track_peak TEXT,
    replay_gain_album_gain TEXT,
    replay_gain_album_peak TEXT,

    -- Technical properties
    file_format TEXT,
    file_size INTEGER,
    duration INTEGER,
    bitrate INTEGER,
    sample_rate INTEGER,
    bit_depth INTEGER,
    channels INTEGER,

    -- External IDs
    acoustid TEXT,
    musicbrainz_recording_id TEXT,
    musicbrainz_track_id TEXT,
    musicbrainz_release_id TEXT,
    musicbrainz_release_group_id TEXT,
    musicbrainz_artist_id TEXT,
    musicbrainz_release_artist_id TEXT,
    musicbrainz_work_id TEXT,

    -- Pipeline state
    status TEXT NOT NULL DEFAULT 'pending',

    -- Timestamps
    added_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_scanned_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    enriched_at TIMESTAMP,

    -- Blake ID
    file_hash TEXT
);