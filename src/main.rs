mod db;
mod reader;
mod track;
mod app;
mod ui;

use crate::app::App;
use crate::track::{hash_file, read_tags};
use reader::collect_new_paths;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::{env, io};

use std::sync::Arc;
use tokio::sync::Semaphore;

use ratatui::{backend::CrosstermBackend, Terminal};


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let path = Path::new(&args[1]);
    let pool = db::init_db().await?;
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
    collect_new_paths(path, &mut new_track_paths, &existing_tracks)?;

    if !new_track_paths.is_empty() {
        println!("New tracks found!");
        
        // threaded tag reading and hashing of all new paths 
        let tasks: Vec<_> = new_track_paths
        .into_iter()
        .map(|p| {
            let sem = Arc::clone(&semaphore);
            async move {
                let permit = sem.acquire().await.unwrap();
                
                let result = tokio::task::spawn_blocking(move || -> Result<track::TrackInfo, Box<dyn std::error::Error + Send + Sync>> {
                    let mut track = read_tags(&p)?;
                    track.file_hash = hash_file(&p);
                    Ok(track)
                }).await;
                drop(permit);
                result
            }
        })
        .collect();
    
        let now = std::time::Instant::now();
        let results = futures::future::join_all(tasks).await;
        println!("Reading took: {:?}", now.elapsed());
        
        let now = std::time::Instant::now();

        let mut seen_isrcs: HashSet<String> = HashSet::new();
        let mut seen_hashes: HashSet<String> = HashSet::new();
        
        for result in results {
            match result {
                Ok(Ok(mut track)) => {
                    // check duplication, and set status based on that
                    if track.isrc.as_ref().map(|i| existing_isrcs.contains(i)).unwrap_or(false)
                    || track.file_hash.as_ref().map(|h| existing_hashes.contains(h)).unwrap_or(false) 
                    || track.isrc.as_ref().map(|i| seen_isrcs.contains(i)).unwrap_or(false) 
                    || track.file_hash.as_ref().map(|h| seen_hashes.contains(h)).unwrap_or(false) {
                        track.status = "duplicate".to_string();
                    } else {
                        track.isrc.as_deref().map(|i| seen_isrcs.insert(i.to_string()));
                        track.file_hash.as_deref().map(|h| seen_hashes.insert(h.to_string()));
                    }
                    db::insert_track(&mut tx, &track).await?;
                }
                Ok(Err(e)) => eprintln!("Failed to read tags: {:?}", e),
                Err(e) => eprintln!("Task panicked: {:?}", e),
            }
        }
        tx.commit().await?;
        println!("Inserting took: {:?}", now.elapsed());
    }
    
    let app = App::new(pool).await?;
    run_app(app).await?;

    Ok(())
}

pub async fn run_app(mut app: App) -> Result<(), Box<dyn std::error::Error>> {
    // enter raw drawing mode and open alternate "new" screen
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut term = Terminal::new(backend)?;
    
    if !app.library_tracks.is_empty() { app.library_state.select(Some(0)) };
    if !app.pending_tracks.is_empty() { app.pending_state.select(Some(0)) };

    loop {
        term.draw(|f| ui::draw(f, &mut app))?;

        ui::poll_events(&mut app)?;

        if app.should_quit { break; }
    }

    // exit raw mode and remove alternate "new" screen
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;

    Ok(())
}