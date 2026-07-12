// src/db.rs
// Handles database queries and functionality

use crate::track::{DuplicateGroupSummary, DuplicateKind, TrackInfo, TrackSummary};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::time::Duration;
use std::{collections::HashSet, path::PathBuf};

/// initializes the Sqlite Pool
///
/// WAL journaling lets the UI keep reading while a scan/watch import holds a
/// write transaction (the default rollback journal would block readers).
/// `synchronous = NORMAL` is the safe pairing for WAL, and a busy timeout lets
/// the pool wait out a momentary writer lock instead of erroring with
/// "database is locked".
pub async fn init_db() -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename("library.db")
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new().connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

// Library registry
//
// Every track belongs to exactly one library (`tracks.library_id`). A library is
// a named, path-pinned collection; all libraries share this one database and all
// per-track queries scope to the active library's id.

/// A named, path-pinned music collection.
#[derive(Clone, Debug)]
pub struct Library {
    pub id: i64,
    pub name: String,
    pub path: String,
}

/// Lists known libraries, most-recently-opened first (NULLs last), then by name.
pub async fn list_libraries(pool: &SqlitePool) -> Result<Vec<Library>, sqlx::Error> {
    Ok(sqlx::query!(
        r#"SELECT id AS "id!", name, path FROM libraries
        ORDER BY last_opened_at DESC, name"#
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| Library {
        id: r.id,
        name: r.name,
        path: r.path,
    })
    .collect())
}

/// Looks up a library by its filesystem root path.
pub async fn find_library_by_path(
    pool: &SqlitePool,
    path: &str,
) -> Result<Option<Library>, sqlx::Error> {
    Ok(sqlx::query!(
        r#"SELECT id AS "id!", name, path FROM libraries WHERE path = ?"#,
        path
    )
    .fetch_optional(pool)
    .await?
    .map(|r| Library {
        id: r.id,
        name: r.name,
        path: r.path,
    }))
}

/// Creates a new library row and returns it (with its assigned id).
pub async fn create_library(
    pool: &SqlitePool,
    name: &str,
    path: &str,
) -> Result<Library, sqlx::Error> {
    let id = sqlx::query!(
        r#"INSERT INTO libraries (name, path) VALUES (?, ?)"#,
        name,
        path
    )
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(Library {
        id,
        name: name.to_string(),
        path: path.to_string(),
    })
}

