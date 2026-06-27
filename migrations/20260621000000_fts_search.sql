-- Full-text search index over the searchable tag columns. Replaces the old
-- `LIKE '%q%'` filters (which forced a full table scan on every keystroke) with
-- an FTS5 MATCH lookup that uses an inverted index.
--
-- This is an external-content table (content='tracks'): the FTS index stores
-- only the tokenized terms and points back at tracks.id, so we don't duplicate
-- the text. The trigger trio below keeps it in sync with the tracks table.

CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
    title,
    artist,
    album,
    album_artist,
    genre,
    content='tracks',
    content_rowid='id'
);

-- Backfill the index from existing rows.
INSERT INTO tracks_fts(rowid, title, artist, album, album_artist, genre)
SELECT id, title, artist, album, album_artist, genre FROM tracks;

-- Keep the index in sync. For an external-content table, deletes/updates are
-- signalled by inserting a special 'delete' command row carrying the OLD values.
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
