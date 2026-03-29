mod db;
mod reader;
mod track;
mod app;

use crate::app::App;
use crate::track::{hash_file, read_tags};
use ratatui::style::{Style, Stylize};
use reader::collect_paths;
use std::fmt::format;
use std::path::{Path, PathBuf};
use std::{env, io, time::Duration};

use std::sync::Arc;
use tokio::sync::Semaphore;

use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
use crossterm::event::{self, Event, KeyCode};
use ratatui::{backend::CrosstermBackend, Terminal};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let path = Path::new(&args[1]);
    let pool = db::init_db().await?;
    let mut tx = pool.clone().begin().await?;

    // create semaphore with 32 concurrent threads. this seems to be optimal
    let semaphore = Arc::new(Semaphore::new(32));

    // loads existing tracks into a HashSet for fast lookup
    let existing_tracks = db::load_existing_paths(&pool).await?;

    // fills a Vec of all new track paths, as compared to the HashSet
    let mut new_track_paths: Vec<PathBuf> = Vec::new();
    collect_paths(path, &mut new_track_paths, &existing_tracks)?;

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
    }
    
    let app = App::new(pool).await?;
    run_app(app).await?;

    // exit raw mode and remove alternate "new" screen
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    Ok(())
}

pub async fn run_app(mut app: App) -> Result<(), Box<dyn std::error::Error>> {
    // enter raw drawing mode and open alternate "new" screen
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut term = Terminal::new(backend)?;
    
    if !app.tracks.is_empty() { app.list_state.select(Some(0)) };

    loop {
        term.draw(|f| {
            let area = f.area();
            // f.render_widget(Paragraph::new("preamble"), area);
            
            let list_items = app.tracks.iter().map(|i| {
                ListItem::new(format!("{} - {}", i.artist.as_deref().unwrap_or("Unknown"), i.title.as_deref().unwrap_or("Unknown")))
            });

            let list = List::new(list_items)
                .block(Block::default()
                .title(format!("Tracks: [{}]/[{}]", app.list_state.selected().unwrap_or(0) + 1, app.tracks.len()))
                .borders(Borders::ALL))
                .highlight_style(Style::default().reversed());

            f.render_stateful_widget(list, area, &mut app.list_state);
        })?;

        if crossterm::event::poll(Duration::from_millis(16))? {
            match crossterm::event::read()? {
                Event::Key(key) => {
                    if key.code == KeyCode::Char('q') {
                        app.should_quit = true;
                    }
                    if key.code == KeyCode::Up {
                        app.list_state.select_previous();
                    }
                    if key.code == KeyCode::Down {
                        app.list_state.select_next();
                    }
                },
                _ => {},
            }
        }
        if app.should_quit { break; }
    }

    Ok(())
}