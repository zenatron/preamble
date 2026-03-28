use crate::track::TrackInfo;
use sqlx::SqlitePool;

pub async fn init_db() -> Result<SqlitePool, sqlx::Error> {
    let pool = sqlx::SqlitePool::connect("sqlite://library.db?mode=rwc").await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub async fn insert_track(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    track: &TrackInfo,
) -> Result<(), sqlx::Error> {
    let path = track.file_path.to_str().unwrap();
    sqlx::query!(
        r#"
        INSERT OR IGNORE INTO tracks (
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
        status
    ) VALUES (
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?
    )"#,
        path, track.title, track.artist, track.album, track.album_artist, track.album_artists, track.composer, track.label, track.genre, track.comment, track.lyrics,
        track.track, track.track_total, track.disc, track.disc_total, track.release_year, track.recording_date, track.original_release_date,
        track.release_type, track.compilation, track.isrc, track.barcode, track.catalog_number, track.bpm, track.language, track.script, track.mood,
        track.replay_gain_track_gain, track.replay_gain_track_peak, track.replay_gain_album_gain, track.replay_gain_album_peak,
        track.file_format, track.file_size, track.duration, track.bitrate, track.sample_rate, track.bit_depth, track.channels,
        track.acoustid, track.musicbrainz_recording_id, track.musicbrainz_track_id, track.musicbrainz_release_id,
        track.musicbrainz_release_group_id, track.musicbrainz_artist_id, track.musicbrainz_release_artist_id, track.musicbrainz_work_id,
        track.status
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
