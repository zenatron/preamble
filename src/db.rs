use crate::track::{TrackInfo, TrackSummary};
use sqlx::{SqlitePool};
use std::collections::HashSet;

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


// db query to return all tracks with a certain status filter
// potentially expand later to allow more flexibility in queries
pub async fn load_tracks(
    pool: &SqlitePool,
    status_filter: Option<&str>,
) -> Result<Vec<TrackSummary>, sqlx::Error> {
    if let Some(status) = status_filter {
        let query_result = sqlx::query!(
            r#"
        SELECT id, isrc, file_path,
        title, artist, album,
        file_format, file_size, duration, bitrate,
        status, file_hash
        FROM tracks
        WHERE status = ?
        "#,
            status
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| TrackSummary {
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
        .collect();
        Ok(query_result)
    } else {
        let query_result = sqlx::query!(
            r#"
        SELECT id, isrc, file_path,
        title, artist, album,
        file_format, file_size, duration, bitrate,
        status, file_hash
        FROM tracks
        "#
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| TrackSummary {
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
        .collect();
        Ok(query_result)
    }
}
