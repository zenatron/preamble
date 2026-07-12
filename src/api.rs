// src/api.rs
//
// Localhost HTTP API so an MCP server (or any tool) can query the music
// library while preamble is running. All endpoints are read-only currently
//
// The server only starts when PREAMBLE_API_PORT is set so it never interferes
// with normal interactive use.

use crate::db;
use crate::track::TrackInfo;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};

// app state

#[derive(Clone)]
pub struct ApiState {
    pub pool: SqlitePool,
}

// responses

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    version: &'static str,
}

#[derive(Serialize)]
struct LibraryResponse {
    id: i64,
    name: String,
    path: String,
}

#[derive(Serialize)]
struct StatsResponse {
    total_tracks: i64,
    total_size: i64,
    total_duration_ms: i64,
    avg_bitrate_kbps: f64,
    lossless: i64,
    lossy: i64,
    by_format: Vec<FormatBreakdown>,
    by_decade: Vec<DecadeBreakdown>,
    top_artists: Vec<ArtistBreakdown>,
    by_status: Vec<StatusBreakdown>,
    health_issues: Vec<HealthBreakdown>,
}

#[derive(Serialize)]
struct FormatBreakdown {
    format: String,
    count: i64,
    total_size: i64,
}

#[derive(Serialize)]
struct DecadeBreakdown {
    decade: String,
    count: i64,
}

#[derive(Serialize)]
struct ArtistBreakdown {
    artist: String,
    count: i64,
}

#[derive(Serialize)]
struct StatusBreakdown {
    status: String,
    count: i64,
}

#[derive(Serialize)]
struct HealthBreakdown {
    issue: String,
    count: i64,
}

// track response in flat json format
#[derive(Serialize)]
struct TrackResponse {
    id: i64,
    file_path: String,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    genre: Option<String>,
    track: Option<u32>,
    track_total: Option<u32>,
    disc: Option<u32>,
    disc_total: Option<u32>,
    release_year: Option<u32>,
    isrc: Option<String>,
    file_format: Option<String>,
    file_size: Option<i64>,
    duration_ms: Option<u32>,
    bitrate_kbps: Option<u32>,
    sample_rate: Option<u32>,
    bit_depth: Option<u32>,
    channels: Option<u32>,
    status: String,
    acoustid: Option<String>,
    musicbrainz_recording_id: Option<String>,
    musicbrainz_release_group_id: Option<String>,
    health_issue: Option<String>,
    file_hash: Option<String>,
    composer: Option<String>,
    label: Option<String>,
    comment: Option<String>,
    bpm: Option<u32>,
    compilation: Option<bool>,
}

impl From<TrackInfo> for TrackResponse {
    fn from(t: TrackInfo) -> Self {
        Self {
            id: t.id.unwrap_or(0),
            file_path: t.file_path.to_string_lossy().into_owned(),
            title: t.title,
            artist: t.artist,
            album: t.album,
            album_artist: t.album_artist,
            genre: t.genre,
            track: t.track,
            track_total: t.track_total,
            disc: t.disc,
            disc_total: t.disc_total,
            release_year: t.release_year,
            isrc: t.isrc,
            file_format: t.file_format,
            file_size: t.file_size,
            duration_ms: t.duration,
            bitrate_kbps: t.bitrate,
            sample_rate: t.sample_rate,
            bit_depth: t.bit_depth,
            channels: t.channels,
            status: t.status,
            acoustid: t.acoustid,
            musicbrainz_recording_id: t.musicbrainz_recording_id,
            musicbrainz_release_group_id: t.musicbrainz_release_group_id,
            health_issue: None,
            file_hash: t.file_hash,
            composer: t.composer,
            label: t.label,
            comment: t.comment,
            bpm: t.bpm,
            compilation: t.compilation,
        }
    }
}

#[derive(Serialize)]
struct TrackSummaryResponse {
    id: Option<i64>,
    file_path: String,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    file_format: Option<String>,
    file_size: Option<i64>,
    duration_ms: Option<u32>,
    bitrate_kbps: Option<u32>,
    status: String,
    isrc: Option<String>,
    file_hash: Option<String>,
    health_issue: Option<String>,
    marked_for_deletion: bool,
}

impl From<crate::track::TrackSummary> for TrackSummaryResponse {
    fn from(s: crate::track::TrackSummary) -> Self {
        Self {
            id: s.id,
            file_path: s.file_path.to_string_lossy().into_owned(),
            title: s.title,
            artist: s.artist,
            album: s.album,
            file_format: s.file_format,
            file_size: s.file_size,
            duration_ms: s.duration,
            bitrate_kbps: s.bitrate,
            status: s.status,
            isrc: s.isrc,
            file_hash: s.file_hash,
            health_issue: s.health_issue,
            marked_for_deletion: s.marked_for_deletion,
        }
    }
}

#[derive(Serialize)]
struct DuplicateGroupResponse {
    kind: String,
    key: String,
    count: u32,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
}

#[derive(Serialize)]
struct ExportResponse {
    path: String,
    format: String,
    tracks: usize,
}

