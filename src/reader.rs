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
use tokio::sync::mpsc::Sender;

pub fn collect_new_paths(
    path: &Path,
    paths: &mut Vec<PathBuf>,
    existing: &HashSet<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_new_paths(&path, paths, existing)?;
            } else {
                if path.extension().and_then(|e| e.to_str()) == Some("flac") {
                    if !existing.contains(path.to_str().unwrap_or("")) {
                        paths.push(path);
                    }
                }
            }
        }
    }
    Ok(())
}

pub async fn scan_library(
    pool: SqlitePool,
    path: PathBuf,
    scan_sender: Sender<ScanEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut tx = pool.clone().begin().await?;

    // create semaphore with 32 concurrent threads. this seems to be optimal
    let semaphore = Arc::new(Semaphore::new(32));

    // loads existing track paths into a HashSet for fast lookup
    let existing_tracks = db::load_existing_paths(&pool).await?;

    // loads existing track isrcs from all tracks in DB
    let existing_isrcs = db::load_existing_isrcs(&pool).await?;

    // loads exsiting track hashes from all tracks in DB
    let existing_hashes = db::load_existing_hashes(&pool).await?;

    // fills a Vec of all new track paths, as compared to the HashSet
    let mut new_track_paths: Vec<PathBuf> = Vec::new();
    collect_new_paths(&path, &mut new_track_paths, &existing_tracks)?;

    //println!("There are {:?} existing tracks and {:?} new tracks!", existing_tracks.len(), new_track_paths.len());

    if !new_track_paths.is_empty() {
        //println!("New tracks found!");

        let total = new_track_paths.len();

        let mut processed = 0usize;
        let mut tasks = FuturesUnordered::new();

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
                    scan_sender.send(ScanEvent::Progress(processed, total)).await.ok();
                }
                Ok(Err(e)) => eprintln!("Failed to read tags: {:?}", e),
                Err(e) => eprintln!("Task panicked: {:?}", e),
            }
        }
        
        tx.commit().await?;
    }
    scan_sender.send(ScanEvent::Done).await.ok();
    Ok(())
}

pub enum ScanEvent {
    Progress(usize, usize), // processed, total
    Done,
    Error(String),
}