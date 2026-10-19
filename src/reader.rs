// src/reader.rs

use std::collections::HashSet;
use std::fs::{self};
use std::path::{Path, PathBuf};

use crate::{db, track};
use crate::track::{hash_file, read_tags};

use std::sync::Arc;
use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;
use sqlx::SqlitePool;

/// Walks `root` collecting audio files (by extension, case-insensitively) that
/// aren't already in `existing`.
///
/// Symlinked directories are followed, but each real directory is visited once,
/// so a link back up the tree can't loop forever. A directory that can't be read
/// (permissions, vanished mid-walk) is logged and skipped instead of failing the
/// whole scan.
pub fn collect_new_paths(
    root: &Path,
    paths: &mut Vec<PathBuf>,
    existing: &HashSet<String>,
    formats: &HashSet<String>,
) {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(real) = fs::canonicalize(&dir) else {
            continue;
        };
        if !visited.insert(real) {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "skipping unreadable directory");
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `metadata` follows symlinks, so a link to a dir is walked as one.
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                let ext_matches = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| formats.contains(&e.to_lowercase()));
                if ext_matches && !existing.contains(path.to_str().unwrap_or("")) {
                    paths.push(path);
                }
            }
        }
    }
    // Stable order: progress and duplicate "first seen" are deterministic.
    paths.sort();
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Imports every new audio file under `path` into the library.
///
/// Always finishes by sending `ScanEvent::Done`, preceded by `ScanEvent::Error`
/// if the scan failed part-way, so the UI never waits on a dead worker.
pub async fn scan_library(
    pool: SqlitePool,
    library_id: i64,
    path: PathBuf,
    formats: Vec<String>,
    concurrency: usize,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    scan_sender: tokio::sync::mpsc::Sender<ScanEvent>,
) -> Result<(), BoxError> {
    let result = scan_library_inner(
        &pool,
        library_id,
        path,
        formats,
        concurrency,
        &cancel,
        &scan_sender,
    )
    .await;
    if let Err(e) = &result {
        tracing::error!(error = %e, "scan failed");
        scan_sender.send(ScanEvent::Error(format!("Scan failed: {e}"))).await.ok();
    }
    scan_sender.send(ScanEvent::Done).await.ok();
    result
}

async fn scan_library_inner(
    pool: &SqlitePool,
    library_id: i64,
    path: PathBuf,
    formats: Vec<String>,
    concurrency: usize,
    cancel: &std::sync::atomic::AtomicBool,
    scan_sender: &tokio::sync::mpsc::Sender<ScanEvent>,
) -> Result<(), BoxError> {
    // Reads/hashes are buffered and written in batches of this size, each in a
    // short transaction. The write lock is only held while a batch is flushed
    // (not while files are hashed), so UI actions during a scan don't stall,
    // and a cancel keeps everything flushed so far.
    const COMMIT_BATCH: usize = 500;

    let formats: HashSet<String> = formats.into_iter().collect();

    // Existing paths / ISRCs / hashes in this library, for the three dedup layers.
    let existing_tracks = db::load_existing_paths(pool, library_id).await?;
    let existing_isrcs = db::load_existing_isrcs(pool, library_id).await?;
    let existing_hashes = db::load_existing_hashes(pool, library_id).await?;

    let new_track_paths = tokio::task::spawn_blocking(move || {
        let mut paths = Vec::new();
        collect_new_paths(&path, &mut paths, &existing_tracks, &formats);
        paths
    })
    .await?;

    let total = new_track_paths.len();
    if total == 0 {
        return Ok(());
    }

    // `buffered` keeps results in path order (so which copy of a duplicate is
    // "first" is deterministic) while running up to `concurrency` at once.
    let mut tasks = futures::stream::iter(new_track_paths)
        .map(|p| {
            tokio::task::spawn_blocking(move || -> Result<track::TrackInfo, BoxError> {
                let mut track = read_tags(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                track.file_hash = hash_file(&p);
                Ok(track)
            })
        })
        .buffered(concurrency.max(1));

    let mut seen_isrcs: HashSet<String> = HashSet::new();
    let mut seen_hashes: HashSet<String> = HashSet::new();
    let mut batch: Vec<track::TrackInfo> = Vec::with_capacity(COMMIT_BATCH);
    let mut processed = 0usize;

    while let Some(result) = tasks.next().await {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::info!("scan cancelled by user");
            break;
        }
        processed += 1;
        scan_sender.try_send(ScanEvent::Progress(processed, total)).ok();
        match result {
            Ok(Ok(mut track)) => {
                let isrc_seen = track
                    .isrc
                    .as_ref()
                    .is_some_and(|i| existing_isrcs.contains(i) || seen_isrcs.contains(i));
                let hash_seen = track
                    .file_hash
                    .as_ref()
                    .is_some_and(|h| existing_hashes.contains(h) || seen_hashes.contains(h));
                if isrc_seen || hash_seen {
                    track.status = "duplicate".to_string();
                }
                // Record keys even for duplicates so a third copy is caught too.
                if let Some(i) = &track.isrc {
                    seen_isrcs.insert(i.clone());
                }
                if let Some(h) = &track.file_hash {
                    seen_hashes.insert(h.clone());
                }
                batch.push(track);
                if batch.len() >= COMMIT_BATCH {
                    flush_batch(pool, library_id, &mut batch).await?;
                }
            }
            Ok(Err(e)) => tracing::warn!(error = %e, "failed to read tags"),
            Err(e) => tracing::error!(error = %e, "tag-read task panicked"),
        }
    }

    flush_batch(pool, library_id, &mut batch).await?;
    Ok(())
}

