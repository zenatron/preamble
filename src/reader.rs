// src/reader.rs

use std::collections::HashSet;
use std::fs::{self};
use std::path::{Path, PathBuf};

use crate::{db, track};
use crate::track::{hash_file, read_tags};

use std::sync::Arc;
use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;
use sqlx::{SqlitePool};

pub fn collect_new_paths(
    path: &Path,
    paths: &mut Vec<PathBuf>,
    existing: &HashSet<String>,
    formats: &HashSet<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_new_paths(&path, paths, existing, formats)?;
            } else {
                let ext_matches = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| formats.contains(&e.to_lowercase()))
                    .unwrap_or(false);
                if ext_matches && !existing.contains(path.to_str().unwrap_or("")) {
                    paths.push(path);
                }
            }
        }
    }
    Ok(())
}

pub async fn scan_library(
    pool: SqlitePool,
    path: PathBuf,
    formats: Vec<String>,
    concurrency: usize,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    scan_sender: tokio::sync::mpsc::Sender<ScanEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut tx = pool.clone().begin().await?;

    // Bound concurrent tag-reads/hashes; tuned via config (default 8).
    let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));

    let formats: HashSet<String> = formats.into_iter().collect();

    // loads existing track paths into a HashSet for fast lookup
    let existing_tracks = db::load_existing_paths(&pool).await?;

    // loads existing track isrcs from all tracks in DB
    let existing_isrcs = db::load_existing_isrcs(&pool).await?;

    // loads exsiting track hashes from all tracks in DB
    let existing_hashes = db::load_existing_hashes(&pool).await?;

    // fills a Vec of all new track paths, as compared to the HashSet
    let mut new_track_paths: Vec<PathBuf> = Vec::new();
    collect_new_paths(&path, &mut new_track_paths, &existing_tracks, &formats)?;

    //println!("There are {:?} existing tracks and {:?} new tracks!", existing_tracks.len(), new_track_paths.len());

    if !new_track_paths.is_empty() {
        //println!("New tracks found!");

        let total = new_track_paths.len();

        let mut processed = 0usize;
        let mut tasks = FuturesUnordered::new();

        // eprintln!("BEFORE SCAN: {:?}", std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH));
        for p in new_track_paths {
            let sem = Arc::clone(&semaphore);
            tasks.push(async move {
                let permit = sem.acquire().await.unwrap();
                
                let result = tokio::task::spawn_blocking(move || -> Result<track::TrackInfo, Box<dyn std::error::Error + Send + Sync>> {
                    let mut track = read_tags(&p)?;
                    track.file_hash = hash_file(&p);
                    Ok(track)
                }).await;
                drop(permit);
                result
            });
        }

        let mut seen_isrcs: HashSet<String> = HashSet::new();
        let mut seen_hashes: HashSet<String> = HashSet::new();

        while let Some(result) = tasks.next().await {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::info!("scan cancelled by user");
                break;
            }
            match result {
                Ok(Ok(mut track)) => {
                    // check duplication, and set status based on that
                    if track.isrc.as_ref().map(|i| existing_isrcs.contains(i)).unwrap_or(false)
                    || track.file_hash.as_ref().map(|h| existing_hashes.contains(h)).unwrap_or(false) 
                    || track.isrc.as_ref().map(|i| seen_isrcs.contains(i)).unwrap_or(false) 
                    || track.file_hash.as_ref().map(|h| seen_hashes.contains(h)).unwrap_or(false) {
                        //println!("Duplicate found: {:?}", track.file_path);
                        track.status = "duplicate".to_string();
                    } else {
                        track.isrc.as_deref().map(|i| seen_isrcs.insert(i.to_string()));
                        track.file_hash.as_deref().map(|h| seen_hashes.insert(h.to_string()));
                    }
                    db::insert_track(&mut tx, &track).await?;
                    processed += 1;
                    scan_sender.try_send(ScanEvent::Progress(processed, total)).ok();
                }
                Ok(Err(e)) => tracing::warn!(error = %e, "failed to read tags"),
                Err(e) => tracing::error!(error = %e, "tag-read task panicked"),
            }
        }
        
        tx.commit().await?;
        // eprintln!("AFTER SCAN: {:?}", std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH));
    }
    scan_sender.send(ScanEvent::Done).await.ok();
    Ok(())
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
    if let Some(stored) = stored_hash {
        if let Some(current) = hash_file(&path.to_path_buf()) {
            if &current != stored {
                return Some("hash_mismatch".to_string());
            }
        }
    }
    if let Some(b) = bitrate {
        if b > 0 && b < low_bitrate_threshold {
            return Some("low_bitrate".to_string());
        }
    }
    None
}

/// Scans every track for integrity problems and records them in `health_issue`.
/// Re-hashes files, so it is intentionally an explicit, cancellable operation.
pub async fn health_check(
    pool: SqlitePool,
    low_bitrate_threshold: u32,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    scan_sender: tokio::sync::mpsc::Sender<ScanEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tracks = db::load_tracks(&pool, None, None, None).await?;
    let total = tracks.len();
    let semaphore = Arc::new(Semaphore::new(16));
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
    concurrency: usize,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    scan_sender: tokio::sync::mpsc::Sender<ScanEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let candidates = db::tracks_for_rescan(&pool).await?;
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
        if let Ok(event) = res {
            if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                // Coalesce: a full channel already has a pending signal.
                let _ = tx.try_send(());
            }
        }
    })?;
    watcher.watch(path, RecursiveMode::Recursive)?;
    Ok(watcher)
}

pub enum ValidateEvent {
    Done,
    Error(String),
}

pub async fn validate_paths(pool: SqlitePool, scan_sender: tokio::sync::oneshot::Sender<ValidateEvent>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let paths_to_check = db::load_tracks_paths(&pool).await?;

    let semaphore = Arc::new(Semaphore::new(32));
    let mut tasks = FuturesUnordered::new();

    for (id, path) in paths_to_check {
        let sem = Arc::clone(&semaphore);
        tasks.push(async move {
            let permit = sem.acquire().await.unwrap();

            let result = tokio::task::spawn_blocking(move || (id, path.exists())).await;
            drop(permit);
            result
        })
    }

    while let Some(result) = tasks.next().await {
        match result {
            Ok((id, exists)) => {
                if !exists {
                    db::update_track_status(&pool, id, "missing").await?;
                }
            },
            _ => {},
        }
    }
    scan_sender.send(ValidateEvent::Done).ok();
    Ok(())
}