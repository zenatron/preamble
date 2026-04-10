mod app;
mod db;
mod formatters;
mod reader;
mod track;
mod ui;

use crate::app::App;
use crate::reader::ScanEvent;
use std::path::{PathBuf};
use std::{env, io};

use ratatui::{backend::CrosstermBackend, Terminal};


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = env::args().collect();
    
    let path: Option<PathBuf> = args.get(1).map(PathBuf::from);
    
    let pool = db::init_db().await?;
    
    // if let Some(ref p) = path {
    //     scan_library(&pool, p).await?;
    // }
    
    let app = App::new(pool, path).await?;
    run_app(app).await?;

    Ok(())
}

pub async fn run_app(mut app: App) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // enter raw drawing mode and open alternate "new" screen
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut term = Terminal::new(backend)?;
    
    // highlight the first element from each tab
    if !app.library_tracks.is_empty() { app.library_state.select(Some(0)) };
    if !app.pending_tracks.is_empty() { app.pending_state.select(Some(0)) };
    if !app.duplicate_tracks.is_empty() { app.duplicate_state.select(Some(0)) };

    loop {
        term.draw(|f| ui::draw(f, &mut app))?;

        if let Some(ref mut rx) = app.scan_receiver {
            match rx.try_recv() {
                Ok(ScanEvent::Progress(n, total)) => app.scan_progress = Some((n, total)),
                Ok(ScanEvent::Done) => {
                    App::reload(&mut app).await?;
                    app.scan_receiver = None;
                    app.scan_progress = None;
                    app.current_screen = app::Screens::Start;
                }
                _ => {}
            }
        }

        ui::poll_events(&mut app).await?;

        if app.should_quit { break; }
    }

    // exit raw mode and remove alternate "new" screen
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;

    Ok(())
}