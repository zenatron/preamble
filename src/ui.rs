use crate::app::{self, App};
use crate::db::{self, load_track_full};
use crate::formatters::{format_thou, format_track_duration};
use crate::reader::{ScanEvent, ValidateEvent, scan_library, validate_paths};

use crossterm::event::{Event, KeyCode, KeyEvent};
use humansize::{DECIMAL, format_size};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    match app.current_screen {
        app::Screens::Start => {
            draw_start_screen(f, app, area);
        }
        app::Screens::Main => {
            draw_main_screen(f, app, area);
        }
        app::Screens::Scanning => {
            draw_scanning_screen(f, app, area);
        }
    }
}

fn draw_start_screen(f: &mut Frame, app: &mut App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Fill(1),   // top padding
        Constraint::Length(6), // title
        Constraint::Length(1), // version
        Constraint::Length(5), // stats
        Constraint::Length(2), // path
        Constraint::Length(4), // tooltips
        Constraint::Fill(1),   // bottom padding
    ])
    .split(area);

    let title = ratatui::text::Text::from(vec![
        Line::from("                                _     _      "),
        Line::from(" _ __  _ __ ___  __ _ _ __ ___ | |__ | | ___ "),
        Line::from("| '_ \\| '__/ _ \\/ _` | '_ ` _ \\| '_ \\| |/ _ \\"),
        Line::from("| |_) | | |  __/ (_| | | | | | | |_) | |  __/"),
        Line::from("| .__/|_| \\___/\\___|_|_| |_| |_|_.__/|_|\\___|"),
        Line::from("|_|                                          "),
    ])
    .light_red();

    f.render_widget(
        title.alignment(ratatui::layout::Alignment::Center),
        sections[1],
    );
    f.render_widget(
        Paragraph::new("v0.1.0")
            .alignment(ratatui::layout::Alignment::Center)
            .blue(),
        sections[2],
    );
    let stats = ratatui::text::Text::from(vec![
        Line::from(vec![
            Span::raw("Total Tracks: "),
            Span::styled(
                format_thou(app.library_stats.total_tracks),
                Style::default().white().bold(),
            ),
        ]),
        Line::from(vec![
            Span::raw("Pending Enrichment: "),
            Span::styled(
                format_thou(app.library_stats.total_pending),
                Style::default().yellow().bold(),
            ),
        ]),
        Line::from(vec![
            Span::raw("Duplicate Tracks: "),
            Span::styled(
                format_thou(app.library_stats.total_duplicates),
                Style::default().light_red().bold(),
            ),
        ]),
        Line::from(vec![
            Span::raw("Missing Tracks: "),
            Span::styled(
                format_thou(app.library_stats.total_missing),
                Style::default().light_red().bold(),
            ),
        ]),
    ]);
    f.render_widget(
        Paragraph::new(stats).alignment(ratatui::layout::Alignment::Center),
        sections[3],
    );
    f.render_widget(
        Paragraph::new(format!("Path to Scan: {:?}", app.pending_scan_path))
            .alignment(ratatui::layout::Alignment::Center)
            .light_cyan(),
        sections[4],
    );
    f.render_widget(
        Paragraph::new(format!(
            "[s] : Scan Library (updates any newly added tracks)\n[r] : Fresh Scan (WARNING: completely rebuilds database)\n[v] : Validate Paths\n[enter] : View Library\n[q] : Quit"
        )).style(Style::new().add_modifier(Modifier::BOLD))
        .alignment(ratatui::layout::Alignment::Center),
        sections[5],
    );

    // check if we are validating, then render the popup
    if app.is_validating {
        draw_spinner_popup(f, app, area);
    }
}

fn draw_spinner_popup(f: &mut Frame, app: &mut App, area: Rect) {
    let outer = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(9), // pane height
        Constraint::Fill(1),
    ])
    .split(area);

    let inner = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(27), // pane width
        Constraint::Fill(1),
    ])
    .split(outer[1]);

    f.render_widget(ratatui::widgets::Clear, inner[1]);

    let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let spinner = frames[app.spinner_tick % frames.len()];

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Validating")
        .yellow();
    let text = Paragraph::new(vec![
        Line::raw(""),
        Line::raw(""),
        Line::raw(""),
        Line::from(format!("{} Checking paths...", spinner)).light_yellow(),
    ])
    .alignment(ratatui::layout::Alignment::Center)
    .centered()
    .block(block);
    f.render_widget(text, inner[1]);
}

