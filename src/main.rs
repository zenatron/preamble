mod app;
mod config;
mod db;
mod enrich;
mod export;
mod formatters;
mod reader;
mod track;
mod ui;
mod undo;

use crate::app::App;
use crate::config::Config;
use crate::reader::ScanEvent;
use std::path::PathBuf;
use std::{env, io};

use ratatui::{Terminal, backend::CrosstermBackend};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = env::args().collect();

    let config = Config::load();

    // Keep the guard alive until main returns so buffered logs are flushed.
    let _log_guard = config::init_logging(&config);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "preamble starting");

    // A path given on the CLI overrides the configured default library path.
    let path: Option<PathBuf> = args
        .get(1)
        .map(PathBuf::from)
        .or_else(|| config.library_path.clone());

    let pool = db::init_db().await?;

    let app = App::new(pool, path, config).await?;
    run_app(app).await?;

    Ok(())
}

pub async fn run_app(mut app: App) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // enter raw drawing mode and open alternate "new" screen
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut term = Terminal::new(backend)?;

    // highlight the first element from each tab

    for tab in &mut app.tabs {
        if !tab.tracks.is_empty() {
            tab.state.select(Some(0));
        }
    }

    // Start the filesystem watcher if enabled in config.
    if app.config.watch {
        app.toggle_watch();
    }

    loop {
        term.draw(|f| ui::draw(f, &mut app))?;

        app.spinner_tick = app.spinner_tick.wrapping_add(1);
        app.expire_status();

        if let Some(ref mut rx) = app.scan_receiver {
            match rx.try_recv() {
                Ok(ScanEvent::Progress(n, total)) => app.scan_progress = Some((n, total)),
                Ok(ScanEvent::Done) => {
                    app.reload().await?;
                    app.scan_receiver = None;
                    app.scan_progress = None;
                    app.current_screen = app::Screens::Start;
                }
                _ => {}
            }
        }

        if let Some(ref mut rx) = app.validating_receiver {
            if let Ok(_) = rx.try_recv() {
                app.reload().await?;
                app.validating_receiver = None;
                app.is_validating = false;
            }
        }

        if let Some(ref mut rx) = app.enrich_receiver {
            match rx.try_recv() {
                Ok(crate::enrich::EnrichEvent::Progress(n, total)) => {
                    app.enrich_progress = Some((n, total));
                }
                Ok(crate::enrich::EnrichEvent::Error(msg)) => {
                    app.set_status(crate::app::StatusLevel::Warning, msg);
                }
                Ok(crate::enrich::EnrichEvent::Done) => {
                    app.reload().await?;
                    app.enrich_receiver = None;
                    app.enrich_progress = None;
                    app.is_enriching = false;
                }
                _ => {}
            }
        }

        app.commit_search_if_due().await;
        app.tick_watch().await;

        ui::poll_events(&mut app).await?;

        if app.quit_confirmed {
            break;
        }
    }

    // exit raw mode and remove alternate "new" screen
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen
    )?;

    Ok(())
}
