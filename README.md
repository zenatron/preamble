# preamble

A terminal music-library manager written in Rust. It scans a directory tree, reads tags from audio files, stores everything in a local SQLite database, and presents a fast keyboard- and mouse-driven TUI for browsing, de-duplicating, enriching, editing, and curating a music collection.

> All code in this repository was written by hand. AI was used as a learning aid and reference during development, not to generate code. This README was partially drafted with AI assistance.

## Highlights

- **Scanning** — concurrent tag reading + BLAKE3 hashing across many audio formats, with live progress and three-layer duplicate detection.
- **Browsing** — sortable columns, faceted filters, debounced search, album/artist grouping, mouse support, and scrollbars.
- **Duplicates** — hash- and ISRC-based groups with a side-by-side comparison and one-key keeper resolution.
- **Trash** — non-destructive, staged deletion that quarantines files and is fully undoable.
- **Enrichment** — AcoustID fingerprinting + MusicBrainz metadata, with a dead-letter/retry view.
- **Tag editing** — edit tags in place; changes are written to the file and the database.
- **Watch mode** — auto-import new files as they appear.
- **Integrity** — health checks (zero-byte, decode errors, hash mismatch, low bitrate) and incremental rescans.
- **Reporting** — a statistics screen plus CSV/JSON/M3U/duplicate-report exports.

```
cargo run -- /path/to/music
```

The path is optional: if omitted, the configured `library_path` is used, and failing that the TUI opens against the existing database.

## Configuration

On first run preamble writes `preamble.toml` with defaults:

```toml
# library_path          : default directory scanned when no CLI path is given
# formats               : audio file extensions to scan (lowercase, no dot)
# scan_concurrency      : files hashed/read in parallel during a scan
# acoustid_api_key      : optional; prefer the ACOUSTID_API_KEY env var / .env
# log_level             : error | warn | info | debug | trace (-> preamble.log)
# watch                 : auto-scan new files in the background on launch
# quarantine_dir        : purged files are moved here instead of deleted
# low_bitrate_threshold : kbps below which the health check flags a track

formats = ["flac", "mp3", "m4a", "mp4", "ogg", "opus", "aiff", "wav", "wv", "ape", "mpc", "aac"]
scan_concurrency = 8
log_level = "info"
watch = false
quarantine_dir = "quarantine"
low_bitrate_threshold = 128
```

A path given on the command line overrides `library_path`. The AcoustID key is read from the `ACOUSTID_API_KEY` environment variable (or a gitignored `.env`) so it never has to live in the committed config. Logs are written to `preamble.log` (`RUST_LOG` overrides `log_level`).

## TUI

Six tabs, navigable with `Tab` (or by clicking):

- **Library** — all tracks; supports grouped browse by artist/album
- **Enrichment** — `pending` tracks awaiting metadata
- **Duplicates** — hash/ISRC duplicate groups with keeper resolution
- **Failed** — enrichment dead letters (`failed` / `not_found`) with retry
- **Missing** — tracks whose files no longer exist
- **Trash** — tracks flagged for deletion, staged for review

### Start screen

| Key | Action |
|-----|--------|
| `s` | Scan for new files |
| `u` | Rescan changed files (incremental, by mtime) |
| `r` | Fresh scan (wipes DB and rescans) |
| `v` | Validate paths (flag missing files) |
| `h` | Health check (integrity scan) |
| `Enter` | Open library |
| `q` | Quit |

Long operations show a progress gauge and can be cancelled with `Esc`.

### Main screen

| Key | Action |
|-----|--------|
| `↑`/`↓` | Navigate rows / groups / duplicate members |
| `←`/`→` | Move between duplicate members |
| `Tab` | Switch tabs |
| `/` | Search (debounced; title/artist/album/album-artist/genre) |
| `f` | Filter facets (format, min bitrate, missing ISRC, unhealthy) |
| `s` / `S` | Cycle sort field / flip direction |
| `g` | Group Library by artist / album (Enter drills in) |
| `Space` | Toggle row selection |
| `i` / `o` / `u` | Select all / none / invert |
| `d` | Flag selected → Trash (in Trash: purge to quarantine) |
| `r` | Trash: restore · Failed: retry enrichment |
| `k` | Duplicates: keep highlighted member, flag the rest |
| `e` | Enrichment: run the pipeline |
| `m` | Edit the highlighted track's tags |
| `p` | Properties panel (Duplicates: switch pane) |
| `PgUp`/`PgDn` | Scroll the properties panel |
| `x` | Export the current view |
| `c` | Statistics screen |
| `w` | Toggle watch mode |
| `z` | Undo the last action |
| `?` | Keybinding help overlay |
| `Esc` | Close panel / clear search-filter / back |
| `q` | Quit |

Mouse: wheel scrolls, clicking a row selects it, clicking the tab bar advances tabs.

## Browsing

The Library, Enrichment, Failed, Missing, and Trash tabs are sortable (`s`/`S`) and filterable (`f`) independently, each remembering its own search query. Search is debounced so fast typing doesn't thrash the database. Grouped browse (`g`) aggregates the Library by artist or album with track counts, sizes, and runtimes; pressing `Enter` on a group filters the Library down to it.

## Tag editing

Press `m` on a track to open the editor (Title, Artist, Album, Album Artist, Genre, Comment, Track/Disc numbers, Year). `↑`/`↓` move between fields, `Enter` saves. Saving writes the tags into the audio file via `lofty` **and** updates the database, and marks the track `manual_review`.

## Deletion: Trash, quarantine, and undo