fn draw_main_screen(f: &mut Frame, app: &mut App, area: Rect) {
    let area = if matches!(app.current_screen, app::Screens::Main) && app.properties_panel_open {
        // create split here for properties panel
        let sections = Layout::horizontal([Constraint::Fill(3), Constraint::Fill(1)]).split(area);
        draw_properties_panel(f, app, sections[1]);
        sections[0]
    } else {
        area
    };

    let sections = Layout::vertical([
        Constraint::Length(3), // tab bar
        Constraint::Length(1), // shortcuts text
        Constraint::Length(3), // search bar
        Constraint::Min(0),
    ])
    .split(area);

    draw_tab_bar(f, app, sections[0]);
    draw_shortcuts_row(f, sections[1]);
    draw_search_bar(f, app, sections[2]);
    draw_table_content(f, app, sections[3]);

    if app.pending_delete {
        draw_delete_prompt(f, area);
    }
}

fn draw_scanning_screen(f: &mut Frame, app: &mut App, area: Rect) {
    let outer = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(5), // gauge height
        Constraint::Fill(1),
    ])
    .split(area);

    let inner = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(50), // gauge width
        Constraint::Fill(1),
    ])
    .split(outer[1]);

    let (n, total) = app.scan_progress.unwrap_or((0, 1));
    let ratio = n as f64 / total as f64;
    let gauge = Gauge::default()
        .block(
            Block::default()
                .title("Scanning...")
                .borders(Borders::ALL)
                .light_green(),
        )
        .gauge_style(
            Style::default()
                .light_green()
                .fg(ratatui::style::Color::Black),
        )
        .ratio(ratio)
        .label(format!("[{}/{}]", n, total));

    f.render_widget(gauge, inner[1]);
}

pub async fn poll_events(app: &mut App) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // poll at 15 FPS
    if crossterm::event::poll(std::time::Duration::from_millis(1000 / 15))? {
        match crossterm::event::read()? {
            Event::Key(key) => {
                if key.code == KeyCode::Char('q') && !app.search_mode {
                    app.should_quit = true;
                }

                match app.current_screen {
                    app::Screens::Start => {
                        handle_start_navigation(app, key).await;
                    }

                    app::Screens::Main => {
                        handle_main_navigation(app, key).await;
                    }
                    app::Screens::Scanning => {}
                };
            }
            _ => {}
        }
    }
    Ok(())
}

async fn handle_start_navigation(app: &mut App, key: KeyEvent) {
    if key.code == KeyCode::Char('s') {
        if let Some(ref path) = app.pending_scan_path {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            app.scan_receiver = Some(rx);
            tokio::spawn(scan_library(app.pool.clone(), path.clone(), tx));
            app.current_screen = app::Screens::Scanning;
        } else {
            app.status_message = Some("Path not provided.".to_string());
        }
    }

    if key.code == KeyCode::Char('r') {
        if let Some(ref path) = app.pending_scan_path {
            db::truncate_tracks(&app.pool).await.ok();
            let (tx, rx) = tokio::sync::mpsc::channel::<ScanEvent>(100);
            app.scan_receiver = Some(rx);
            tokio::spawn(scan_library(app.pool.clone(), path.clone(), tx));
            app.current_screen = app::Screens::Scanning;
        } else {
            app.status_message = Some("Path not provided.".to_string());
        }
    }

    if key.code == KeyCode::Char('v') {
        let (tx, rx) = tokio::sync::oneshot::channel::<ValidateEvent>();
        app.is_validating = true;
        app.validating_receiver = Some(rx);
        tokio::spawn(validate_paths(app.pool.clone(), tx));
    }

    if key.code == KeyCode::Enter {
        app.current_screen = app::Screens::Main;
    }
}

async fn handle_search_query(app: &mut App) {
    match app.current_tab {
        app::Tabs::Library => {
            app.library_tracks = db::load_tracks(&app.pool, None, None, Some(&app.search_query))
                .await
                .unwrap_or_default();
            if !app.library_tracks.is_empty() {
                app.library_state.select(Some(0));
            } else {
                app.library_state.select(None);
            }
        }
        app::Tabs::Enrichment => {
            app.pending_tracks =
                db::load_tracks(&app.pool, None, Some("pending"), Some(&app.search_query))
                    .await
                    .unwrap_or_default();
            if !app.pending_tracks.is_empty() {
                app.pending_state.select(Some(0));
            } else {
                app.pending_state.select(None);
            }
        }
        app::Tabs::Duplicates => {
            app.duplicate_tracks =
                db::load_tracks(&app.pool, None, Some("duplicate"), Some(&app.search_query))
                    .await
                    .unwrap_or_default();
            if !app.duplicate_tracks.is_empty() {
                app.duplicate_state.select(Some(0));
            } else {
                app.duplicate_state.select(None);
            }
        }
        app::Tabs::Missing => {
            app.missing_tracks =
                db::load_tracks(&app.pool, None, Some("missing"), Some(&app.search_query))
                    .await
                    .unwrap_or_default();
            if !app.missing_tracks.is_empty() {
                app.missing_state.select(Some(0));
            } else {
                app.missing_state.select(None);
            }
        }
    }
}

