mod api;
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

const USAGE: &str = "\
preamble - a terminal music library manager

Usage: preamble [OPTIONS]

Options:
  -p, --path <PATH>  Library directory to open (or create). Defaults to the
                     library picker, or the configured library_path
  -v, --version      Print version and exit
  -h, --help         Print this help and exit";

/// Parsed command line.
#[derive(Debug)]
struct Cli {
    path: Option<PathBuf>,
}

/// Parses `args` (excluding the program name).
///
/// `Ok(None)` means the flag was fully handled by printing (`--help`,
/// `--version`) and the process should exit successfully. `Err` carries a
/// message describing what was wrong with the invocation.
fn parse_args(args: &[String]) -> Result<Option<Cli>, String> {
    let mut path: Option<PathBuf> = None;
    let mut args = args.iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-v" | "--version" => {
                println!("preamble {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-p" | "--path" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("'{arg}' requires a directory argument"))?;
                path = Some(PathBuf::from(value));
            }
            // --path=/music is the same as --path /music.
            _ if arg.starts_with("--path=") => {
                let value = arg.trim_start_matches("--path=");
                if value.is_empty() {
                    return Err("'--path' requires a directory argument".into());
                }
                path = Some(PathBuf::from(value));
            }
            // A bare path used to be accepted here; it now needs --path so the
            // argument's meaning is explicit.
            _ if !arg.starts_with('-') => {
                return Err(format!(
                    "unexpected argument '{arg}'; pass a library with '--path {arg}'"
                ));
            }
            _ => return Err(format!("unknown option '{arg}'")),
        }
    }

    Ok(Some(Cli { path }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = env::args().skip(1).collect();
    // Parsed before any setup so --version/--help stay instant and side-effect
    // free (no DB, no log file).
    let cli = match parse_args(&args) {
        Ok(Some(cli)) => cli,
        Ok(None) => return Ok(()),
        Err(message) => {
            eprintln!("preamble: {message}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let config = Config::load();

    // Keep the guard alive until main returns so buffered logs are flushed.
    let _log_guard = config::init_logging(&config);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "preamble starting");

    let pool = db::init_db().await?;

    // Start the HTTP API in the background if PREAMBLE_API_PORT is set.
    let _api_handle = if let Some(port) = api::port_from_env() {
        Some(api::start(pool.clone(), port).await?)
    } else {
        None
    };

    // Migrate any pre-multi-library rows into a default library before anything
    // queries by library_id.
    db::ensure_default_library(&pool, config.library_path.as_deref()).await?;

    // --path selects (or creates) a library directly; otherwise we show the
    // picker. The config's library_path only seeds the default library's name
    // during the one-time backfill above.
    let mut app = App::new(pool, config).await?;
    resolve_startup(&mut app, cli.path).await?;
    run_app(app).await?;

    Ok(())
}

/// Decides which screen the app opens on: an existing library (CLI path matches),
/// the create form (CLI path is new, or there are no libraries yet), or the
/// picker (no path given but libraries exist).
async fn resolve_startup(
    app: &mut App,
    cli_path: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cli_path {
        Some(p) => {
            let path_str = p.to_string_lossy().into_owned();
            match db::find_library_by_path(&app.pool, &path_str).await? {
                Some(lib) => app.open_library(lib).await?,
                None => {
                    // Unknown path: pre-fill the create form and let the user
                    // confirm a name.
                    app.new_lib_name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    app.new_lib_path = path_str;
                    app.new_lib_focus = crate::app::NewLibField::Name;
                    app.current_screen = crate::app::Screens::CreateLibrary;
                }
            }
        }
        None => {
            app.refresh_libraries().await?;
            app.current_screen = if app.libraries.is_empty() {
                crate::app::Screens::CreateLibrary
            } else {
                crate::app::Screens::Picker
            };
        }
    }
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

    // Start the filesystem watcher if enabled in config and a library is open.
    if app.config.watch && app.active_library.is_some() {
        app.toggle_watch();
    }

    loop {
        // While an operation animates (gauge/spinner) keep advancing the tick
        // and repainting; otherwise only repaint when something changed.
        if app.is_busy() {
            app.spinner_tick = app.spinner_tick.wrapping_add(1);
            app.needs_redraw = true;
        }
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

        // Drains input (sets needs_redraw on any event) then applies any pending
        // lazy tab reload before we decide whether to paint.
        ui::poll_events(&mut app).await?;
        app.ensure_fresh().await?;

        if app.needs_redraw {
            term.draw(|f| ui::draw(f, &mut app))?;
            app.needs_redraw = false;
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Cli>, String> {
        let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        parse_args(&owned)
    }

    #[test]
    fn no_args_leaves_path_unset() {
        let cli = parse(&[]).unwrap().unwrap();
        assert!(cli.path.is_none());
    }

    #[test]
    fn path_flag_accepts_both_forms() {
        for args in [vec!["--path", "/music"], vec!["-p", "/music"], vec!["--path=/music"]] {
            let cli = parse(&args).unwrap().unwrap();
            assert_eq!(cli.path, Some(PathBuf::from("/music")), "{args:?}");
        }
    }

    #[test]
    fn help_and_version_stop_before_startup() {
        for flag in ["-h", "--help", "-v", "--version"] {
            assert!(parse(&[flag]).unwrap().is_none(), "{flag}");
        }
    }

    #[test]
    fn path_without_value_is_an_error() {
        assert!(parse(&["--path"]).is_err());
        assert!(parse(&["--path="]).is_err());
    }

    #[test]
    fn bare_path_is_rejected_with_a_hint() {
        let err = parse(&["/music"]).unwrap_err();
        assert!(err.contains("--path /music"), "{err}");
    }

    #[test]
    fn unknown_option_is_an_error() {
        assert!(parse(&["--nope"]).is_err());
    }
}