/// Records that a library was just opened (drives the recency ordering).
pub async fn touch_library(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE libraries SET last_opened_at = CURRENT_TIMESTAMP WHERE id = ?"#,
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// One-time backfill for databases that predate multi-library support: assigns
/// every orphaned track (`library_id IS NULL`) to a default library. The default
/// is named/pathed from the running config when available so the migrated data
/// keeps a sensible home. No-op once every track has a library.
pub async fn ensure_default_library(
    pool: &SqlitePool,
    config_path: Option<&std::path::Path>,
) -> Result<(), sqlx::Error> {
    let orphans: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) FROM tracks WHERE library_id IS NULL"#)
        .fetch_one(pool)
        .await?;
    if orphans == 0 {
        return Ok(());
    }

    let (name, path) = match config_path {
        Some(p) => {
            let path = p.to_string_lossy().into_owned();
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Default Library".to_string());
            (name, path)
        }
        None => ("Default Library".to_string(), String::new()),
    };

    let lib = match find_library_by_path(pool, &path).await? {
        Some(l) => l,
        // Fall back to a generic name if the derived one collides (UNIQUE name).
        None => match create_library(pool, &name, &path).await {
            Ok(l) => l,
            Err(_) => create_library(pool, "Default Library", &path).await?,
        },
    };

    sqlx::query!(
        r#"UPDATE tracks SET library_id = ? WHERE library_id IS NULL"#,
        lib.id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// gets all existing paths in the form of a HashSet
/// this is the weakest layer of deduplication
pub async fn load_existing_paths(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<HashSet<String>, sqlx::Error> {
    let rows = sqlx::query_scalar!("SELECT file_path FROM tracks WHERE library_id = ?", library_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

/// gets all existing isrcs in the form of a HashSet
/// this is a very solid layer of deduplication, if isrc is present for each track
pub async fn load_existing_isrcs(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<HashSet<String>, sqlx::Error> {
    let rows = sqlx::query_scalar!(
        "SELECT isrc FROM tracks WHERE library_id = ? AND isrc IS NOT NULL AND isrc != ''",
        library_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().flatten().collect())
}

/// gets all existing hashes in the form of a HashSet
/// this is the strongest layer of deduplication, but the most performance intensive on initial hashing
pub async fn load_existing_hashes(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<HashSet<String>, sqlx::Error> {
    let rows = sqlx::query_scalar!(
        "SELECT file_hash FROM tracks WHERE library_id = ? AND file_hash IS NOT NULL",
        library_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().flatten().collect())
}

/// inserts a single track into the Sqlite DB keeping the TX alive
pub async fn insert_track(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    track: &TrackInfo,
    library_id: i64,
) -> Result<(), sqlx::Error> {
    // Filenames aren't guaranteed UTF-8 on Linux; skip rather than panic on the
    // rare non-UTF-8 path so one odd file can't abort an entire scan.
    let Some(path) = track.file_path.to_str() else {
        tracing::warn!(path = ?track.file_path, "skipping track with non-UTF-8 path");
        return Ok(());
    };
    sqlx::query!(
        r#"
        INSERT INTO tracks (
        file_path,
        title,
        artist,
        album,
        album_artist,
        album_artists,
        composer,
        label,
        genre,
        comment,
        lyrics,
        track,
        track_total,
        disc,
        disc_total,
        release_year,
        recording_date,
        original_release_date,
        release_type,
        compilation,
        isrc,
        barcode,
        catalog_number,
        bpm,
        language,
        script,
        mood,
        replay_gain_track_gain,
        replay_gain_track_peak,
        replay_gain_album_gain,
        replay_gain_album_peak,
        file_format,
        file_size,
        duration,
        bitrate,
        sample_rate,
        bit_depth,
        channels,
        acoustid,
        musicbrainz_recording_id,
        musicbrainz_track_id,
        musicbrainz_release_id,
        musicbrainz_release_group_id,
        musicbrainz_artist_id,
        musicbrainz_release_artist_id,
        musicbrainz_work_id,
        status,
        file_hash,
        library_id
    ) VALUES (
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?
    )"#,
        path,
        track.title,
        track.artist,
        track.album,
        track.album_artist,
        track.album_artists,
        track.composer,
        track.label,
        track.genre,
        track.comment,
        track.lyrics,
        track.track,
        track.track_total,
        track.disc,
        track.disc_total,
        track.release_year,
        track.recording_date,
        track.original_release_date,
        track.release_type,
        track.compilation,
        track.isrc,
        track.barcode,
        track.catalog_number,
        track.bpm,
        track.language,
        track.script,
        track.mood,
        track.replay_gain_track_gain,
        track.replay_gain_track_peak,
        track.replay_gain_album_gain,
        track.replay_gain_album_peak,
        track.file_format,
        track.file_size,
        track.duration,
        track.bitrate,
        track.sample_rate,
        track.bit_depth,
        track.channels,
        track.acoustid,
        track.musicbrainz_recording_id,
        track.musicbrainz_track_id,
        track.musicbrainz_release_id,
        track.musicbrainz_release_group_id,
        track.musicbrainz_artist_id,
        track.musicbrainz_release_artist_id,
        track.musicbrainz_work_id,
        track.status,
        track.file_hash,
        library_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Maps an anonymous sqlx record (with the standard TrackSummary column set)
/// into a TrackSummary. Used by every summary query so the field wiring lives
/// in one place. All call sites SELECT the same columns in the same shape.
macro_rules! row_to_summary {
    ($row:expr) => {{
        let row = $row;
        TrackSummary {
            is_selected: false,
            // `.into()` accepts both i64 and Option<i64>, since sqlx infers the
            // id column's nullability differently across these queries.
            id: row.id.into(),
            isrc: row.isrc,
            file_path: std::path::PathBuf::from(row.file_path),
            title: row.title,
            artist: row.artist,
            album: row.album,
            file_format: row.file_format,
            file_size: row.file_size,
            duration: row.duration.map(|v| v as u32),
            bitrate: row.bitrate.map(|v| v as u32),
            status: row.status,
            file_hash: row.file_hash,
            marked_for_deletion: row.marked_for_deletion != 0,
            health_issue: row.health_issue,
        }
    }};
}

/// Builds an FTS5 MATCH expression from raw user input. Each whitespace token
/// becomes a quoted prefix term (`"foo"*`), so search-as-you-type matches
/// partial words, and quoting neutralizes FTS5 operator characters in the input.
/// Tokens are implicitly AND-ed by FTS5. Returns `None` when there's nothing to
/// search for.
fn fts_match_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split_whitespace()
        .map(|tok| format!("\"{}\"*", tok.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// db query to return all tracks with a certain status filter, and optionally ID filter
/// potentially expand later to allow more flexibility in queries
pub async fn load_tracks(
    pool: &SqlitePool,
    id: Option<i64>,
    status_filter: Option<&str>,
    search_query: Option<&str>,
    library_id: i64,
) -> Result<Vec<TrackSummary>, sqlx::Error> {
    match (id, status_filter, search_query) {
        (Some(id_val), Some(status), None) => Ok(sqlx::query!(
            r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
                FROM tracks WHERE library_id = ? AND id = ? AND status = ?"#,
            library_id,
            id_val,
            status,
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row_to_summary!(row))
        .collect()),
        (Some(id_val), None, None) => Ok(sqlx::query!(
            r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
                FROM tracks WHERE library_id = ? AND id = ?"#,
            library_id,
            id_val
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row_to_summary!(row))
        .collect()),
        (None, Some(status), None) => Ok(sqlx::query!(
            r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
                FROM tracks WHERE library_id = ? AND status = ?"#,
            library_id,
            status
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row_to_summary!(row))
        .collect()),
        (None, Some(status), Some(query)) => {
            let Some(fts) = fts_match_query(query) else {
                return Box::pin(load_tracks(pool, None, Some(status), None, library_id)).await;
            };
            Ok(sqlx::query!(
                r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
                FROM tracks
                WHERE library_id = ?
                AND id IN (SELECT rowid FROM tracks_fts WHERE tracks_fts MATCH ?)
                AND status = ?"#,
                library_id,
                fts,
                status
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| row_to_summary!(row))
            .collect())
        }
        (None, None, Some(query)) => {
            let Some(fts) = fts_match_query(query) else {
                return Box::pin(load_tracks(pool, None, None, None, library_id)).await;
            };
            Ok(sqlx::query!(
                r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
                FROM tracks WHERE library_id = ?
                AND id IN (SELECT rowid FROM tracks_fts WHERE tracks_fts MATCH ?)"#,
                library_id,
                fts
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| row_to_summary!(row))
            .collect())
        }
        (None, None, None) => Ok(sqlx::query!(
            r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
                FROM tracks WHERE library_id = ?"#,
            library_id
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row_to_summary!(row))
        .collect()),
        _ => Ok(Vec::new()),
    }
}

/// returns the complete TrackInfo for a single track by id
pub async fn load_track_full(pool: &SqlitePool, id: i64) -> Result<Option<TrackInfo>, sqlx::Error> {
    let query_result = sqlx::query!(
        r#"SELECT id, file_path, title, artist, album, album_artist, album_artists,
        composer, label, genre, comment, lyrics, track, track_total, disc, disc_total,
        release_year, recording_date, original_release_date, release_type, compilation,
        isrc, barcode, catalog_number, bpm, language, script, mood,
        replay_gain_track_gain, replay_gain_track_peak, replay_gain_album_gain, replay_gain_album_peak,
        file_format, file_size, duration, bitrate, sample_rate, bit_depth, channels,
        acoustid, musicbrainz_recording_id, musicbrainz_track_id, musicbrainz_release_id,
        musicbrainz_release_group_id, musicbrainz_artist_id, musicbrainz_release_artist_id,
        musicbrainz_work_id, status, file_hash
        FROM tracks WHERE id = ?"#,
        id
    )
    .fetch_optional(pool)
    .await?
    .map(|row| TrackInfo {
        id: Some(row.id),
        file_path: std::path::PathBuf::from(row.file_path),
        title: row.title,
        artist: row.artist,
        album: row.album,
        album_artist: row.album_artist,
        album_artists: row.album_artists,
        composer: row.composer,
        label: row.label,
        genre: row.genre,
        comment: row.comment,
        lyrics: row.lyrics,
        track: row.track.map(|v| v as u32),
        track_total: row.track_total.map(|v| v as u32),
        disc: row.disc.map(|v| v as u32),
        disc_total: row.disc_total.map(|v| v as u32),
        release_year: row.release_year.map(|v| v as u32),
        recording_date: row.recording_date,
        original_release_date: row.original_release_date,
        release_type: row.release_type,
        compilation: row.compilation.map(|v| v != 0),
        isrc: row.isrc,
        barcode: row.barcode,
        catalog_number: row.catalog_number,
        bpm: row.bpm.map(|v| v as u32),
        language: row.language,
        script: row.script,
        mood: row.mood,
        replay_gain_track_gain: row.replay_gain_track_gain,
        replay_gain_track_peak: row.replay_gain_track_peak,
        replay_gain_album_gain: row.replay_gain_album_gain,
        replay_gain_album_peak: row.replay_gain_album_peak,
        file_format: row.file_format,
        file_size: row.file_size,
        duration: row.duration.map(|v| v as u32),
        bitrate: row.bitrate.map(|v| v as u32),
        sample_rate: row.sample_rate.map(|v| v as u32),
        bit_depth: row.bit_depth.map(|v| v as u32),
        channels: row.channels.map(|v| v as u32),
        acoustid: row.acoustid,
        musicbrainz_recording_id: row.musicbrainz_recording_id,
        musicbrainz_track_id: row.musicbrainz_track_id,
        musicbrainz_release_id: row.musicbrainz_release_id,
        musicbrainz_release_group_id: row.musicbrainz_release_group_id,
        musicbrainz_artist_id: row.musicbrainz_artist_id,
        musicbrainz_release_artist_id: row.musicbrainz_release_artist_id,
        musicbrainz_work_id: row.musicbrainz_work_id,
        status: row.status,
        file_hash: row.file_hash,
    });
    Ok(query_result)
}

/// function to count tracks from db with a specified status filter
pub async fn count_tracks(
    pool: &SqlitePool,
    status_filter: Option<&str>,
    library_id: i64,
) -> Result<i64, sqlx::Error> {
    if let Some(status) = status_filter {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) FROM tracks WHERE library_id = ? AND status = ?"#,
            library_id,
            status
        )
        .fetch_one(pool)
        .await
    } else {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) FROM tracks WHERE library_id = ?"#,
            library_id
        )
        .fetch_one(pool)
        .await
    }
}

/// Enrichment dead letters: tracks that failed matching or weren't found.
/// `search` applies the same FTS5 filter as the Library tab so search behaves
/// identically across tabs.
pub async fn load_dead_letter(
    pool: &SqlitePool,
    search: Option<&str>,
    library_id: i64,
) -> Result<Vec<TrackSummary>, sqlx::Error> {
    let fts = search.and_then(fts_match_query);
    match fts {
        Some(fts) => Ok(sqlx::query!(
            r#"SELECT id, isrc, file_path, title, artist, album,
            file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
            FROM tracks WHERE library_id = ? AND status IN ('failed', 'not_found')
            AND id IN (SELECT rowid FROM tracks_fts WHERE tracks_fts MATCH ?)
            ORDER BY artist, album, title"#,
            library_id,
            fts
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row_to_summary!(row))
        .collect()),
        None => Ok(sqlx::query!(
            r#"SELECT id, isrc, file_path, title, artist, album,
            file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
            FROM tracks WHERE library_id = ? AND status IN ('failed', 'not_found')
            ORDER BY artist, album, title"#,
            library_id
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row_to_summary!(row))
        .collect()),
    }
}

/// Aggregated library statistics for the stats screen.
pub struct Stats {
    pub total_tracks: i64,
    pub total_size: i64,
    pub total_duration: i64,
    pub avg_bitrate: f64,
    pub lossless: i64,
    pub lossy: i64,
    pub by_format: Vec<(String, i64, i64)>, // format, count, size
    pub by_decade: Vec<(String, i64)>,
    pub top_artists: Vec<(String, i64)>,
    pub by_status: Vec<(String, i64)>,
    pub health: Vec<(String, i64)>,
}

pub async fn compute_stats(pool: &SqlitePool, library_id: i64) -> Result<Stats, sqlx::Error> {
    let totals = sqlx::query!(
        r#"SELECT COUNT(*) AS "n!", COALESCE(SUM(file_size), 0) AS "size!",
        COALESCE(SUM(duration), 0) AS "dur!", COALESCE(AVG(bitrate), 0.0) AS "avg!"
        FROM tracks WHERE library_id = ?"#,
        library_id
    )
    .fetch_one(pool)
    .await?;

    let lossless = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM tracks
        WHERE library_id = ? AND file_format IN ('FLAC','ALAC','APE','WAV','AIFF','WAVPACK')"#,
        library_id
    )
    .fetch_one(pool)
    .await?;

    let by_format = sqlx::query!(
        r#"SELECT COALESCE(file_format, '?') AS "fmt!: String", COUNT(*) AS "n!: i64",
        COALESCE(SUM(file_size), 0) AS "size!: i64"
        FROM tracks WHERE library_id = ? GROUP BY file_format ORDER BY 2 DESC"#,
        library_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.fmt, r.n, r.size))
    .collect();

    let by_decade = sqlx::query!(
        r#"SELECT ((release_year / 10) * 10) AS "decade!: i64", COUNT(*) AS "n!: i64"
        FROM tracks WHERE library_id = ? AND release_year IS NOT NULL
        GROUP BY 1 ORDER BY 1"#,
        library_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (format!("{}s", r.decade), r.n))
    .collect();

    let top_artists = sqlx::query!(
        r#"SELECT COALESCE(artist, '(unknown)') AS "name!: String", COUNT(*) AS "n!: i64"
        FROM tracks WHERE library_id = ? GROUP BY artist ORDER BY 2 DESC LIMIT 12"#,
        library_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.name, r.n))
    .collect();

    let by_status = sqlx::query!(
        r#"SELECT status AS "s!: String", COUNT(*) AS "n!: i64"
        FROM tracks WHERE library_id = ? GROUP BY status ORDER BY 2 DESC"#,
        library_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.s, r.n))
    .collect();

    let health = sqlx::query!(
        r#"SELECT health_issue AS "issue!: String", COUNT(*) AS "n!: i64"
        FROM tracks WHERE library_id = ? AND health_issue IS NOT NULL
        GROUP BY health_issue ORDER BY 2 DESC"#,
        library_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.issue, r.n))
    .collect();

    Ok(Stats {
        total_tracks: totals.n,
        total_size: totals.size,
        total_duration: totals.dur,
        avg_bitrate: totals.avg,
        lossless,
        lossy: totals.n - lossless,
        by_format,
        by_decade,
        top_artists,
        by_status,
        health,
    })
}

/// One aggregated artist/album group for the grouped Library browse view.
pub struct GroupRow {
    pub name: String,
    pub count: i64,
    pub total_size: i64,
    pub total_duration: i64,
}

/// Aggregates the library by artist or album for the grouped browse view.
pub async fn load_groups(
    pool: &SqlitePool,
    mode: crate::app::GroupMode,
    library_id: i64,
) -> Result<Vec<GroupRow>, sqlx::Error> {
    let rows = match mode {
        crate::app::GroupMode::Artist => sqlx::query!(
            r#"SELECT COALESCE(artist, '(unknown)') AS "name!: String",
                COUNT(*) AS "count!: i64",
                COALESCE(SUM(file_size), 0) AS "total_size!: i64",
                COALESCE(SUM(duration), 0) AS "total_duration!: i64"
                FROM tracks WHERE library_id = ? GROUP BY artist ORDER BY 2 DESC, 1"#,
            library_id
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| GroupRow {
            name: r.name,
            count: r.count,
            total_size: r.total_size,
            total_duration: r.total_duration,
        })
        .collect(),
        _ => sqlx::query!(
            r#"SELECT COALESCE(album, '(unknown)') AS "name!: String",
                COUNT(*) AS "count!: i64",
                COALESCE(SUM(file_size), 0) AS "total_size!: i64",
                COALESCE(SUM(duration), 0) AS "total_duration!: i64"
                FROM tracks WHERE library_id = ? GROUP BY album ORDER BY 2 DESC, 1"#,
            library_id
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| GroupRow {
            name: r.name,
            count: r.count,
            total_size: r.total_size,
            total_duration: r.total_duration,
        })
        .collect(),
    };
    Ok(rows)
}

/// Distinct non-null file formats present in the library, for the format facet.
pub async fn distinct_formats(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<Vec<String>, sqlx::Error> {
    Ok(sqlx::query_scalar!(
        r#"SELECT DISTINCT file_format FROM tracks
        WHERE library_id = ? AND file_format IS NOT NULL ORDER BY file_format"#,
        library_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .flatten()
    .collect())
}

/// Deletes every track in the given library (the "fresh scan" rebuild). Scoped
/// so rebuilding one library never touches another's rows.
pub async fn truncate_tracks(pool: &SqlitePool, library_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(r#"DELETE FROM tracks WHERE library_id = ?"#, library_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Deletes a single track from a provided database id
pub async fn delete_single_track(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(r#"DELETE FROM tracks WHERE id = ?"#, id)
        .execute(pool)
        .await?;
    Ok(())
}

// Undo log

/// Records a reversible action on the undo stack.
pub async fn log_action(
    pool: &SqlitePool,
    kind: &str,
    summary: &str,
    action: &crate::undo::UndoAction,
) -> Result<(), sqlx::Error> {
    let payload = serde_json::to_string(action).unwrap_or_default();
    sqlx::query!(
        r#"INSERT INTO action_log (kind, summary, payload) VALUES (?, ?, ?)"#,
        kind,
        summary,
        payload
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Pops the most recent not-yet-undone action (without marking it undone).
pub async fn take_last_undo(
    pool: &SqlitePool,
) -> Result<Option<(i64, String, crate::undo::UndoAction)>, Box<dyn std::error::Error + Send + Sync>>
{
    let row = sqlx::query!(
        r#"SELECT id AS "id!", summary, payload FROM action_log
        WHERE undone = 0 ORDER BY id DESC LIMIT 1"#
    )
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            let action: crate::undo::UndoAction = serde_json::from_str(&r.payload)?;
            Ok(Some((r.id, r.summary, action)))
        }
        None => Ok(None),
    }
}

pub async fn mark_undone(pool: &SqlitePool, log_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(r#"UPDATE action_log SET undone = 1 WHERE id = ?"#, log_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Re-inserts a full track row (used when undoing a purge). Wraps the existing
/// transaction-based insert in its own transaction.
pub async fn insert_track_pool(
    pool: &SqlitePool,
    track: &TrackInfo,
    library_id: i64,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    insert_track(&mut tx, track, library_id).await?;
    tx.commit().await?;
    Ok(())
}

// Trash workflow

/// Counts tracks currently flagged for deletion (the Trash tab).
pub async fn count_marked(pool: &SqlitePool, library_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM tracks WHERE library_id = ? AND marked_for_deletion = 1"#,
        library_id
    )
    .fetch_one(pool)
    .await
}

/// Loads every track flagged for deletion. `search` applies the same FTS5
/// filter as the Library tab for consistent cross-tab search.
pub async fn load_marked_tracks(
    pool: &SqlitePool,
    search: Option<&str>,
    library_id: i64,
) -> Result<Vec<TrackSummary>, sqlx::Error> {
    let fts = search.and_then(fts_match_query);
    match fts {
        Some(fts) => Ok(sqlx::query!(
            r#"SELECT id, isrc, file_path, title, artist, album,
            file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
            FROM tracks WHERE library_id = ? AND marked_for_deletion = 1
            AND id IN (SELECT rowid FROM tracks_fts WHERE tracks_fts MATCH ?)
            ORDER BY artist, album, title"#,
            library_id,
            fts
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row_to_summary!(row))
        .collect()),
        None => Ok(sqlx::query!(
            r#"SELECT id, isrc, file_path, title, artist, album,
            file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
            FROM tracks WHERE library_id = ? AND marked_for_deletion = 1
            ORDER BY artist, album, title"#,
            library_id
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row_to_summary!(row))
        .collect()),
    }
}

/// Flags or unflags a single track for deletion. Non-destructive - the file is
/// only removed later, during an explicit purge.
pub async fn set_marked_for_deletion(
    pool: &SqlitePool,
    id: i64,
    marked: bool,
) -> Result<(), sqlx::Error> {
    let flag = if marked { 1 } else { 0 };
    sqlx::query!(
        r#"UPDATE tracks SET marked_for_deletion = ? WHERE id = ?"#,
        flag,
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Flags every member of a duplicate group EXCEPT the chosen keeper. Used when
/// resolving a duplicate group by picking which copy to keep.
pub async fn mark_group_except(
    pool: &SqlitePool,
    member_ids: &[i64],
    keeper_id: i64,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for &id in member_ids {
        let flag: i64 = if id == keeper_id { 0 } else { 1 };
        sqlx::query!(
            r#"UPDATE tracks SET marked_for_deletion = ? WHERE id = ?"#,
            flag,
            id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Records (or clears) a track's integrity problem from the health check.
pub async fn set_health_issue(
    pool: &SqlitePool,
    id: i64,
    issue: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE tracks SET health_issue = ? WHERE id = ?"#,
        issue,
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Tracks plus their last-scan time (unix seconds) for the incremental rescan.
pub async fn tracks_for_rescan(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<Vec<(i64, PathBuf, i64)>, sqlx::Error> {
    Ok(sqlx::query!(
        r#"SELECT id AS "id!", file_path,
        CAST(COALESCE(strftime('%s', last_scanned_at), '0') AS INTEGER) AS "last_scanned!: i64"
        FROM tracks WHERE library_id = ?"#,
        library_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.id, PathBuf::from(r.file_path), r.last_scanned))
    .collect())
}

/// Refreshes a track's metadata from a freshly read file (incremental rescan).
pub async fn update_track_metadata(
    pool: &SqlitePool,
    id: i64,
    t: &TrackInfo,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE tracks SET
            title = ?, artist = ?, album = ?, album_artist = ?, album_artists = ?,
            composer = ?, label = ?, genre = ?, comment = ?, lyrics = ?,
            track = ?, track_total = ?, disc = ?, disc_total = ?,
            release_year = ?, recording_date = ?, original_release_date = ?,
            isrc = ?, barcode = ?, catalog_number = ?, bpm = ?,
            language = ?, script = ?, mood = ?,
            replay_gain_track_gain = ?, replay_gain_track_peak = ?,
            replay_gain_album_gain = ?, replay_gain_album_peak = ?,
            file_format = ?, file_size = ?, duration = ?, bitrate = ?,
            sample_rate = ?, bit_depth = ?, channels = ?, file_hash = ?,
            last_scanned_at = CURRENT_TIMESTAMP
        WHERE id = ?"#,
        t.title,
        t.artist,
        t.album,
        t.album_artist,
        t.album_artists,
        t.composer,
        t.label,
        t.genre,
        t.comment,
        t.lyrics,
        t.track,
        t.track_total,
        t.disc,
        t.disc_total,
        t.release_year,
        t.recording_date,
        t.original_release_date,
        t.isrc,
        t.barcode,
        t.catalog_number,
        t.bpm,
        t.language,
        t.script,
        t.mood,
        t.replay_gain_track_gain,
        t.replay_gain_track_peak,
        t.replay_gain_album_gain,
        t.replay_gain_album_peak,
        t.file_format,
        t.file_size,
        t.duration,
        t.bitrate,
        t.sample_rate,
        t.bit_depth,
        t.channels,
        t.file_hash,
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn load_tracks_paths(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<Vec<(i64, PathBuf)>, sqlx::Error> {
    let query_result = sqlx::query!(
        r#"SELECT id, file_path FROM tracks WHERE library_id = ?"#,
        library_id
    )
    .fetch_all(pool)
    .await?
        .into_iter()
        // `id` is a non-null PK in practice, but sqlx infers it nullable here;
        // skip any NULL rather than unwrap-panicking.
        .filter_map(|r| r.id.map(|id| (id, PathBuf::from(r.file_path))))
        .collect();
    Ok(query_result)
}

/// Applies manual tag edits to a track row and marks it `manual_review`.
/// Mirrors what was written to the file so the DB stays consistent.
pub async fn update_track_tags(
    pool: &SqlitePool,
    id: i64,
    e: &crate::track::TagEdits,
) -> Result<(), sqlx::Error> {
    use crate::track::TagEdits;
    let title = TagEdits::opt(&e.title);
    let artist = TagEdits::opt(&e.artist);
    let album = TagEdits::opt(&e.album);
    let album_artist = TagEdits::opt(&e.album_artist);
    let genre = TagEdits::opt(&e.genre);
    let comment = TagEdits::opt(&e.comment);
    let track = e.track.trim().parse::<i64>().ok();
    let disc = e.disc.trim().parse::<i64>().ok();
    let year = e.year.trim().parse::<i64>().ok();

    sqlx::query!(
        r#"UPDATE tracks SET
            title = ?, artist = ?, album = ?, album_artist = ?,
            genre = ?, comment = ?, track = ?, disc = ?, release_year = ?,
            status = 'manual_review'
        WHERE id = ?"#,
        title,
        artist,
        album,
        album_artist,
        genre,
        comment,
        track,
        disc,
        year,
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_track_status(
    pool: &SqlitePool,
    id: i64,
    new_status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE tracks SET status = ? WHERE id = ?"#,
        new_status,
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Writes enrichment results back to a track and marks it `enriched`. Existing
/// tag values are preserved (COALESCE keeps the current value when set), so we
/// only fill gaps rather than overwriting tags the file already carried.
pub async fn apply_enrichment(
    pool: &SqlitePool,
    id: i64,
    r: &crate::enrich::EnrichmentResult,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE tracks SET
            acoustid = COALESCE(acoustid, ?),
            musicbrainz_recording_id = COALESCE(musicbrainz_recording_id, ?),
            musicbrainz_release_group_id = COALESCE(musicbrainz_release_group_id, ?),
            musicbrainz_artist_id = COALESCE(musicbrainz_artist_id, ?),
            title = COALESCE(title, ?),
            artist = COALESCE(artist, ?),
            album = COALESCE(album, ?),
            status = 'enriched',
            enriched_at = CURRENT_TIMESTAMP
        WHERE id = ?"#,
        r.acoustid,
        r.mb_recording_id,
        r.mb_release_group_id,
        r.mb_artist_id,
        r.title,
        r.artist,
        r.album,
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Builds the duplicate-group list shown in the Duplicates tab. Two kinds of
/// groups are produced, matching the scanner's own detection layers:

///   * Hash groups  - byte-for-byte identical files (same BLAKE3 hash).
///   * ISRC groups  - the same recording across non-identical files (e.g. a
///                    FLAC and an MP3 of the same track). To avoid restating a
///                    hash group, an ISRC group is only included when its
///                    members span more than one distinct file (different or
///                    missing hashes), i.e. it captures something the hash
///                    layer can't.
pub async fn load_duplicate_groups(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<Vec<DuplicateGroupSummary>, sqlx::Error> {
    let mut groups: Vec<DuplicateGroupSummary> = sqlx::query!(
        r#"SELECT file_hash,
        COUNT(*) AS "n!: i64",
        MIN(title) AS "title?: String",
        MIN(artist) AS "artist?: String",
        MIN(album) AS "album?: String"
        FROM tracks
        WHERE library_id = ? AND file_hash IS NOT NULL AND marked_for_deletion = 0
          AND NOT EXISTS (SELECT 1 FROM duplicates_skipped ds WHERE ds.key = file_hash AND ds.kind = 'hash' AND ds.library_id = ?)
        GROUP BY file_hash
        HAVING COUNT(*) > 1
        ORDER BY 2 DESC, 4, 5, 3"#,
        library_id,
        library_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| DuplicateGroupSummary {
        kind: DuplicateKind::Hash,
        key: row.file_hash.unwrap_or_default(),
        count: row.n as u32,
        title: row.title,
        artist: row.artist,
        album: row.album,
    })
    .collect();

    let isrc_groups = sqlx::query!(
        r#"SELECT isrc,
        COUNT(*) AS "n!: i64",
        MIN(title) AS "title?: String",
        MIN(artist) AS "artist?: String",
        MIN(album) AS "album?: String"
        FROM tracks
        WHERE library_id = ? AND isrc IS NOT NULL AND isrc != '' AND marked_for_deletion = 0
          AND NOT EXISTS (SELECT 1 FROM duplicates_skipped ds WHERE ds.key = isrc AND ds.kind = 'isrc' AND ds.library_id = ?)
        GROUP BY isrc
        HAVING COUNT(*) > 1
            AND COUNT(DISTINCT COALESCE(file_hash, CAST(id AS TEXT))) > 1
        ORDER BY 2 DESC, 4, 5, 3"#,
        library_id,
        library_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| DuplicateGroupSummary {
        kind: DuplicateKind::Isrc,
        key: row.isrc.unwrap_or_default(),
        count: row.n as u32,
        title: row.title,
        artist: row.artist,
        album: row.album,
    });

    groups.extend(isrc_groups);
    Ok(groups)
}

/// Counts the duplicate groups shown in the Duplicates tab without loading their
/// members. Mirrors the grouping in `load_duplicate_groups` (hash groups + ISRC
/// groups that span more than one distinct file) so the tab badge stays correct
/// cheaply, instead of reloading the full DuplicatesView.
pub async fn count_duplicate_groups(pool: &SqlitePool, library_id: i64) -> Result<i64, sqlx::Error> {
    let hash_groups = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM (
            SELECT file_hash FROM tracks
            WHERE library_id = ? AND file_hash IS NOT NULL AND marked_for_deletion = 0
              AND NOT EXISTS (SELECT 1 FROM duplicates_skipped ds WHERE ds.key = file_hash AND ds.kind = 'hash' AND ds.library_id = ?)
            GROUP BY file_hash HAVING COUNT(*) > 1
        )"#,
        library_id,
        library_id
    )
    .fetch_one(pool)
    .await?;

    let isrc_groups = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM (
            SELECT isrc FROM tracks
            WHERE library_id = ? AND isrc IS NOT NULL AND isrc != '' AND marked_for_deletion = 0
              AND NOT EXISTS (SELECT 1 FROM duplicates_skipped ds WHERE ds.key = isrc AND ds.kind = 'isrc' AND ds.library_id = ?)
            GROUP BY isrc
            HAVING COUNT(*) > 1
                AND COUNT(DISTINCT COALESCE(file_hash, CAST(id AS TEXT))) > 1
        )"#,
        library_id,
        library_id
    )
    .fetch_one(pool)
    .await?;

    Ok(hash_groups + isrc_groups)
}

/// Loads the member tracks of a duplicate group, keyed by hash or ISRC.
pub async fn load_duplicate_members(
    pool: &SqlitePool,
    kind: DuplicateKind,
    key: &str,
    library_id: i64,
) -> Result<Vec<TrackSummary>, sqlx::Error> {
    let rows = match kind {
        DuplicateKind::Hash => {
            sqlx::query!(
                r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
                FROM tracks WHERE library_id = ? AND file_hash = ? AND marked_for_deletion = 0"#,
                library_id,
                key
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| row_to_summary!(row))
            .collect()
        }
        DuplicateKind::Isrc => {
            sqlx::query!(
                r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash, marked_for_deletion, health_issue
                FROM tracks WHERE library_id = ? AND isrc = ? AND marked_for_deletion = 0"#,
                library_id,
                key
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| row_to_summary!(row))
            .collect()
        }
    };
    Ok(rows)
}

/// Records that the user dismissed a duplicate group (keep all), so it no
/// longer appears in the Duplicates tab
pub async fn skip_duplicate_group(
    pool: &SqlitePool,
    key: &str,
    kind: &str,
    library_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"INSERT OR IGNORE INTO duplicates_skipped (key, kind, library_id) VALUES (?, ?, ?)"#,
        key,
        kind,
        library_id
    )
    .execute(pool)
    .await?;
    Ok(())
}