async fn handle_main_navigation(app: &mut App, key: KeyEvent) {
    // handle searching
    if key.code == KeyCode::Char('/') {
        app.search_mode = true;
    }
    {
        if app.search_mode {
            if key.code == KeyCode::Backspace {
                app.search_query.pop();
                handle_search_query(app).await;
            }
            if key.code == KeyCode::Esc {
                app.search_mode = false;
                app.search_query = String::new();
                App::reload(app).await.ok();
            }
            if key.code == KeyCode::Enter {
                app.search_mode = false;
                handle_search_query(app).await;
            }
            if let KeyCode::Char(c) = key.code {
                app.search_query.push(c);
                handle_search_query(app).await;
            }
            return;
        }
    }

    if key.code == KeyCode::Char('p') {
        app.properties_panel_open = true;
        load_selected_track(app).await;
    }
    if key.code == KeyCode::Up {
        // highlight previous track
        match app.current_tab {
            app::Tabs::Library => {
                if app.library_state.selected().is_some() {
                    app.library_state.select_previous();
                }
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
            app::Tabs::Enrichment => {
                if app.pending_state.selected().is_some() {
                    app.pending_state.select_previous();
                }
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
            app::Tabs::Duplicates => {
                if app.duplicate_state.selected().is_some() {
                    app.duplicate_state.select_previous();
                }
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
            app::Tabs::Missing => {
                if app.missing_state.selected().is_some() {
                    app.missing_state.select_previous();
                }
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
        }
    }
    if key.code == KeyCode::Down {
        // highlight next track
        match app.current_tab {
            app::Tabs::Library => {
                if app.library_state.selected().is_some() {
                    app.library_state.select_next();
                }
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
            app::Tabs::Enrichment => {
                if app.pending_state.selected().is_some() {
                    app.pending_state.select_next();
                }
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
            app::Tabs::Duplicates => {
                if app.duplicate_state.selected().is_some() {
                    app.duplicate_state.select_next();
                }
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
            app::Tabs::Missing => {
                if app.missing_state.selected().is_some() {
                    app.missing_state.select_next();
                }
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
        }
    }
    if key.code == KeyCode::Tab {
        app.pending_delete = false;
        // cycle app tabs
        match app.current_tab {
            app::Tabs::Library => {
                app.current_tab = app::Tabs::Enrichment;
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
            app::Tabs::Enrichment => {
                app.current_tab = app::Tabs::Duplicates;
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
            app::Tabs::Duplicates => {
                app.current_tab = app::Tabs::Missing;
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
            app::Tabs::Missing => {
                app.current_tab = app::Tabs::Library;
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
        }
    }

    // handle toggle track row selection
    if key.code == KeyCode::Char(' ') {
        match app.current_tab {
            app::Tabs::Library => {
                if let Some(idx) = app.library_state.selected() {
                    app.library_tracks[idx].is_selected = !app.library_tracks[idx].is_selected;
                }
            }
            app::Tabs::Enrichment => {
                if let Some(idx) = app.pending_state.selected() {
                    app.pending_tracks[idx].is_selected = !app.pending_tracks[idx].is_selected;
                }
            }
            app::Tabs::Duplicates => {
                if let Some(idx) = app.duplicate_state.selected() {
                    app.duplicate_tracks[idx].is_selected = !app.duplicate_tracks[idx].is_selected;
                }
            }
            app::Tabs::Missing => {
                if let Some(idx) = app.missing_state.selected() {
                    app.missing_tracks[idx].is_selected = !app.missing_tracks[idx].is_selected;
                }
            }
        }
    }

    // handle SELECT ALL
    if key.code == KeyCode::Char('i') {
        match app.current_tab {
            app::Tabs::Library => app
                .library_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = true),
            app::Tabs::Enrichment => app
                .pending_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = true),
            app::Tabs::Duplicates => app
                .duplicate_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = true),
            app::Tabs::Missing => app
                .missing_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = true),
        }
    }

    // handle DESELECT ALL
    if key.code == KeyCode::Char('o') {
        match app.current_tab {
            app::Tabs::Library => app
                .library_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = false),
            app::Tabs::Enrichment => app
                .pending_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = false),
            app::Tabs::Duplicates => app
                .duplicate_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = false),
            app::Tabs::Missing => app
                .missing_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = false),
        }
    }

    // handle INVERT SELECTION
    if key.code == KeyCode::Char('u') {
        match app.current_tab {
            app::Tabs::Library => app
                .library_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = !f.is_selected),
            app::Tabs::Enrichment => app
                .pending_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = !f.is_selected),
            app::Tabs::Duplicates => app
                .duplicate_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = !f.is_selected),
            app::Tabs::Missing => app
                .missing_tracks
                .iter_mut()
                .for_each(|f| f.is_selected = !f.is_selected),
        }
    }

    // handle DELETE tracks
    if key.code == KeyCode::Char('d') {
        if app.pending_delete {
            match app.current_tab {
                app::Tabs::Library => {}
                app::Tabs::Enrichment => {}
                app::Tabs::Duplicates => {
                    for track in &app.duplicate_tracks {
                        if track.is_selected {
                            if let Some(id) = track.id {
                                std::fs::remove_file(track.file_path.clone()).ok();
                                db::delete_single_track(&app.pool, id).await.ok();
                            }
                        }
                    }
                }
                app::Tabs::Missing => {
                    for track in &app.missing_tracks {
                        if track.is_selected {
                            if let Some(id) = track.id {
                                std::fs::remove_file(track.file_path.clone()).ok();
                                db::delete_single_track(&app.pool, id).await.ok();
                            }
                        }
                    }
                }
            }
            App::reload(app).await.ok();
            app.pending_delete = false;
        } else {
            app.pending_delete = true;
        }
    }

    // DO NOT PUT ANY HANDLERS BELOW THIS ONE
    if key.code == KeyCode::Esc {
        if app.pending_delete {
            app.pending_delete = false;
            return;
        }
        if app.properties_panel_open {
            app.properties_panel_open = false;
        } else {
            if !matches!(app.current_screen, app::Screens::Start) {
                app.current_screen = app::Screens::Start;
            }
        }
    }
}

async fn load_selected_track(app: &mut App) {
    let mut selected_track_db_id = 0;
    match app.current_tab {
        app::Tabs::Library => {
            if let Some(idx) = app.library_state.selected() {
                selected_track_db_id = app.library_tracks[idx].id.unwrap_or(0);
            }
        }
        app::Tabs::Enrichment => {
            if let Some(idx) = app.pending_state.selected() {
                selected_track_db_id = app.pending_tracks[idx].id.unwrap_or(0);
            }
        }
        app::Tabs::Duplicates => {
            if let Some(idx) = app.duplicate_state.selected() {
                selected_track_db_id = app.duplicate_tracks[idx].id.unwrap_or(0);
            }
        }
        app::Tabs::Missing => {
            if let Some(idx) = app.missing_state.selected() {
                selected_track_db_id = app.missing_tracks[idx].id.unwrap_or(0);
            }
        }
    }
    app.properties_of_track = load_track_full(&app.pool, selected_track_db_id)
        .await
        .ok()
        .flatten();
}

fn draw_tab_bar(f: &mut Frame, app: &mut App, area: Rect) {
    let tab_bar =
        ratatui::widgets::Tabs::new(vec!["Library", "Enrichment", "Duplicates", "Missing"])
            .select(match app.current_tab {
                app::Tabs::Library => 0,
                app::Tabs::Enrichment => 1,
                app::Tabs::Duplicates => 2,
                app::Tabs::Missing => 3,
            })
            .block(Block::default().borders(Borders::ALL).light_green())
            .highlight_style(Style::default().bold().light_green().reversed());

    f.render_widget(tab_bar, area);
}

fn draw_shortcuts_row(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new("Shortcuts: | cycle [TAB]s | [SPACE]lect | [U]nvert selection | s[I]lect all | select n[O]ne | [D]elete | [ESC]ape | [P]roperties | sea[/]rch"
        )
        .blue(),
        area
    );
}

