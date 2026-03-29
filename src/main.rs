mod db;
mod reader;
mod track;

use crate::track::read_tags;
use reader::collect_paths;
use std::env;
use std::path::{Path, PathBuf};

use std::sync::Arc;
use tokio::sync::Semaphore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let path = Path::new(&args[1]);
    let pool = db::init_db().await?;
    let mut tx = pool.begin().await?;

    // create semaphore with 32 concurrent threads. this seems to be optimal
    let semaphore = Arc::new(Semaphore::new(32));

    // loads existing tracks into a HashSet for fast lookup
    let existing_tracks = db::load_existing_paths(&pool).await?;

    // fills a Vec of all new track paths, as compared to the HashSet
    let mut new_track_paths: Vec<PathBuf> = Vec::new();
    collect_paths(path, &mut new_track_paths, &existing_tracks)?;

    if new_track_paths.is_empty() {
        println!("No new tracks found.");
        return Ok(());
    }

    let tasks: Vec<_> = new_track_paths
        .into_iter()
        .map(|p| {
            let sem = Arc::clone(&semaphore);
            async move {
                let permit = sem.acquire().await.unwrap();
                let result = tokio::task::spawn_blocking(move || read_tags(&p)).await;
                drop(permit);
                result
            }
        })
        .collect();

    let now = std::time::Instant::now();
    let results = futures::future::join_all(tasks).await;
    println!("Reading took: {:?}", now.elapsed());

    let now = std::time::Instant::now();

    for result in results {
        match result {
            Ok(Ok(track)) => {
                db::insert_track(&mut tx, &track).await?;
            }
            Ok(Err(e)) => eprintln!("Failed to read tags: {:?}", e),
            Err(e) => eprintln!("Task panicked: {:?}", e),
        }
    }
    tx.commit().await?;
    println!("Inserting took: {:?}", now.elapsed());

    Ok(())
}
