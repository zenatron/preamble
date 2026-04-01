use crate::app::{self, App};
use crossterm::event::{Event, KeyCode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    // f.render_widget(Paragraph::new("preamble"), area);

    let sections = Layout::vertical(
        [Constraint::Length(3), Constraint::Min(0)]
    ).split(area);

    let tab_bar = ratatui::widgets::Tabs::new(vec!["Library", "Enrichment", "Duplicates"])
        .select(
            match app.current_tab {
                app::Tabs::Library => 0,
                app::Tabs::Enrichment => 1,
                app::Tabs::Duplicates => 2,
            }
        )
        .block(Block::default().borders(Borders::ALL));
    
    f.render_widget(tab_bar, sections[0]);


    match app.current_tab {
        app::Tabs::Library => {
            let library_items = app.library_tracks.iter().map(|i| {
                Row::new(vec![
                    Cell::new(i.title.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.artist.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.album.as_deref().unwrap_or("Unknown")),

                    Cell::new(i.file_format.as_deref().unwrap_or("Unknown")),
                    Cell::new(i.file_size.map(|v| v.to_string()).unwrap_or("-".to_string())),
                    Cell::new(i.duration.map(|v| v.to_string()).unwrap_or("-".to_string())),
                    Cell::new(i.bitrate.map(|v| v.to_string()).unwrap_or("-".to_string())),

                    Cell::new(i.status.as_str()),
                    Cell::new(i.file_hash.as_deref().unwrap_or("Unknown")),
                ])
            });

            let list = Table::new(
                library_items,
                [
                    Constraint::Percentage(25), // title
                    Constraint::Percentage(20), // artist
                    Constraint::Percentage(15), // album

                    Constraint::Percentage(5), // file format
                    Constraint::Percentage(10), // file size
                    Constraint::Percentage(10), // duration
                    Constraint::Percentage(10), // bitrate

                    Constraint::Percentage(5), // status
                    // Constraint::Percentage(10), // file hash
                ]
            )
                .block(
                    Block::default()
                        .title(format!(
                            "Library: [{}]/[{}]",
                            app.library_state.selected().unwrap_or(0) + 1,
                            app.library_tracks.len()
                        ))
                        .borders(Borders::ALL),
                )
                .row_highlight_style(Style::default().reversed())
                .header(Row::new(vec![
                    Cell::new("Title"),
                    Cell::new("Artist"),
                    Cell::new("Album"),
                    Cell::new("Format"),
                    Cell::new("Size (bytes)"),
                    Cell::new("Duration (ms)"),
                    Cell::new("Bitrate"),
                    Cell::new("Status"),
                    // Cell::new("File Hash"),
                ]).bold().bottom_margin(1));

            f.render_stateful_widget(list, sections[1], &mut app.library_state);
        }
        app::Tabs::Enrichment => {
            // let enrichment_items = app.pending_tracks.iter().map(|i| {
            //     ListItem::new(format!(
            //         "{} - {}",
            //         i.artist.as_deref().unwrap_or("Unknown"),
            //         i.title.as_deref().unwrap_or("Unknown")
            //     ))
            // });

            // let list = List::new(enrichment_items)
            //     .block(
            //         Block::default()
            //             .title(format!(
            //                 "Pending Enrichment: [{}]/[{}]",
            //                 app.pending_state.selected().unwrap_or(0) + 1,
            //                 app.pending_tracks.len()
            //             ))
            //             .borders(Borders::ALL),
            //     )
            //     .highlight_style(Style::default().reversed());

            // f.render_stateful_widget(list, sections[1], &mut app.pending_state);
        }
        app::Tabs::Duplicates => {
            // f.render_widget(Paragraph::new("Duplicates - WIP"), sections[1]);
        }
    }
}

pub fn poll_events(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    if crossterm::event::poll(std::time::Duration::from_millis(16))? {

        match crossterm::event::read()? {
            Event::Key(key) => {
                if key.code == KeyCode::Char('q') {
                    app.should_quit = true;
                }
                if key.code == KeyCode::Up {
                    match app.current_tab {
                        app::Tabs::Library => {
                            app.library_state.select_previous();
                        }
                        app::Tabs::Enrichment => {
                            app.pending_state.select_previous();
                        }
                        app::Tabs::Duplicates => {}
                    }
                }
                if key.code == KeyCode::Down {
                    match app.current_tab {
                        app::Tabs::Library => {
                            app.library_state.select_next();
                        }
                        app::Tabs::Enrichment => {
                            app.pending_state.select_next();
                        }
                        app::Tabs::Duplicates => {}
                    }
                }
                if key.code == KeyCode::Tab {
                    // cycle app tabs
                    match app.current_tab {
                        // TODO: currently doesn't actually change what is being drawn
                        app::Tabs::Library => app.current_tab = app::Tabs::Enrichment,
                        app::Tabs::Enrichment => app.current_tab = app::Tabs::Duplicates,
                        app::Tabs::Duplicates => app.current_tab = app::Tabs::Library,
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
