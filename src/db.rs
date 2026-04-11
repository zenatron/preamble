use crate::track::{TrackInfo, TrackSummary};
use sqlx::{SqlitePool};
use std::{collections::HashSet, path::PathBuf};

// initializes the Sqlite Pool
pub async fn init_db() -> Result<SqlitePool, sqlx::Error> {
    let pool = sqlx::SqlitePool::connect("sqlite://library.db?mode=rwc").await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

// gets all existing paths in the form of a HashSet
// this is the weakest layer of deduplication
pub async fn load_existing_paths(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows = sqlx::query_scalar!("SELECT file_path FROM tracks")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

// gets all existing isrcs in the form of a HashSet
// this is a very solid layer of deduplication, if isrc is present for each track
pub async fn load_existing_isrcs(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows = sqlx::query_scalar!("SELECT isrc FROM tracks WHERE isrc IS NOT NULL")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().flatten().collect())
}

// gets all existing hashes in the form of a HashSet
// this is the strongest layer of deduplication, but the most performance intensive on initial hashing
pub async fn load_existing_hashes(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows = sqlx::query_scalar!("SELECT file_hash FROM tracks WHERE file_hash IS NOT NULL")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().flatten().collect())
}

// inserts a single track into the Sqlite DB keeping the TX alive
pub async fn insert_track(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    track: &TrackInfo,
) -> Result<(), sqlx::Error> {
    let path = track.file_path.to_str().unwrap();
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
        file_hash
    ) VALUES (
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?
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
        track.file_hash
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}


// db query to return all tracks with a certain status filter, and optionally ID filter
// potentially expand later to allow more flexibility in queries
pub async fn load_tracks(
    pool: &SqlitePool,
    status_filter: Option<&str>,
    id: Option<i64>,
) -> Result<Vec<TrackSummary>, sqlx::Error> {
    match (id, status_filter) {
        (Some(id_val), Some(status)) => {
            Ok(sqlx::query!(
                r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash
                FROM tracks WHERE id = ? AND status = ?"#,
                id_val,
                status
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| TrackSummary {
                is_selected: false,
                id: Some(row.id),
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
            })
            .collect())
        }
        (Some(id_val), None) => {
            Ok(sqlx::query!(
                r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash
                FROM tracks WHERE id = ?"#,
                id_val
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| TrackSummary {
                is_selected: false,
                id: Some(row.id),
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
            })
            .collect())
        }
        (None, Some(status)) => {
            Ok(sqlx::query!(
                r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash
                FROM tracks WHERE status = ?"#,
                status
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| TrackSummary {
                is_selected: false,
                id: Some(row.id),
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
            })
            .collect())
        }
        (None, None) => {
            Ok(sqlx::query!(
                r#"SELECT id, isrc, file_path, title, artist, album,
                file_format, file_size, duration, bitrate, status, file_hash
                FROM tracks"#
            )
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| TrackSummary {
                is_selected: false,
                id: Some(row.id),
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
            })
            .collect())
        }
    }
}

// returns the complete TrackInfo for a single track by id
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

// function to count tracks from db with a specified status filter
pub async fn count_tracks(
    pool: &SqlitePool,
    status_filter: Option<&str>,
) -> Result<i64, sqlx::Error> {
    if let Some(status) = status_filter {
        sqlx::query_scalar!(r#"SELECT COUNT(*) FROM tracks WHERE status = ?"#, status)
            .fetch_one(pool)
            .await
    } else {
        sqlx::query_scalar!(r#"SELECT COUNT(*) FROM tracks"#)
            .fetch_one(pool)
            .await
    }
}

pub async fn truncate_tracks(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query!(r#"DELETE FROM tracks"#).execute(pool).await?;
    Ok(())
}

pub async fn delete_single_track(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(r#"DELETE FROM tracks WHERE id = ?"#, id).execute(pool).await?;
    Ok(())
}

pub async fn load_tracks_paths(pool: &SqlitePool) -> Result<Vec<(i64, PathBuf)>, sqlx::Error> {
    let query_result = sqlx::query!(r#"SELECT id, file_path FROM tracks"#)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| (r.id.unwrap(), PathBuf::from(r.file_path)))
        .collect();
    Ok(query_result)
}

pub async fn update_track_status(pool: &SqlitePool, id: i64, new_status: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(r#"UPDATE tracks SET status = ? WHERE id = ?"#, new_status, id).execute(pool).await?;
    Ok(())
}