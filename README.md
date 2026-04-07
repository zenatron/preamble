# preamble

A music library scanner and metadata tool written in Rust. Walks a directory tree, reads tags from FLAC files, persists them to a local SQLite database, and presents everything in a terminal UI.

No AI was used in the writing of the code. AI was partially used in the writing of this README.

## What it does

Launch with an optional path argument to point it at your music directory. From the start screen you can trigger a scan, a fresh rescan, or jump straight into the library browser.

The scan pipeline:
- Recursively collects all FLAC files not already in the database
- Reads audio tags and technical properties concurrently (32 threads) using `lofty`
- Hashes every file with BLAKE3
- Detects duplicates via file hash, ISRC, and file path — within the scan batch and against existing DB records
- Bulk-inserts everything into SQLite with live progress reporting

```
cargo run -- /path/to/music
```

The path argument is optional. If omitted, the TUI launches against the existing database.

## TUI

Three tabs navigable with `Tab`:

- **Library** — all tracks with title, artist, album, format, size, duration, bitrate, and status
- **Enrichment** — tracks with `status = 'pending'` awaiting metadata enrichment
- **Duplicates** — tracks flagged as duplicates via hash or ISRC matching

Start screen keybinds:

| Key | Action |
|-----|--------|
| `s` | Scan directory for new files |
| `r` | Fresh scan (wipes DB and rescans) |
| `Enter` | Open library |
| `q` | Quit |

Main screen keybinds:

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate tracks |
| `Tab` | Switch tabs |
| `Esc` | Return to start screen |
| `q` | Quit |

## Structure

```
src/
  main.rs       -- entry point, event loop, run_app
  app.rs        -- App struct, LibraryStats, screen/tab state
  ui.rs         -- draw functions, poll_events
  reader.rs     -- recursive path collection, scan_library pipeline, ScanEvent
  track.rs      -- TrackInfo and TrackSummary structs, tag reading, file hashing
  db.rs         -- database init, migrations, queries
migrations/
  *.sql         -- schema
```

## Deduplication layers

Duplicates are detected at three levels, applied in order:

1. **File path** — skips files already known to the database (weakest, catches re-scans)
2. **ISRC** — catches the same recording in different files or formats
3. **BLAKE3 hash** — catches exact byte-for-byte copies (strongest)

All three are also checked within the current scan batch, so duplicates introduced in a single scan run are caught before hitting the database.

## What gets stored

Full tag metadata including title, artist, album, album artist, composer, label, genre, ISRC, barcode, catalog number, BPM, replay gain, recording/release dates, MusicBrainz IDs, AcoustID, and technical properties (duration, bitrate, sample rate, bit depth, channels, file size, file format, BLAKE3 hash). Track status tracks pipeline state: `pending`, `duplicate`, `enriched`, `not_found`, `failed`, `manual_review`.

## Dependencies

- `lofty` — tag reading across FLAC, MP3, Ogg, MP4 and more
- `sqlx` — async SQLite with compile-time checked queries
- `tokio` — async runtime, thread pool for concurrent tag reading
- `futures` — `FuturesUnordered` for streaming concurrent task results
- `blake3` — fast file hashing
- `ratatui` + `crossterm` — terminal UI
- `humansize` — human-readable file sizes

## TODOs

### Immediate
- [ ] Support MP3, Ogg, and other formats alongside FLAC
- [ ] Config file for library path and supported formats
- [ ] Display `status_message` in the TUI

### Enrichment pipeline
- [ ] AcoustID fingerprinting for pending tracks
- [ ] MusicBrainz lookup against fingerprint results
- [ ] Enrichment worker triggered from the Enrichment tab
- [ ] Dead letter queue for tracks that fail matching

### TUI
- [ ] Track detail view on selection
- [ ] Manual tag editing
- [ ] Watch mode — monitor directory for new files
- [ ] Structured logging with `tracing`
