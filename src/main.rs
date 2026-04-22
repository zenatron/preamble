mod app;
mod db;
mod formatters;
mod reader;
mod track;
mod ui;

use crate::app::App;
use crate::reader::ScanEvent;
use std::path::PathBuf;
use std::{env, io};

use ratatui::{Terminal, backend::CrosstermBackend};

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

    for tab in &mut app.tabs {
        if !tab.tracks.is_empty() {
            tab.state.select(Some(0));
        }
    }

    loop {
        term.draw(|f| ui::draw(f, &mut app))?;

        app.spinner_tick = app.spinner_tick.wrapping_add(1);

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

        ui::poll_events(&mut app).await?;

        if app.should_quit {
            break;
        }
    }

    // exit raw mode and remove alternate "new" screen
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;

    Ok(())
}