fn draw_search_bar(f: &mut Frame, app: &mut App, area: Rect) {
    let block = if app.search_mode {
        Paragraph::new(format!("{}█", app.search_query.as_str()))
            .block(Block::bordered())
            .light_yellow()
    } else {
        Paragraph::new("press / to search")
            .block(Block::bordered())
            .gray()
    };

    f.render_widget(block, area);
}

fn draw_delete_prompt(f: &mut Frame, area: Rect) {
    let outer = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(9), // pane height
        Constraint::Fill(1),
    ])
    .split(area);

    let inner = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(48), // pane width
        Constraint::Fill(1),
    ])
    .split(outer[1]);

    f.render_widget(ratatui::widgets::Clear, inner[1]);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("DELETE?")
        .red();
    let text = Paragraph::new(vec![
        Line::raw(""),
        Line::raw(""),
        Line::from(format!("Do you want to delete ALL selected tracks?"))
            .light_red()
            .centered(),
        Line::raw(""),
        Line::from(format!("[ESC] CANCEL /// [D] DELETE"))
            .light_red()
            .centered(),
    ])
    .alignment(ratatui::layout::Alignment::Center)
    .centered()
    .block(block);
    f.render_widget(text, inner[1]);
}

fn draw_table_content(f: &mut Frame, app: &mut App, area: Rect) {
    match app.current_tab {
        app::Tabs::Library => {
            if app.library_tracks.len() == 0 {
                f.render_widget(Paragraph::new("No Library Tracks Found"), area);
                return;
            }

            let library_items = app.library_tracks.iter().map(|i| {
                let row = Row::new(vec![
                    Cell::new(if i.is_selected { "[X]" } else { "[ ]" }),
                    Cell::new(i.title.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.artist.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.album.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.file_format.as_deref().unwrap_or("Unknown")),
                    Cell::new(
                        i.file_size
                            .map(|v| format_size(v as u64, DECIMAL))
                            .unwrap_or("-".to_string()),
                    ),
                    Cell::new(format_track_duration(i.duration).unwrap_or("-".to_string())),
                    Cell::new(i.bitrate.map(|v| v.to_string()).unwrap_or("-".to_string())),
                    Cell::new(i.status.as_str()),
                    // Cell::new(i.file_hash.as_deref().unwrap_or("Unknown")),
                ]);
                if i.is_selected {
                    row.style(Style::default().light_green())
                } else {
                    row
                }
            });

            let list = Table::new(
                library_items,
                [
                    Constraint::Min(3),         // is selected?
                    Constraint::Percentage(25), // title
                    Constraint::Percentage(15), // artist
                    Constraint::Percentage(20), // album
                    Constraint::Percentage(10), // file format
                    Constraint::Percentage(10), // file size
                    Constraint::Percentage(5),  // duration
                    Constraint::Percentage(5),  // bitrate
                    Constraint::Percentage(10), // status
                                                // Constraint::Percentage(10), // file hash
                ],
            )
            .block(
                Block::default()
                    .title(
                        format!(
                            "Library: [{}/{}] - Selected: [{}/{}]",
                            app.library_state.selected().unwrap_or(0) + 1,
                            app.library_tracks.len(),
                            app.library_tracks.iter().filter(|f| f.is_selected).count(),
                            app.library_tracks.len(),
                        )
                        .light_magenta(),
                    )
                    .borders(Borders::ALL),
            )
            .row_highlight_style(Style::default().reversed())
            .header(
                Row::new(vec![
                    Cell::new("   "),
                    Cell::new("Title"),
                    Cell::new("Artist"),
                    Cell::new("Album"),
                    Cell::new("Format"),
                    Cell::new("Size"),
                    Cell::new("Duration"),
                    Cell::new("Bitrate"),
                    Cell::new("Status"),
                    // Cell::new("File Hash"),
                ])
                .bold()
                .bottom_margin(1),
            );

            f.render_stateful_widget(list, area, &mut app.library_state);
        }
        app::Tabs::Enrichment => {
            if app.pending_tracks.len() == 0 {
                f.render_widget(Paragraph::new("No Tracks Needing Enrichment Found"), area);
                return;
            }

            let enrichment_items = app.pending_tracks.iter().map(|i| {
                let row = Row::new(vec![
                    Cell::new(if i.is_selected { "[X]" } else { "[ ]" }),
                    Cell::new(i.title.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.artist.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.album.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.file_format.as_deref().unwrap_or("Unknown")),
                    Cell::new(
                        i.file_size
                            .map(|v| format_size(v as u64, DECIMAL))
                            .unwrap_or("-".to_string()),
                    ),
                    Cell::new(format_track_duration(i.duration).unwrap_or("-".to_string())),
                    Cell::new(i.bitrate.map(|v| v.to_string()).unwrap_or("-".to_string())),
                    Cell::new(i.status.as_str()),
                    // Cell::new(i.file_hash.as_deref().unwrap_or("Unknown")),
                ]);
                if i.is_selected {
                    row.style(Style::default().light_green())
                } else {
                    row
                }
            });

            let list = Table::new(
                enrichment_items,
                [
                    Constraint::Min(3),         // is selected?
                    Constraint::Percentage(25), // title
                    Constraint::Percentage(15), // artist
                    Constraint::Percentage(20), // album
                    Constraint::Percentage(10), // file format
                    Constraint::Percentage(10), // file size
                    Constraint::Percentage(5),  // duration
                    Constraint::Percentage(5),  // bitrate
                    Constraint::Percentage(10), // status
                                                // Constraint::Percentage(10), // file hash
                ],
            )
            .block(
                Block::default()
                    .title(
                        format!(
                            "Need Enrichment: [{}/{}] - Selected: [{}/{}]",
                            app.pending_state.selected().unwrap_or(0) + 1,
                            app.pending_tracks.len(),
                            app.pending_tracks.iter().filter(|f| f.is_selected).count(),
                            app.pending_tracks.len(),
                        )
                        .light_magenta(),
                    )
                    .borders(Borders::ALL),
            )
            .row_highlight_style(Style::default().reversed())
            .header(
                Row::new(vec![
                    Cell::new("   "),
                    Cell::new("Title"),
                    Cell::new("Artist"),
                    Cell::new("Album"),
                    Cell::new("Format"),
                    Cell::new("Size"),
                    Cell::new("Duration"),
                    Cell::new("Bitrate"),
                    Cell::new("Status"),
                    // Cell::new("File Hash"),
                ])
                .bold()
                .bottom_margin(1),
            );

            f.render_stateful_widget(list, area, &mut app.pending_state);
        }
        app::Tabs::Duplicates => {
            if app.duplicate_tracks.len() == 0 {
                f.render_widget(Paragraph::new("No Duplicate Tracks Found"), area);
                return;
            }

            let duplicate_items = app.duplicate_tracks.iter().map(|i| {
                let row = Row::new(vec![
                    Cell::new(if i.is_selected { "[X]" } else { "[ ]" }),
                    Cell::new(i.title.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.artist.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.album.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.file_format.as_deref().unwrap_or("Unknown")),
                    Cell::new(
                        i.file_size
                            .map(|v| format_size(v as u64, DECIMAL))
                            .unwrap_or("-".to_string()),
                    ),
                    Cell::new(format_track_duration(i.duration).unwrap_or("-".to_string())),
                    Cell::new(i.bitrate.map(|v| v.to_string()).unwrap_or("-".to_string())),
                    Cell::new(i.status.as_str()),
                    // Cell::new(i.file_hash.as_deref().unwrap_or("Unknown")),
                ]);
                if i.is_selected {
                    row.style(Style::default().light_green())
                } else {
                    row
                }
            });

            let list = Table::new(
                duplicate_items,
                [
                    Constraint::Min(3),         // is selected?
                    Constraint::Percentage(25), // title
                    Constraint::Percentage(15), // artist
                    Constraint::Percentage(20), // album
                    Constraint::Percentage(10), // file format
                    Constraint::Percentage(10), // file size
                    Constraint::Percentage(5),  // duration
                    Constraint::Percentage(5),  // bitrate
                    Constraint::Percentage(10), // status
                                                // Constraint::Percentage(10), // file hash
                ],
            )
            .block(
                Block::default()
                    .title(
                        format!(
                            "Duplicates: [{}/{}] - Selected: [{}/{}]",
                            app.duplicate_state.selected().unwrap_or(0) + 1,
                            app.duplicate_tracks.len(),
                            app.duplicate_tracks
                                .iter()
                                .filter(|f| f.is_selected)
                                .count(),
                            app.duplicate_tracks.len(),
                        )
                        .light_magenta(),
                    )
                    .borders(Borders::ALL),
            )
            .row_highlight_style(Style::default().reversed())
            .header(
                Row::new(vec![
                    Cell::new("   "),
                    Cell::new("Title"),
                    Cell::new("Artist"),
                    Cell::new("Album"),
                    Cell::new("Format"),
                    Cell::new("Size"),
                    Cell::new("Duration"),
                    Cell::new("Bitrate"),
                    Cell::new("Status"),
                    // Cell::new("File Hash"),
                ])
                .bold()
                .bottom_margin(1),
            );

            f.render_stateful_widget(list, area, &mut app.duplicate_state);
        }
        app::Tabs::Missing => {
            if app.missing_tracks.len() == 0 {
                f.render_widget(Paragraph::new("No Missing Tracks Found"), area);
                return;
            }

            let missing_items = app.missing_tracks.iter().map(|i| {
                let row = Row::new(vec![
                    Cell::new(if i.is_selected { "[X]" } else { "[ ]" }),
                    Cell::new(i.title.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.artist.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.album.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.file_format.as_deref().unwrap_or("Unknown")),
                    Cell::new(
                        i.file_size
                            .map(|v| format_size(v as u64, DECIMAL))
                            .unwrap_or("-".to_string()),
                    ),
                    Cell::new(format_track_duration(i.duration).unwrap_or("-".to_string())),
                    Cell::new(i.bitrate.map(|v| v.to_string()).unwrap_or("-".to_string())),
                    Cell::new(i.status.as_str()),
                    // Cell::new(i.file_hash.as_deref().unwrap_or("Unknown")),
                ]);
                if i.is_selected {
                    row.style(Style::default().light_green())
                } else {
                    row
                }
            });

            let list = Table::new(
                missing_items,
                [
                    Constraint::Min(3),         // is selected?
                    Constraint::Percentage(25), // title
                    Constraint::Percentage(15), // artist
                    Constraint::Percentage(20), // album
                    Constraint::Percentage(10), // file format
                    Constraint::Percentage(10), // file size
                    Constraint::Percentage(5),  // duration
                    Constraint::Percentage(5),  // bitrate
                    Constraint::Percentage(10), // status
                                                // Constraint::Percentage(10), // file hash
                ],
            )
            .block(
                Block::default()
                    .title(
                        format!(
                            "Missing: [{}/{}] - Selected: [{}/{}]",
                            app.missing_state.selected().unwrap_or(0) + 1,
                            app.missing_tracks.len(),
                            app.missing_tracks.iter().filter(|f| f.is_selected).count(),
                            app.missing_tracks.len(),
                        )
                        .light_magenta(),
                    )
                    .borders(Borders::ALL),
            )
            .row_highlight_style(Style::default().reversed())
            .header(
                Row::new(vec![
                    Cell::new("   "),
                    Cell::new("Title"),
                    Cell::new("Artist"),
                    Cell::new("Album"),
                    Cell::new("Format"),
                    Cell::new("Size"),
                    Cell::new("Duration"),
                    Cell::new("Bitrate"),
                    Cell::new("Status"),
                    // Cell::new("File Hash"),
                ])
                .bold()
                .bottom_margin(1),
            );

            f.render_stateful_widget(list, area, &mut app.missing_state);
        }
    }
}

