use std::rc::Rc;

use crate::app::{self, App};
use crossterm::event::{Event, KeyCode};
use humansize::{DECIMAL, format_size};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    match app.current_screen {
        app::Screens::Start => {
            draw_start_screen(f, app, area);
        }
        app::Screens::Main => {
            draw_main_screen(f, app, area);
        }
        app::Screens::Scanning => todo!(),
    }
}

fn draw_start_screen(f: &mut Frame, app: &mut App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Fill(1),   // top padding
        Constraint::Length(6), // title
        Constraint::Length(1), // version
        Constraint::Length(4), // stats
        Constraint::Length(4), // tooltips
        Constraint::Fill(1),   // bottom padding
    ])
    .split(area);

    let title = ratatui::text::Text::from(vec![
        Line::from(" _ __  _ __ ___  __ _ _ __ ___ | |__ | | ___ "),
        Line::from("| '_ \\| '__/ _ \\/ _` | '_ ` _ \\| '_ \\| |/ _ \\"),
        Line::from("| |_) | | |  __/ (_| | | | | | | |_) | |  __/"),
        Line::from("| .__/|_|  \\___|\\___|__| |_| |_|_.__/|_|\\___|"),
        Line::from("|_|                                          "),
    ]);

    f.render_widget(
        title.alignment(ratatui::layout::Alignment::Center),
        sections[1],
    );
    f.render_widget(
        Paragraph::new("v0.1.0").alignment(ratatui::layout::Alignment::Center),
        sections[2],
    );
    f.render_widget(
        Paragraph::new(format!(
            "Total Tracks: {}\nPending Enrichment: {}\nDuplicate Tracks: {}",
            format_thou(app.library_stats.total_tracks),
            format_thou(app.library_stats.total_pending),
            format_thou(app.library_stats.total_duplicates)
        ))
        .alignment(ratatui::layout::Alignment::Center),
        sections[3],
    );
    f.render_widget(
        Paragraph::new(
            format!("[s] : Scan Library\n[enter] : View Library\n[q] : Quit")
        )
            .alignment(ratatui::layout::Alignment::Center),
        sections[4],
    );
}

fn draw_main_screen(f: &mut Frame, app: &mut App, area: Rect) {
    let sections = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);

    draw_tab_bar(f, app, &sections);
    draw_table_content(f, app, &sections);
}

pub fn poll_events(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    if crossterm::event::poll(std::time::Duration::from_millis(16))? {
        match crossterm::event::read()? {
            Event::Key(key) => {
                if key.code == KeyCode::Char('q') {
                    app.should_quit = true;
                }

                match app.current_screen {
                    app::Screens::Start => {
                        if key.code == KeyCode::Char('s') {
                            // todo!(); // TODO: scan library
                        }
                        if key.code == KeyCode::Enter {
                            app.current_screen = app::Screens::Main;
                        }
                    }

                    app::Screens::Main => {
                        if key.code == KeyCode::Up {
                            // highlight previous track
                            match app.current_tab {
                                app::Tabs::Library => app.library_state.select_previous(),
                                app::Tabs::Enrichment => app.pending_state.select_previous(),
                                app::Tabs::Duplicates => app.duplicate_state.select_previous(),
                            }
                        }
                        if key.code == KeyCode::Down {
                            // highlight next track
                            match app.current_tab {
                                app::Tabs::Library => app.library_state.select_next(),
                                app::Tabs::Enrichment => app.pending_state.select_next(),
                                app::Tabs::Duplicates => app.duplicate_state.select_next(),
                            }
                        }
                        if key.code == KeyCode::Tab {
                            // cycle app tabs
                            match app.current_tab {
                                app::Tabs::Library => app.current_tab = app::Tabs::Enrichment,
                                app::Tabs::Enrichment => app.current_tab = app::Tabs::Duplicates,
                                app::Tabs::Duplicates => app.current_tab = app::Tabs::Library,
                            }
                        }

                        if key.code == KeyCode::Esc {
                            app.current_screen = app::Screens::Start;
                        }
                    }
                    app::Screens::Scanning => {}
                };
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn format_track_duration(duration_millis: Option<u32>) -> Option<String> {
    duration_millis.map(|mut d| {
        d /= 1000;
        let mins = d / 60;
        let secs = d % 60;
        format!("{}:{:02}", mins, secs)
    })
}

pub fn format_thou(n: u32) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn draw_tab_bar(f: &mut Frame, app: &mut App, sections: &Rc<[Rect]>) {
    let tab_bar = ratatui::widgets::Tabs::new(vec!["Library", "Enrichment", "Duplicates"])
        .select(match app.current_tab {
            app::Tabs::Library => 0,
            app::Tabs::Enrichment => 1,
            app::Tabs::Duplicates => 2,
        })
        .block(Block::default().borders(Borders::ALL));

    f.render_widget(tab_bar, sections[0]);
}

fn draw_table_content(f: &mut Frame, app: &mut App, sections: &Rc<[Rect]>) {
    match app.current_tab {
        app::Tabs::Library => {
            if app.library_tracks.len() == 0 {
                f.render_widget(Paragraph::new("No Library Tracks Found"), sections[1]);
                return;
            }

            let library_items = app.library_tracks.iter().map(|i| {
                Row::new(vec![
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
                ])
            });

            let list = Table::new(
                library_items,
                [
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
                    .title(format!(
                        "Library: [{}/{}]",
                        app.library_state.selected().unwrap_or(0) + 1,
                        app.library_tracks.len()
                    ))
                    .borders(Borders::ALL),
            )
            .row_highlight_style(Style::default().reversed())
            .header(
                Row::new(vec![
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

            f.render_stateful_widget(list, sections[1], &mut app.library_state);
        }
        app::Tabs::Enrichment => {
            if app.pending_tracks.len() == 0 {
                f.render_widget(
                    Paragraph::new("No Tracks Needing Enrichment Found"),
                    sections[1],
                );
                return;
            }

            let enrichment_items = app.pending_tracks.iter().map(|i| {
                Row::new(vec![
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
                ])
            });

            let list = Table::new(
                enrichment_items,
                [
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
                    .title(format!(
                        "Need Enrichment: [{}/{}]",
                        app.pending_state.selected().unwrap_or(0) + 1,
                        app.pending_tracks.len()
                    ))
                    .borders(Borders::ALL),
            )
            .row_highlight_style(Style::default().reversed())
            .header(
                Row::new(vec![
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

            f.render_stateful_widget(list, sections[1], &mut app.pending_state);
        }
        app::Tabs::Duplicates => {
            if app.duplicate_tracks.len() == 0 {
                f.render_widget(Paragraph::new("No Duplicate Tracks Found"), sections[1]);
                return;
            }

            let duplicate_items = app.duplicate_tracks.iter().map(|i| {
                Row::new(vec![
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
                ])
            });

            let list = Table::new(
                duplicate_items,
                [
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
                    .title(format!(
                        "Duplicates: [{}/{}]",
                        app.duplicate_state.selected().unwrap_or(0) + 1,
                        app.duplicate_tracks.len()
                    ))
                    .borders(Borders::ALL),
            )
            .row_highlight_style(Style::default().reversed())
            .header(
                Row::new(vec![
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

            f.render_stateful_widget(list, sections[1], &mut app.duplicate_state);
        }
    }
}
