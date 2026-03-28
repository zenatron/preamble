# preamble

A music library scanner written in Rust. Walks a directory tree, reads tags from FLAC files, and persists them to a local SQLite database.

## What it does

Point it at a directory and it recursively collects every FLAC file, reads the audio tags and properties using `lofty`, then bulk-inserts everything into a SQLite database in a single transaction. On a library of ~640 files, the full scan completes in under 60ms.

```
cargo run -- /path/to/music
```

## Structure

```
src/
  main.rs       -- entry point, orchestrates scan and insert
  track.rs      -- TrackInfo struct, tag reading, Display impl
  reader.rs     -- recursive path collection
  db.rs         -- database init, migrations, insert
migrations/
  *.sql         -- schema
```

## What gets stored

| Field | Type |
|---|---|
| file_path | TEXT NOT NULL UNIQUE |
| artist, album, title, genre, comment | TEXT |
| track, track_total, disc, disc_total | INTEGER |
| release_year | INTEGER |
| duration, bitrate, sample_rate | INTEGER |
| bit_depth, channels | INTEGER |

## Dependencies

- `lofty` — tag reading across FLAC, MP3, Ogg, MP4
- `sqlx` — async SQLite with compile-time checked queries
- `tokio` — async runtime, thread pool for concurrent tag reading
- `futures` — `join_all` for collecting concurrent task results

## TODOs

### Immediate
- [ ] Support MP3, Ogg, and other formats alongside FLAC
- [ ] Incremental scan — skip files already in the database
- [ ] Detect and report files with missing or incomplete tags

### Pipeline
- [ ] AcoustID fingerprinting for files with missing tags
- [ ] MusicBrainz lookup against fingerprint results
- [ ] Confidence scoring — tag agreement vs fingerprint vs path heuristics
- [ ] Dead letter queue for files that fail matching
- [ ] Atomic file moves once a track is confirmed

### TUI
- [ ] Library browser with ratatui
- [ ] DLQ dashboard — review and resolve unmatched files
- [ ] Manual tag editing

### Infrastructure
- [ ] Config file for library path, supported formats, pipeline rules
- [ ] Structured logging with `tracing`
- [ ] Watch mode — monitor inbox directory for new files