fn draw_properties_panel(f: &mut Frame, app: &mut App, area: Rect) {
    if let Some(track) = &app.properties_of_track {
        let block = Block::default()
            .title(Line::from("Track Properties").bold().light_magenta())
            .borders(Borders::ALL);

        let inner = block.inner(area);

        let lines = vec![
            // core tags
            Line::from(vec![
                Span::styled("Title: ", Style::default().bold().light_blue()),
                Span::raw(track.title.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Artist: ", Style::default().bold().light_blue()),
                Span::raw(track.artist.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Album: ", Style::default().bold().light_blue()),
                Span::raw(track.album.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Album Artist: ", Style::default().bold().light_blue()),
                Span::raw(track.album_artist.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Album Artists: ", Style::default().bold().light_blue()),
                Span::raw(track.album_artists.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Composer: ", Style::default().bold().light_blue()),
                Span::raw(track.composer.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Label: ", Style::default().bold().light_blue()),
                Span::raw(track.label.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Genre: ", Style::default().bold().light_blue()),
                Span::raw(track.genre.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Comment: ", Style::default().bold().light_blue()),
                Span::raw(track.comment.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Lyrics: ", Style::default().bold().light_blue()),
                Span::raw(track.lyrics.as_deref().unwrap_or("")),
            ]),
            // numbering
            Line::from(vec![
                Span::styled("Track: ", Style::default().bold().light_blue()),
                Span::raw(track.track.map(|v| v.to_string()).unwrap_or_default()),
            ]),
            Line::from(vec![
                Span::styled("Track Total: ", Style::default().bold().light_blue()),
                Span::raw(track.track_total.map(|v| v.to_string()).unwrap_or_default()),
            ]),
            Line::from(vec![
                Span::styled("Disc: ", Style::default().bold().light_blue()),
                Span::raw(track.disc.map(|v| v.to_string()).unwrap_or_default()),
            ]),
            Line::from(vec![
                Span::styled("Disc Total: ", Style::default().bold().light_blue()),
                Span::raw(track.disc_total.map(|v| v.to_string()).unwrap_or_default()),
            ]),
            // dates
            Line::from(vec![
                Span::styled("Release Year: ", Style::default().bold().light_blue()),
                Span::raw(
                    track
                        .release_year
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                ),
            ]),
            Line::from(vec![
                Span::styled("Recording Date: ", Style::default().bold().light_blue()),
                Span::raw(track.recording_date.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled(
                    "Original Release Date: ",
                    Style::default().bold().light_blue(),
                ),
                Span::raw(track.original_release_date.as_deref().unwrap_or("")),
            ]),
            // release metadata
            Line::from(vec![
                Span::styled("Release Type: ", Style::default().bold().light_blue()),
                Span::raw(track.release_type.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Compilation: ", Style::default().bold().light_blue()),
                Span::raw(
                    track
                        .compilation
                        .map(|v| if v { "Yes" } else { "No" })
                        .unwrap_or(""),
                ),
            ]),
            Line::from(vec![
                Span::styled("ISRC: ", Style::default().bold().light_blue()),
                Span::raw(track.isrc.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Barcode: ", Style::default().bold().light_blue()),
                Span::raw(track.barcode.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Catalog Number: ", Style::default().bold().light_blue()),
                Span::raw(track.catalog_number.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("BPM: ", Style::default().bold().light_blue()),
                Span::raw(track.bpm.map(|v| v.to_string()).unwrap_or_default()),
            ]),
            Line::from(vec![
                Span::styled("Language: ", Style::default().bold().light_blue()),
                Span::raw(track.language.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Script: ", Style::default().bold().light_blue()),
                Span::raw(track.script.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("Mood: ", Style::default().bold().light_blue()),
                Span::raw(track.mood.as_deref().unwrap_or("")),
            ]),
            // replaygain
            Line::from(vec![
                Span::styled("RG Track Gain: ", Style::default().bold().light_blue()),
                Span::raw(track.replay_gain_track_gain.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("RG Track Peak: ", Style::default().bold().light_blue()),
                Span::raw(track.replay_gain_track_peak.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("RG Album Gain: ", Style::default().bold().light_blue()),
                Span::raw(track.replay_gain_album_gain.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("RG Album Peak: ", Style::default().bold().light_blue()),
                Span::raw(track.replay_gain_album_peak.as_deref().unwrap_or("")),
            ]),
            // tech properties
            Line::from(vec![
                Span::styled("File Format: ", Style::default().bold().light_blue()),
                Span::raw(track.file_format.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("File Size: ", Style::default().bold().light_blue()),
                Span::raw(
                    track
                        .file_size
                        .map(|v| format_size(v as u64, DECIMAL))
                        .unwrap_or_default(),
                ),
            ]),
            Line::from(vec![
                Span::styled("Duration: ", Style::default().bold().light_blue()),
                Span::raw(format_track_duration(track.duration).unwrap_or_default()),
            ]),
            Line::from(vec![
                Span::styled("Bitrate: ", Style::default().bold().light_blue()),
                Span::raw(track.bitrate.map(|v| v.to_string()).unwrap_or_default()),
            ]),
            Line::from(vec![
                Span::styled("Sample Rate: ", Style::default().bold().light_blue()),
                Span::raw(track.sample_rate.map(|v| v.to_string()).unwrap_or_default()),
            ]),
            Line::from(vec![
                Span::styled("Bit Depth: ", Style::default().bold().light_blue()),
                Span::raw(track.bit_depth.map(|v| v.to_string()).unwrap_or_default()),
            ]),
            Line::from(vec![
                Span::styled("Channels: ", Style::default().bold().light_blue()),
                Span::raw(track.channels.map(|v| v.to_string()).unwrap_or_default()),
            ]),
            // external IDs
            Line::from(vec![
                Span::styled("AcoustID: ", Style::default().bold().light_blue()),
                Span::raw(track.acoustid.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("MB Recording ID: ", Style::default().bold().light_blue()),
                Span::raw(track.musicbrainz_recording_id.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("MB Track ID: ", Style::default().bold().light_blue()),
                Span::raw(track.musicbrainz_track_id.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("MB Release ID: ", Style::default().bold().light_blue()),
                Span::raw(track.musicbrainz_release_id.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled(
                    "MB Release Group ID: ",
                    Style::default().bold().light_blue(),
                ),
                Span::raw(track.musicbrainz_release_group_id.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("MB Artist ID: ", Style::default().bold().light_blue()),
                Span::raw(track.musicbrainz_artist_id.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled(
                    "MB Release Artist ID: ",
                    Style::default().bold().light_blue(),
                ),
                Span::raw(track.musicbrainz_release_artist_id.as_deref().unwrap_or("")),
            ]),
            Line::from(vec![
                Span::styled("MB Work ID: ", Style::default().bold().light_blue()),
                Span::raw(track.musicbrainz_work_id.as_deref().unwrap_or("")),
            ]),
            // pipeline state
            Line::from(vec![
                Span::styled("Status: ", Style::default().bold().light_blue()),
                Span::raw(track.status.as_str()),
            ]),
            // file hash
            Line::from(vec![
                Span::styled("File Hash: ", Style::default().bold().light_blue()),
                Span::raw(track.file_hash.as_deref().unwrap_or("")),
            ]),
            // file path (duh)
            Line::from(vec![
                Span::styled("File Path: ", Style::default().bold().light_blue()),
                Span::raw(track.file_path.to_string_lossy().into_owned()),
            ]),
        ];

        f.render_widget(block, area);
        f.render_widget(Paragraph::new(lines), inner);
    }
}