/// Writes a batch of scanned tracks in one short transaction. A row that
/// already exists (another scan got there first) is skipped, not fatal.
async fn flush_batch(
    pool: &SqlitePool,
    library_id: i64,
    batch: &mut Vec<track::TrackInfo>,
) -> Result<(), sqlx::Error> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for track in batch.drain(..) {
        match db::insert_track(&mut tx, &track, library_id).await {
            Ok(_) => {}
            Err(e) if db::is_unique_violation(&e) => {
                tracing::debug!(path = %track.file_path.display(), "already imported; skipping");
            }
            Err(e) => return Err(e),
        }
    }
    tx.commit().await
}

pub enum ScanEvent {
    Progress(usize, usize), // processed, total
    Done,
    Error(String),
}

/// Classifies a single file's integrity. `None` means healthy.
fn classify_health(
    path: &Path,
    stored_hash: &Option<String>,
    bitrate: Option<u32>,
    low_bitrate_threshold: u32,
) -> Option<String> {
    if !path.exists() {
        return Some("missing_file".to_string());
    }
    match std::fs::metadata(path) {
        Ok(m) if m.len() == 0 => return Some("zero_byte".to_string()),
        Err(_) => return Some("missing_file".to_string()),
        _ => {}
    }
    if read_tags(path).is_err() {
        return Some("decode_error".to_string());
    }
    if let Some(stored) = stored_hash
        && let Some(current) = hash_file(path)
            && &current != stored {
                return Some("hash_mismatch".to_string());
            }
    if let Some(b) = bitrate
        && b > 0 && b < low_bitrate_threshold {
            return Some("low_bitrate".to_string());
        }
    None
}

/// Scans every track for integrity problems and records them in `health_issue`.
/// Re-hashes files, so it is intentionally an explicit, cancellable operation.
pub async fn health_check(
    pool: SqlitePool,
    library_id: i64,
    low_bitrate_threshold: u32,
    concurrency: usize,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    scan_sender: tokio::sync::mpsc::Sender<ScanEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tracks = db::load_tracks(&pool, None, None, None, library_id).await?;
    let total = tracks.len();
    let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut tasks = FuturesUnordered::new();

    for t in tracks {
        let Some(id) = t.id else { continue };
        let path = t.file_path.clone();
        let hash = t.file_hash.clone();
        let bitrate = t.bitrate;
        let sem = Arc::clone(&semaphore);
        tasks.push(async move {
            let permit = sem.acquire().await.unwrap();
            let result = tokio::task::spawn_blocking(move || {
                classify_health(&path, &hash, bitrate, low_bitrate_threshold)
            })
            .await;
            drop(permit);
            (id, result)
        });
    }

    let mut processed = 0usize;
    while let Some((id, result)) = tasks.next().await {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        if let Ok(issue) = result {
            db::set_health_issue(&pool, id, issue.as_deref()).await.ok();
        }
        processed += 1;
        scan_sender
            .try_send(ScanEvent::Progress(processed, total))
            .ok();
    }
    scan_sender.send(ScanEvent::Done).await.ok();
    Ok(())
}