// query params

#[derive(Deserialize)]
struct LibraryQuery {
    library_id: i64,
}

#[derive(Deserialize)]
struct TracksQuery {
    library_id: i64,
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    100
}

#[derive(Deserialize)]
struct ExportQuery {
    library_id: i64,
    #[serde(default = "default_export_format")]
    format: String,
}

fn default_export_format() -> String {
    "json".to_string()
}

// api handlers

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn list_libraries(State(state): State<ApiState>) -> Result<Json<Vec<LibraryResponse>>, StatusCode> {
    let libs = db::list_libraries(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        libs.into_iter()
            .map(|l| LibraryResponse {
                id: l.id,
                name: l.name,
                path: l.path,
            })
            .collect(),
    ))
}

async fn get_stats(
    State(state): State<ApiState>,
    Query(q): Query<LibraryQuery>,
) -> Result<Json<StatsResponse>, StatusCode> {
    let stats = db::compute_stats(&state.pool, q.library_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(StatsResponse {
        total_tracks: stats.total_tracks,
        total_size: stats.total_size,
        total_duration_ms: stats.total_duration,
        avg_bitrate_kbps: stats.avg_bitrate,
        lossless: stats.lossless,
        lossy: stats.lossy,
        by_format: stats
            .by_format
            .into_iter()
            .map(|(format, count, total_size)| FormatBreakdown {
                format,
                count,
                total_size,
            })
            .collect(),
        by_decade: stats
            .by_decade
            .into_iter()
            .map(|(decade, count)| DecadeBreakdown { decade, count })
            .collect(),
        top_artists: stats
            .top_artists
            .into_iter()
            .map(|(artist, count)| ArtistBreakdown { artist, count })
            .collect(),
        by_status: stats
            .by_status
            .into_iter()
            .map(|(status, count)| StatusBreakdown { status, count })
            .collect(),
        health_issues: stats
            .health
            .into_iter()
            .map(|(issue, count)| HealthBreakdown { issue, count })
            .collect(),
    }))
}

async fn list_tracks(
    State(state): State<ApiState>,
    Query(q): Query<TracksQuery>,
) -> Result<Json<Vec<TrackSummaryResponse>>, StatusCode> {
    let tracks = db::load_tracks(
        &state.pool,
        None,
        q.status.as_deref(),
        q.search.as_deref(),
        q.library_id,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let result: Vec<TrackSummaryResponse> = tracks
        .into_iter()
        .take(q.limit)
        .map(TrackSummaryResponse::from)
        .collect();

    Ok(Json(result))
}

async fn get_track(
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<TrackResponse>, StatusCode> {
    let track = db::load_track_full(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(TrackResponse::from(track)))
}

async fn list_duplicates(
    State(state): State<ApiState>,
    Query(q): Query<LibraryQuery>,
) -> Result<Json<Vec<DuplicateGroupResponse>>, StatusCode> {
    let groups = db::load_duplicate_groups(&state.pool, q.library_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        groups
            .into_iter()
            .map(|g| DuplicateGroupResponse {
                kind: g.kind.label().to_string(),
                key: g.key,
                count: g.count,
                title: g.title,
                artist: g.artist,
                album: g.album,
            })
            .collect(),
    ))
}

async fn export_tracks(
    State(state): State<ApiState>,
    Query(q): Query<ExportQuery>,
) -> Result<Json<ExportResponse>, StatusCode> {
    let tracks = db::load_tracks(&state.pool, None, None, None, q.library_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let ext = match q.format.as_str() {
        "csv" => "csv",
        "m3u" => "m3u",
        _ => "json",
    };

    let path = crate::export::export_path("library", ext);
    let format = q.format.clone();

    match format.as_str() {
        "csv" => crate::export::export_csv(&tracks, &path),
        "m3u" => crate::export::export_m3u(&tracks, &path),
        _ => crate::export::export_json(&tracks, &path),
    }
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ExportResponse {
        path: path.to_string_lossy().into_owned(),
        format,
        tracks: tracks.len(),
    }))
}

// server starting

/// Starts the HTTP API server on `127.0.0.1:{port}`. Binds as a background
/// tokio task so it doesn't block the TUI. Returns the bound address.
pub async fn start(pool: SqlitePool, port: u16) -> std::io::Result<SocketAddr> {
    let state = ApiState { pool };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = axum::Router::new()
        .route("/api/health", get(health))
        .route("/api/libraries", get(list_libraries))
        .route("/api/stats", get(get_stats))
        .route("/api/tracks", get(list_tracks))
        .route("/api/tracks/{id}", get(get_track))
        .route("/api/duplicates", get(list_duplicates))
        .route("/api/export", get(export_tracks))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("API server error: {e}");
        }
    });

    tracing::info!("API server listening on http://{bound}");
    Ok(bound)
}

/// Returns the port from PREAMBLE_API_PORT env var, or None if not set.
pub fn port_from_env() -> Option<u16> {
    std::env::var("PREAMBLE_API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
}
