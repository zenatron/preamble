// src/enrich.rs
//
// Metadata enrichment pipeline for `pending` tracks.
//
//   audio file --> fpcalc --> fingerprint+duration --> AcoustID --> MusicBrainz
//   recording/release-group metadata --> written back to the row.
//
// AcoustID's `meta=recordings+releasegroups` returns metadata sourced from
// MusicBrainz (recording/artist/release-group MBIDs + titles), so a single
// AcoustID call yields the MusicBrainz match without a second round-trip.
// Requests are issued sequentially with a small delay to stay within the
// AcoustID rate limit, and the HTTP client sends an identifying User-Agent.

use std::time::Duration;

use serde::Deserialize;
use sqlx::SqlitePool;

use crate::db;

const ACOUSTID_LOOKUP_URL: &str = "https://api.acoustid.org/v2/lookup";
const USER_AGENT: &str = concat!("preamble/", env!("CARGO_PKG_VERSION"));
// Spacing between AcoustID requests; the service asks for <= 3 req/s.
const REQUEST_SPACING: Duration = Duration::from_millis(350);

pub enum EnrichEvent {
    Progress(usize, usize), // processed, total
    Done,
    Error(String),
}

/// Metadata resolved for a single track. Fields are `None` when the match did
/// not supply them.
#[derive(Default)]
pub struct EnrichmentResult {
    pub acoustid: Option<String>,
    pub mb_recording_id: Option<String>,
    pub mb_release_group_id: Option<String>,
    pub mb_artist_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

// fpcalc (Chromaprint)

#[derive(Deserialize)]
struct FpcalcOutput {
    duration: f64,
    fingerprint: String,
}

async fn fingerprint(path: &std::path::Path) -> Result<FpcalcOutput, String> {
    let output = tokio::process::Command::new("fpcalc")
        .arg("-json")
        .arg(path)
        .output()
        .await
        .map_err(|e| format!("failed to run fpcalc (is it installed?): {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "fpcalc failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    serde_json::from_slice::<FpcalcOutput>(&output.stdout)
        .map_err(|e| format!("could not parse fpcalc output: {e}"))
}

// AcoustID response shapes (partial)

#[derive(Deserialize)]
struct AcoustIdResponse {
    status: String,
    #[serde(default)]
    error: Option<AcoustIdError>,
    #[serde(default)]
    results: Vec<AcoustIdResult>,
}

#[derive(Deserialize)]
struct AcoustIdError {
    message: String,
}

#[derive(Deserialize)]
struct AcoustIdResult {
    id: String,
    #[serde(default)]
    score: f64,
    #[serde(default)]
    recordings: Vec<AcoustIdRecording>,
}

#[derive(Deserialize)]
struct AcoustIdRecording {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artists: Vec<AcoustIdArtist>,
    #[serde(default)]
    releasegroups: Vec<AcoustIdReleaseGroup>,
}

#[derive(Deserialize)]
struct AcoustIdArtist {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct AcoustIdReleaseGroup {
    #[serde(default)]
    title: Option<String>,
}

/// Looks up a fingerprint against AcoustID and folds the best match into an
/// `EnrichmentResult`. Returns `Ok(None)` when AcoustID has no recording match.
async fn lookup(
    client: &reqwest::Client,
    api_key: &str,
    fp: &FpcalcOutput,
) -> Result<Option<EnrichmentResult>, String> {
    let duration = fp.duration.round() as i64;
    let response = client
        .get(ACOUSTID_LOOKUP_URL)
        .query(&[
            ("client", api_key),
            ("duration", &duration.to_string()),
            ("fingerprint", &fp.fingerprint),
            ("meta", "recordings+releasegroups+compress"),
        ])
        .send()
        .await
        .map_err(|e| format!("AcoustID request failed: {e}"))?;

    let body = response
        .json::<AcoustIdResponse>()
        .await
        .map_err(|e| format!("could not parse AcoustID response: {e}"))?;

    if body.status != "ok" {
        let msg = body
            .error
            .map(|e| e.message)
            .unwrap_or_else(|| "unknown error".to_string());
        return Err(format!("AcoustID error: {msg}"));
    }

    // Highest-scoring result, then its first recording.
    let best = body.results.into_iter().max_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let Some(best) = best else {
        return Ok(None);
    };

    let acoustid = Some(best.id);
    let Some(recording) = best.recordings.into_iter().next() else {
        // Matched a fingerprint but no linked recording metadata.
        return Ok(Some(EnrichmentResult {
            acoustid,
            ..Default::default()
        }));
    };

    let artist_names: Vec<String> = recording.artists.iter().map(|a| a.name.clone()).collect();
    let artist = (!artist_names.is_empty()).then(|| artist_names.join("; "));
    let mb_artist_id = recording.artists.into_iter().next().map(|a| a.id);
    let album = recording.releasegroups.into_iter().find_map(|rg| rg.title);

    Ok(Some(EnrichmentResult {
        acoustid,
        mb_recording_id: Some(recording.id),
        mb_release_group_id: None, // release-group MBID omitted by compress meta
        mb_artist_id,
        title: recording.title,
        artist,
        album,
    }))
}

/// Enriches every `pending` track. Emits progress over `sender` and updates
/// each row's status to `enriched`, `not_found`, or `failed`.
pub async fn enrich_pending(
    pool: SqlitePool,
    api_key: String,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    sender: tokio::sync::mpsc::Sender<EnrichEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let pending = db::load_tracks(&pool, None, Some("pending"), None).await?;
    let total = pending.len();

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()?;

    for (processed, track) in pending.into_iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::info!("enrichment cancelled by user");
            break;
        }
        let Some(id) = track.id else { continue };

        let new_status = match fingerprint(&track.file_path).await {
            Ok(fp) => match lookup(&client, &api_key, &fp).await {
                Ok(Some(result)) => {
                    db::apply_enrichment(&pool, id, &result).await.ok();
                    None // status set to 'enriched' inside apply_enrichment
                }
                Ok(None) => Some("not_found"),
                Err(msg) => {
                    sender.try_send(EnrichEvent::Error(msg)).ok();
                    Some("failed")
                }
            },
            Err(msg) => {
                sender.try_send(EnrichEvent::Error(msg)).ok();
                Some("failed")
            }
        };

        if let Some(status) = new_status {
            db::update_track_status(&pool, id, status).await.ok();
        }

        sender
            .try_send(EnrichEvent::Progress(processed + 1, total))
            .ok();

        // Stay within the AcoustID rate limit.
        tokio::time::sleep(REQUEST_SPACING).await;
    }

    sender.send(EnrichEvent::Done).await.ok();
    Ok(())
}