/// Re-reads tags for tracks whose file was modified since it was last scanned,
/// keeping DB metadata in sync with on-disk edits.
pub async fn rescan_changed(
    pool: SqlitePool,
    library_id: i64,
    concurrency: usize,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    scan_sender: tokio::sync::mpsc::Sender<ScanEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let candidates = db::tracks_for_rescan(&pool, library_id).await?;
    let total = candidates.len();
    let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut tasks = FuturesUnordered::new();

    for (id, path, last_scanned) in candidates {
        let sem = Arc::clone(&semaphore);
        tasks.push(async move {
            let permit = sem.acquire().await.unwrap();
            let result = tokio::task::spawn_blocking(move || {
                // Only re-read when the file's mtime is newer than last scan.
                let changed = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|mt| mt.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64 > last_scanned)
                    .unwrap_or(false);
                if changed {
                    read_tags(&path).ok().map(|mut t| {
                        t.file_hash = hash_file(&path);
                        t
                    })
                } else {
                    None
                }
            })
            .await;
            drop(permit);
            (id, result)
        });
    }

    let mut processed = 0usize;
    let mut updated = 0usize;
    while let Some((id, result)) = tasks.next().await {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        if let Ok(Some(track)) = result {
            db::update_track_metadata(&pool, id, &track).await.ok();
            updated += 1;
        }
        processed += 1;
        scan_sender
            .try_send(ScanEvent::Progress(processed, total))
            .ok();
    }
    tracing::info!(updated, "incremental rescan complete");
    scan_sender.send(ScanEvent::Done).await.ok();
    Ok(())
}

/// Builds a recursive filesystem watcher over `path` that pings `tx` whenever a
/// file is created or modified. The returned watcher must be kept alive.
pub fn watch_dir(
    path: &Path,
    tx: tokio::sync::mpsc::Sender<()>,
) -> notify::Result<notify::RecommendedWatcher> {
    use notify::{Event, EventKind, RecursiveMode, Watcher};

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res
            && matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                // Coalesce: a full channel already has a pending signal.
                let _ = tx.try_send(());
            }
    })?;
    watcher.watch(path, RecursiveMode::Recursive)?;
    Ok(watcher)
}

pub enum ValidateEvent {
    Done,
    Error(String),
}

/// Flags tracks whose file is gone as `missing`, and puts previously-missing
/// tracks whose file is back into the queue again. Always answers on
/// `scan_sender`, with an error message if the check couldn't run.
pub async fn validate_paths(
    pool: SqlitePool,
    library_id: i64,
    concurrency: usize,
    scan_sender: tokio::sync::oneshot::Sender<ValidateEvent>,
) -> Result<(), BoxError> {
    let result = validate_paths_inner(&pool, library_id, concurrency).await;
    let event = match &result {
        Ok(()) => ValidateEvent::Done,
        Err(e) => ValidateEvent::Error(format!("Validation failed: {e}")),
    };
    scan_sender.send(event).ok();
    result
}

async fn validate_paths_inner(
    pool: &SqlitePool,
    library_id: i64,
    concurrency: usize,
) -> Result<(), BoxError> {
    let paths_to_check = db::load_tracks_paths(pool, library_id).await?;
    let mut checks = futures::stream::iter(paths_to_check)
        .map(|(id, path)| tokio::task::spawn_blocking(move || (id, path.exists())))
        .buffer_unordered(concurrency.max(1));

    let mut missing = Vec::new();
    let mut present = Vec::new();
    while let Some(result) = checks.next().await {
        if let Ok((id, exists)) = result {
            if exists { present.push(id) } else { missing.push((id, "missing")) }
        }
    }
    db::set_status_many(pool, &missing).await?;
    db::clear_missing(pool, &present).await?;
    Ok(())
}