Deletion is never immediate. Pressing `d` **flags** the selected (or highlighted) tracks for deletion — a reversible flag independent of pipeline status. Flagged tracks gather in the **Trash** tab, where you can `r`estore them or `d` purge. Purging **moves** files into the `quarantine_dir` (rather than deleting them) and removes their rows. Every flag, restore, keeper-resolution, retry, and purge is recorded in a reversible action log: press `z` to undo, which also moves quarantined files back and re-inserts their rows.

## Duplicates

The Duplicates tab lists duplicate **groups** on the left and a transposed tag-by-tag **member comparison** on the right (one column per file). Press `p` to focus the members pane, then move between member columns with `←`/`→`. A suggested keeper is pre-highlighted (`★`) — highest bitrate, then largest file, then most complete tags — which you can override. Press `k` to keep the highlighted member; the rest of the group is flagged into Trash and the resolved group disappears.

Two kinds of groups are surfaced: **hash** groups (byte-identical files) and **ISRC** groups (the same recording across non-identical files, e.g. a FLAC and an MP3). An ISRC group is only shown when it spans more than one distinct file, so it never simply restates a hash group.

## Enrichment

Pending tracks can be enriched against [AcoustID](https://acoustid.org) and MusicBrainz (`e` on the Enrichment tab):

```
audio file ──fpcalc──▶ fingerprint + duration ──AcoustID──▶ MusicBrainz metadata ──▶ written back
```

For each pending track the worker runs `fpcalc` (Chromaprint), looks the fingerprint up against AcoustID (whose `recordings` metadata comes from MusicBrainz), and fills any missing title/artist/album plus the AcoustID and MusicBrainz IDs. Existing tags are never overwritten. Each track transitions to `enriched`, `not_found`, or `failed`; the last two land in the **Failed** tab, where `r` re-queues them. Requests are rate-limited and sent with an identifying User-Agent, and the run can be cancelled with `Esc`.

Requirements: [`fpcalc`](https://acoustid.org/chromaprint) on your `PATH`, and an AcoustID key in `ACOUSTID_API_KEY`.

## Watch mode

With watch mode on (`w`, or `watch = true` in config) a filesystem watcher monitors the library directory and, once it settles, runs a background import of any new files — no screen switch, just a status update and a `● WATCH` indicator in the tab bar.

## Integrity & maintenance

- **Health check** (`h` on the start screen) scans every file for problems — `missing_file`, `zero_byte`, `decode_error`, `hash_mismatch`, `low_bitrate` — and records them in `health_issue`, surfaced in the status column and filterable via the "unhealthy" facet.
- **Incremental rescan** (`u`) re-reads tags only for files whose modification time is newer than their last scan, keeping the DB in sync with on-disk edits.

## Statistics & export

The statistics screen (`c`) shows totals (tracks, size, runtime, average bitrate, lossless ratio) and breakdowns by format, decade, top artists, status, and health. The export picker (`x`) writes the current view to **CSV**, **JSON**, or an **M3U** playlist, or produces a **duplicate report** CSV — each to a timestamped file in the working directory.

## Deduplication layers

Duplicates are detected at three levels during a scan, applied in order:

1. **File path** — skips files already in the database (catches re-scans)
2. **ISRC** — catches the same recording in different files/formats
3. **BLAKE3 hash** — catches exact byte-for-byte copies

All three are also checked within the current scan batch.

## What gets stored

Full tag metadata (title, artist, album, album artist, composer, label, genre, ISRC, barcode, catalog number, BPM, replay gain, dates, MusicBrainz IDs, AcoustID) and technical properties (duration, bitrate, sample rate, bit depth, channels, size, format, BLAKE3 hash). Pipeline `status` is one of `pending`, `duplicate`, `enriched`, `not_found`, `failed`, `missing`, `manual_review`. Separate columns track the `marked_for_deletion` flag and any `health_issue`; an `action_log` table backs undo.

## Dependencies

- `lofty` — tag reading/writing across many formats
- `sqlx` — async SQLite with compile-time checked queries
- `tokio` / `futures` — async runtime and concurrent task streaming
- `blake3` — fast file hashing
- `reqwest` + `serde_json` — AcoustID/MusicBrainz lookups
- `notify` — filesystem watching
- `csv` — exports
- `serde` + `toml` — configuration and serialization
- `tracing` (+ subscriber/appender) — structured file logging
- `ratatui` + `crossterm` — terminal UI
- `humansize` — human-readable file sizes

## Structure

```
src/
  main.rs       -- entry point, event loop, run_app
  config.rs     -- preamble.toml + .env loading, logging init
  app.rs        -- App state, tabs, sort/filter/group/edit/watch/undo state
  ui.rs         -- draw functions, event + mouse handling, popups
  reader.rs     -- scan, watch, health check, incremental rescan
  enrich.rs     -- fpcalc + AcoustID/MusicBrainz pipeline
  export.rs     -- CSV / JSON / M3U / duplicate-report exporters
  undo.rs       -- reversible action model
  track.rs      -- TrackInfo / TrackSummary, tag reading + writing, hashing
  db.rs         -- database init, migrations, queries, stats
migrations/
  *.sql         -- schema
```

## TODOs

- [x] Multi-format scanning, config file, status messages
- [x] AcoustID + MusicBrainz enrichment, dead-letter/retry
- [x] Duplicate resolution, Trash + quarantine + undo
- [x] Manual tag editing
- [x] Watch mode, incremental rescan, health checks
- [x] Sorting, filtering, grouping, search, mouse, help overlay
- [x] Statistics screen and exports
- [ ] Direct MusicBrainz release/label/date lookup for richer metadata
- [ ] Cover art fetching
- [ ] Remappable keybindings

## Contributing

This is a personal learning project — contributions aren't expected, but feedback, bug reports, and suggestions are welcome via issues. If you do open a PR, please keep it focused and avoid AI-generated code.
