use crate::app::{self, App, StatusLevel};
use crate::db::{self, load_track_full};
use crate::enrich::enrich_pending;
use crate::formatters::{common_path_prefix, format_thou, format_track_duration};
use crate::reader::{
    ScanEvent, ValidateEvent, health_check, rescan_changed, scan_library, validate_paths,
};
use crate::track::TrackSummary;

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
        app::Screens::Picker => {
            draw_picker_screen(f, app, area);
        }
        app::Screens::CreateLibrary => {
            draw_create_library_screen(f, app, area);
        }
        app::Screens::Start => {
            draw_start_screen(f, app, area);
        }
        app::Screens::Main => {
            draw_main_screen(f, app, area);
        }
        app::Screens::Scanning => {
            draw_scanning_screen(f, app, area);
        }
        app::Screens::Stats => {
            draw_stats_screen(f, app, area);
        }
    }

    if app.export_mode {
        draw_export_popup(f, app, area);
    }

    if app.filter_mode {
        draw_filter_popup(f, app, area);
    }

    if app.help_open {
        draw_help_overlay(f, area);
    }

    if app.edit.is_some() {
        draw_edit_popup(f, app, area);
    }

    if app.should_quit {
        draw_confirmation_prompt(
            f,
            area,
            "QUIT",
            "Do you want to quit the app?",
            "[ESC] CANCEL /// [Q] QUIT",
        );
    }
}

fn centered_popup(area: Rect, width: u16, height: u16) -> Rect {
    let v = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width),
        Constraint::Fill(1),
    ])
    .split(v[1])[1]
}

fn draw_export_popup(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_popup(area, 54, 10);
    f.render_widget(ratatui::widgets::Clear, popup);

    let count = app.tabs[app.current_tab].tracks.len();
    let key = |k: &str| Span::styled(format!("  [{k}] "), Style::default().light_green().bold());
    let lines = vec![
        Line::from(format!("  Export {count} row(s) from this view:")).gray(),
        Line::from(""),
        Line::from(vec![key("c"), Span::raw("CSV")]),
        Line::from(vec![key("j"), Span::raw("JSON")]),
        Line::from(vec![key("m"), Span::raw("M3U playlist")]),
        Line::from(vec![key("d"), Span::raw("Duplicate report (CSV)")]),
        Line::from(""),
        Line::from("  [esc] cancel").gray(),
    ];

    let block = Block::default()
        .title("Export".light_magenta().bold())
        .borders(Borders::ALL)
        .border_style(Style::default().light_magenta());
    f.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_filter_popup(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_popup(area, 52, 11);
    f.render_widget(ratatui::widgets::Clear, popup);

    let filter = &app.tabs[app.current_tab].filter;
    let on = |b: bool| if b { "ON" } else { "off" };
    let fmt = filter.format.clone().unwrap_or_else(|| "any".to_string());
    let br = filter
        .min_bitrate
        .map(|b| format!("≥ {b} kbps"))
        .unwrap_or_else(|| "any".to_string());

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  [t] ", Style::default().light_green().bold()),
            Span::raw(format!("Format         {fmt}")),
        ]),
        Line::from(vec![
            Span::styled("  [b] ", Style::default().light_green().bold()),
            Span::raw(format!("Min bitrate    {br}")),
        ]),
        Line::from(vec![
            Span::styled("  [i] ", Style::default().light_green().bold()),
            Span::raw(format!("Missing ISRC   {}", on(filter.no_isrc))),
        ]),
        Line::from(vec![
            Span::styled("  [h] ", Style::default().light_green().bold()),
            Span::raw(format!("Unhealthy only {}", on(filter.unhealthy))),
        ]),
        Line::from(""),
        Line::from("  [c] clear   [esc/enter] close").gray(),
    ];

    let block = Block::default()
        .title("Filters".light_magenta().bold())
        .borders(Borders::ALL)
        .border_style(Style::default().light_magenta());
    f.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_edit_popup(f: &mut Frame, app: &App, area: Rect) {
    let Some(edit) = &app.edit else {
        return;
    };
    let height = edit.fields.len() as u16 + 6;
    let popup = centered_popup(area, 66, height);
    f.render_widget(ratatui::widgets::Clear, popup);

    let mut lines = vec![Line::raw("")];
    for (i, (label, value)) in edit.fields.iter().enumerate() {
        let focused = i == edit.focus;
        let shown = if focused {
            format!("{value}█")
        } else {
            value.clone()
        };
        let value_style = if focused {
            Style::default().light_yellow()
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {label:<14}"),
                Style::default().light_blue().bold(),
            ),
            Span::styled(shown, value_style),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from("  [↑/↓] field · [enter] save to file + DB · [esc] cancel").gray());

    let block = Block::default()
        .title("Edit Tags".light_magenta().bold())
        .borders(Borders::ALL)
        .border_style(Style::default().light_magenta());
    f.render_widget(Paragraph::new(lines).block(block), popup);
}

fn draw_help_overlay(f: &mut Frame, area: Rect) {
    let popup = centered_popup(area, 64, 22);
    f.render_widget(ratatui::widgets::Clear, popup);

    let key = |k: &str| Span::styled(format!("{k:<10}"), Style::default().light_green().bold());
    let row =
        |k: &str, d: &str| Line::from(vec![Span::raw("  "), key(k), Span::raw(d.to_string())]);

    let lines = vec![
        Line::from("  Navigation").light_cyan().bold(),
        row("↑/↓", "move selection / groups / members"),
        row("Tab", "switch tab"),
        row("/", "search"),
        row("f", "filter facets popup"),
        row("s / S", "cycle sort field / flip direction"),
        row("p", "properties panel (Dup: switch pane)"),
        row("PgUp/PgDn", "scroll properties"),
        Line::from("  Actions").light_cyan().bold(),
        row("Space", "toggle row selection"),
        row("i/o/u", "select all / none / invert"),
        row("d", "flag → Trash (Trash: purge)"),
        row("r", "Trash: restore · Enrichment: retry"),
        row("k", "Duplicates: keep highlighted member"),
        row("K", "Duplicates: keep all (dismiss group)"),
        row("e", "Enrichment: run pipeline"),
        row("m", "edit tags of highlighted track"),
        row("x", "export current view"),
        row("c", "statistics screen"),
        row("w", "toggle watch mode"),
        row("z", "undo last action"),
        Line::from("  [?] or [esc] to close").gray(),
    ];

    let block = Block::default()
        .title("Keybindings".light_magenta().bold())
        .borders(Borders::ALL)
        .border_style(Style::default().light_magenta());
    f.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The preamble wordmark, shared by the picker / create / start screens.
fn preamble_logo() -> ratatui::text::Text<'static> {
    ratatui::text::Text::from(vec![
        Line::from("                                _     _      "),
        Line::from(" _ __  _ __ ___  __ _ _ __ ___ | |__ | | ___ "),
        Line::from("| '_ \\| '__/ _ \\/ _` | '_ ` _ \\| '_ \\| |/ _ \\"),
        Line::from("| |_) | | |  __/ (_| | | | | | | |_) | |  __/"),
        Line::from("| .__/|_| \\___/\\___|_|_| |_| |_|_.__/|_|\\___|"),
        Line::from("|_|                                          "),
    ])
    .light_red()
}

/// Library picker: lists known libraries plus a trailing "Create new..." row.
fn draw_picker_screen(f: &mut Frame, app: &mut App, area: Rect) {
    use ratatui::layout::Alignment;
    let sections = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(6), // title
        Constraint::Length(2), // subtitle
        Constraint::Min(6),    // list
        Constraint::Length(2), // hints
        Constraint::Fill(1),
    ])
    .split(area);

    f.render_widget(preamble_logo().alignment(Alignment::Center), sections[1]);
    f.render_widget(
        Paragraph::new("Select a library")
            .alignment(Alignment::Center)
            .blue(),
        sections[2],
    );

    let mut lines: Vec<Line> = Vec::new();
    for (i, lib) in app.libraries.iter().enumerate() {
        let selected = i == app.picker_index;
        let marker = if selected { "▶ " } else { "  " };
        let line = Line::from(format!("{marker}{}  —  {}", lib.name, lib.path));
        lines.push(if selected {
            line.style(Style::default().light_cyan().add_modifier(Modifier::BOLD))
        } else {
            line
        });
    }
    let create_selected = app.picker_index >= app.libraries.len();
    let marker = if create_selected { "▶ " } else { "  " };
    let create_line = Line::from(format!("{marker}➕ Create new library…"));
    lines.push(if create_selected {
        create_line.style(Style::default().light_green().add_modifier(Modifier::BOLD))
    } else {
        create_line
    });

    let list = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Libraries "));
    let width = 60.min(area.width.saturating_sub(4));
    f.render_widget(list, centered_popup(sections[3], width, sections[3].height));

    f.render_widget(
        Paragraph::new("[↑/↓] move   [enter] open / create   [q] quit")
            .alignment(Alignment::Center)
            .style(Style::new().add_modifier(Modifier::BOLD)),
        sections[4],
    );

    let bottom = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    draw_status_bar(f, app, bottom);
}

/// Create-library form: a Path field and a Name field with focus highlighting.
fn draw_create_library_screen(f: &mut Frame, app: &mut App, area: Rect) {
    use ratatui::layout::Alignment;
    let sections = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(6), // title
        Constraint::Length(2), // subtitle
        Constraint::Length(6), // form
        Constraint::Length(2), // hints
        Constraint::Fill(1),
    ])
    .split(area);

    f.render_widget(preamble_logo().alignment(Alignment::Center), sections[1]);
    f.render_widget(
        Paragraph::new("Create a new library")
            .alignment(Alignment::Center)
            .blue(),
        sections[2],
    );

    let path_focused = app.new_lib_focus == app::NewLibField::Path;
    let name_focused = app.new_lib_focus == app::NewLibField::Name;
    let field_style = |on: bool| {
        if on {
            Style::default().light_cyan().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    };
    let caret = |on: bool| if on { "_" } else { "" };

    let form_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Path:  "),
            Span::styled(
                format!("{}{}", app.new_lib_path, caret(path_focused)),
                field_style(path_focused),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Name:  "),
            Span::styled(
                format!("{}{}", app.new_lib_name, caret(name_focused)),
                field_style(name_focused),
            ),
        ]),
    ];

    let form = Paragraph::new(form_lines)
        .block(Block::default().borders(Borders::ALL).title(" New Library "));
    let width = 60.min(area.width.saturating_sub(4));
    f.render_widget(form, centered_popup(sections[3], width, sections[3].height));

    f.render_widget(
        Paragraph::new("[tab] switch field   [enter] create   [esc] back")
            .alignment(Alignment::Center)
            .style(Style::new().add_modifier(Modifier::BOLD)),
        sections[4],
    );

    let bottom = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    draw_status_bar(f, app, bottom);
}

fn draw_start_screen(f: &mut Frame, app: &mut App, area: Rect) {
    let sections = Layout::vertical([
        Constraint::Fill(1),   // top padding
        Constraint::Length(6), // title
        Constraint::Length(1), // version
        Constraint::Length(6), // stats
        Constraint::Length(2), // path
        Constraint::Length(5), // tooltips
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
        Paragraph::new(concat!("v", env!("CARGO_PKG_VERSION")))
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
        Line::from(vec![
            Span::raw("Flagged for Deletion: "),
            Span::styled(
                format_thou(app.library_stats.total_marked),
                Style::default().light_red().bold(),
            ),
        ]),
    ]);
    f.render_widget(
        Paragraph::new(stats).alignment(ratatui::layout::Alignment::Center),
        sections[3],
    );
    let lib_name = app
        .active_library
        .as_ref()
        .map(|l| l.name.as_str())
        .unwrap_or("(none)");
    f.render_widget(
        Paragraph::new(format!(
            "Library: {}   ·   Path: {}",
            lib_name,
            app.pending_scan_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(none)".to_string())
        ))
        .alignment(ratatui::layout::Alignment::Center)
        .light_cyan(),
        sections[4],
    );
    f.render_widget(
        Paragraph::new(format!(
            "[s] Scan new   [u] Rescan changed   [r] Fresh scan (rebuilds DB)\n[v] Validate paths   [h] Health check   [L] Switch library\n[enter] View Library    [q] Quit"
        )).style(Style::new().add_modifier(Modifier::BOLD))
        .alignment(ratatui::layout::Alignment::Center),
        sections[5],
    );

    // status bar pinned to the bottom row
    let bottom = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    draw_status_bar(f, app, bottom);

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
        Constraint::Min(0),    // table content
        Constraint::Length(1), // status bar
    ])
    .split(area);

    draw_tab_bar(f, app, sections[0]);
    draw_shortcuts_row(f, app, sections[1]);
    draw_search_bar(f, app, sections[2]);
    draw_table_content(f, app, sections[3]);
    draw_status_bar(f, app, sections[4]);

    if app.is_enriching {
        draw_enrich_popup(f, app, area);
    }

    if app.pending_purge {
        draw_confirmation_prompt(
            f,
            area,
            "PURGE TRASH",
            "Permanently delete flagged files from disk?",
            "[ESC] CANCEL /// [D] PURGE",
        );
    }
}

fn draw_enrich_popup(f: &mut Frame, app: &mut App, area: Rect) {
    let outer = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(5), // pane height
        Constraint::Fill(1),
    ])
    .split(area);

    let inner = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(50), // pane width
        Constraint::Fill(1),
    ])
    .split(outer[1]);

    f.render_widget(ratatui::widgets::Clear, inner[1]);

    let (n, total) = app.enrich_progress.unwrap_or((0, 0));
    let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let spinner = frames[app.spinner_tick % frames.len()];
    let ratio = if total == 0 {
        0.0
    } else {
        (n as f64 / total as f64).clamp(0.0, 1.0)
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(format!("{spinner} Enriching (AcoustID → MusicBrainz)"))
                .title_bottom(Line::from(" [Esc] cancel ").gray().right_aligned())
                .borders(Borders::ALL)
                .light_cyan(),
        )
        .gauge_style(
            Style::default()
                .light_cyan()
                .fg(ratatui::style::Color::Black),
        )
        .ratio(ratio)
        .label(format!("[{n}/{total}]"));

    f.render_widget(gauge, inner[1]);
}

/// IDs of the tracks an action should target in a normal (track-list) tab:
/// every checkbox-selected row, or - if none are selected - the highlighted row.
fn target_ids(tab: &app::TabData) -> Vec<i64> {
    let selected: Vec<i64> = tab
        .tracks
        .iter()
        .filter(|t| t.is_selected)
        .filter_map(|t| t.id)
        .collect();
    if !selected.is_empty() {
        return selected;
    }
    tab.state
        .selected()
        .and_then(|idx| tab.tracks.get(idx))
        .and_then(|t| t.id)
        .into_iter()
        .collect()
}

/// Moves every flagged track (the Trash tab's contents) into the quarantine
/// directory, removes their rows, and logs the purge so it can be undone.
async fn purge_marked(app: &mut App) {
    let targets: Vec<(i64, std::path::PathBuf)> = app.tabs[app.current_tab]
        .tracks
        .iter()
        .filter_map(|t| t.id.map(|id| (id, t.file_path.clone())))
        .collect();

    let qdir = app.config.quarantine_dir.clone();
    std::fs::create_dir_all(&qdir).ok();

    let mut restored = Vec::new();
    for (id, path) in targets {
        // Capture the full row so the purge can be reversed.
        let Ok(Some(track)) = db::load_track_full(&app.pool, id).await else {
            continue;
        };
        let filename = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.to_string());
        let qpath = qdir.join(format!("{id}_{filename}"));

        if crate::undo::move_file(&path, &qpath).is_ok() {
            db::delete_single_track(&app.pool, id).await.ok();
            restored.push(crate::undo::RestoredTrack {
                track,
                original_path: path,
                quarantine_path: qpath,
                library_id: app.active_library_id(),
            });
        }
    }

    let purged = restored.len();
    if purged > 0 {
        let action = crate::undo::UndoAction::RestoreRows { tracks: restored };
        db::log_action(
            &app.pool,
            "purge",
            &format!("Purged {purged} track(s)"),
            &action,
        )
        .await
        .ok();
    }
    app.reload().await.ok();
    if purged > 0 {
        app.set_status(
            StatusLevel::Success,
            format!("Purged {purged} track(s) to quarantine ([z] to undo)."),
        );
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
                .title(format!("{}...", app.scan_label))
                .title_bottom(Line::from(" [Esc] cancel ").gray().right_aligned())
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

fn draw_stats_screen(f: &mut Frame, app: &App, area: Rect) {
    let Some(s) = &app.stats else {
        f.render_widget(Paragraph::new("No statistics").light_magenta(), area);
        return;
    };

    let outer = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(4), // totals
        Constraint::Min(0),    // body
        Constraint::Length(1), // hint
    ])
    .split(area);

    f.render_widget(
        Paragraph::new("Library Statistics")
            .light_magenta()
            .bold()
            .alignment(ratatui::layout::Alignment::Center),
        outer[0],
    );

    // Totals block.
    let secs = s.total_duration / 1000;
    let runtime = format!("{}h {}m", secs / 3600, (secs % 3600) / 60);
    let lossless_pct = if s.total_tracks > 0 {
        (s.lossless as f64 / s.total_tracks as f64) * 100.0
    } else {
        0.0
    };
    let totals = vec![
        Line::from(vec![
            Span::styled("Tracks: ", Style::default().light_blue()),
            Span::styled(format_thou(s.total_tracks as u32), Style::default().bold()),
            Span::raw("    "),
            Span::styled("Size: ", Style::default().light_blue()),
            Span::styled(
                format_size(s.total_size.max(0) as u64, DECIMAL),
                Style::default().bold(),
            ),
            Span::raw("    "),
            Span::styled("Runtime: ", Style::default().light_blue()),
            Span::styled(runtime, Style::default().bold()),
        ]),
        Line::from(vec![
            Span::styled("Avg bitrate: ", Style::default().light_blue()),
            Span::styled(
                format!("{:.0} kbps", s.avg_bitrate),
                Style::default().bold(),
            ),
            Span::raw("    "),
            Span::styled("Lossless: ", Style::default().light_blue()),
            Span::styled(
                format!("{} ({:.0}%)", format_thou(s.lossless as u32), lossless_pct),
                Style::default().light_green().bold(),
            ),
            Span::raw("    "),
            Span::styled("Lossy: ", Style::default().light_blue()),
            Span::styled(
                format_thou(s.lossy.max(0) as u32),
                Style::default().light_yellow().bold(),
            ),
        ]),
    ];
    f.render_widget(
        Paragraph::new(totals).block(Block::default().borders(Borders::ALL).light_green()),
        outer[1],
    );

    // Body: two columns of breakdown panels.
    let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(outer[2]);
    let left =
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).split(cols[0]);
    let right =
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).split(cols[1]);

    let fmt_lines: Vec<Line> = s
        .by_format
        .iter()
        .map(|(fmt, n, size)| {
            Line::from(format!(
                "{:<8} {:>6}  {:>10}",
                fmt,
                n,
                format_size(*size.max(&0) as u64, DECIMAL)
            ))
        })
        .collect();
    render_stats_panel(f, left[0], "By Format", fmt_lines);

    let artist_lines: Vec<Line> = s
        .top_artists
        .iter()
        .map(|(name, n)| Line::from(format!("{:>4}  {}", n, name)))
        .collect();
    render_stats_panel(f, right[0], "Top Artists", artist_lines);

    let decade_lines: Vec<Line> = s
        .by_decade
        .iter()
        .map(|(d, n)| Line::from(format!("{:<8} {:>6}", d, n)))
        .collect();
    render_stats_panel(f, left[1], "By Decade", decade_lines);

    let mut status_lines: Vec<Line> = s
        .by_status
        .iter()
        .map(|(st, n)| Line::from(format!("{:<14} {:>6}", st, n)))
        .collect();
    if !s.health.is_empty() {
        status_lines.push(Line::from("- health -").gray());
        for (issue, n) in &s.health {
            status_lines.push(Line::from(format!("{:<14} {:>6}", issue, n)).light_red());
        }
    }
    render_stats_panel(f, right[1], "By Status", status_lines);

    f.render_widget(Paragraph::new("[esc] back to library").gray(), outer[3]);
}

fn render_stats_panel(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line>) {
    let block = Block::default()
        .title(title.light_magenta().bold())
        .borders(Borders::ALL);
    f.render_widget(Paragraph::new(lines).block(block), area);
}

pub async fn poll_events(app: &mut App) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Poll for input at ~15 FPS. An available event means something may have
    // changed, so request a redraw; an idle screen produces no events and so is
    // not needlessly repainted.
    if crossterm::event::poll(std::time::Duration::from_millis(1000 / 15))? {
        app.needs_redraw = true;
        match crossterm::event::read()? {
            Event::Key(key) => {
                match app.current_screen {
                    app::Screens::Picker => {
                        handle_picker_navigation(app, key).await;
                    }
                    app::Screens::CreateLibrary => {
                        handle_create_navigation(app, key).await;
                    }
                    app::Screens::Start => {
                        handle_start_navigation(app, key).await;
                    }

                    app::Screens::Main => {
                        handle_main_navigation(app, key).await;
                    }
                    app::Screens::Scanning => {
                        if key.code == KeyCode::Esc {
                            app.request_cancel();
                        }
                    }
                    app::Screens::Stats => {
                        if matches!(
                            key.code,
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('c')
                        ) {
                            app.current_screen = app::Screens::Main;
                        }
                    }
                };
            }
            Event::Mouse(m) => {
                if matches!(app.current_screen, app::Screens::Main) {
                    handle_mouse(app, m).await;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

async fn handle_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    use crossterm::event::{MouseButton, MouseEventKind};

    // Popups capture the wheel; ignore mouse while they're open.
    if app.help_open || app.filter_mode || app.search_mode {
        return;
    }

    let on_duplicates = app.tabs[app.current_tab].label == "Duplicates";

    match m.kind {
        MouseEventKind::ScrollDown => {
            if app.properties_panel_open {
                app.properties_scroll = app.properties_scroll.saturating_add(3);
            } else if on_duplicates {
                app.duplicates.select_member(1);
            } else {
                let state = &mut app.tabs[app.current_tab].state;
                if state.selected().is_some() {
                    state.select_next();
                }
                if app.properties_panel_open {
                    load_selected_track(app).await;
                }
            }
        }
        MouseEventKind::ScrollUp => {
            if app.properties_panel_open {
                app.properties_scroll = app.properties_scroll.saturating_sub(3);
            } else if on_duplicates {
                app.duplicates.select_member(-1);
            } else {
                let state = &mut app.tabs[app.current_tab].state;
                if state.selected().is_some() {
                    state.select_previous();
                }
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // Click on the tab bar cycles to the next tab.
            if rect_contains(app.tab_bar_area, m.column, m.row) {
                app.current_tab = (app.current_tab + 1) % app.tabs.len();
                return;
            }
            // Click on a table row selects it. Data rows start 3 lines below the
            // table area top (top border + header + header margin).
            if !on_duplicates && rect_contains(app.table_area, m.column, m.row) {
                let first_row = app.table_area.y + 3;
                if m.row >= first_row {
                    let tab = &mut app.tabs[app.current_tab];
                    let offset = tab.state.offset();
                    let idx = offset + (m.row - first_row) as usize;
                    if idx < tab.tracks.len() {
                        tab.state.select(Some(idx));
                        if app.properties_panel_open {
                            load_selected_track(app).await;
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn rect_contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

/// Library picker: move the highlight over known libraries plus a trailing
/// "Create new..." row; Enter opens the highlighted library or starts the create
/// form. `q` quits (nothing is open yet).
async fn handle_picker_navigation(app: &mut App, key: KeyEvent) {
    let create_row = app.libraries.len(); // index of the "Create new..." entry
    match key.code {
        KeyCode::Char('q') => app.quit_confirmed = true,
        KeyCode::Up | KeyCode::Char('k') => {
            app.picker_index = app.picker_index.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.picker_index < create_row {
                app.picker_index += 1;
            }
        }
        KeyCode::Enter => {
            if app.picker_index >= create_row {
                app.new_lib_path.clear();
                app.new_lib_name.clear();
                app.new_lib_focus = app::NewLibField::Path;
                app.current_screen = app::Screens::CreateLibrary;
            } else if let Some(lib) = app.libraries.get(app.picker_index).cloned() {
                if let Err(e) = app.open_library(lib).await {
                    app.set_status(StatusLevel::Error, format!("Open failed: {e}"));
                }
            }
        }
        _ => {}
    }
}

/// Create-library form: edit the Path/Name fields, Enter validates and creates,
/// Esc backs out to the picker (or quits when there are no libraries yet).
async fn handle_create_navigation(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.refresh_libraries().await.ok();
            if app.libraries.is_empty() {
                app.quit_confirmed = true;
            } else {
                app.current_screen = app::Screens::Picker;
            }
        }
        KeyCode::Tab | KeyCode::Up | KeyCode::Down => {
            app.new_lib_focus = match app.new_lib_focus {
                app::NewLibField::Path => app::NewLibField::Name,
                app::NewLibField::Name => app::NewLibField::Path,
            };
        }
        KeyCode::Backspace => match app.new_lib_focus {
            app::NewLibField::Path => {
                app.new_lib_path.pop();
            }
            app::NewLibField::Name => {
                app.new_lib_name.pop();
            }
        },
        KeyCode::Char(c) => match app.new_lib_focus {
            app::NewLibField::Path => app.new_lib_path.push(c),
            app::NewLibField::Name => app.new_lib_name.push(c),
        },
        KeyCode::Enter => create_library_from_form(app).await,
        _ => {}
    }
}

/// Validates the create form and, on success, creates the library and opens it.
async fn create_library_from_form(app: &mut App) {
    let path = app.new_lib_path.trim().to_string();
    let name = app.new_lib_name.trim().to_string();
    if path.is_empty() || name.is_empty() {
        app.set_status(StatusLevel::Warning, "Both a path and a name are required.");
        return;
    }
    if !std::path::Path::new(&path).is_dir() {
        app.set_status(StatusLevel::Error, format!("Not a directory: {path}"));
        return;
    }
    // Friendlier than letting the UNIQUE(path) constraint surface as an error.
    if matches!(
        db::find_library_by_path(&app.pool, &path).await,
        Ok(Some(_))
    ) {
        app.set_status(
            StatusLevel::Warning,
            "A library already exists for that path.",
        );
        return;
    }
    match db::create_library(&app.pool, &name, &path).await {
        Ok(lib) => match app.open_library(lib).await {
            Ok(()) => app.set_status(StatusLevel::Success, format!("Created library \"{name}\".")),
            Err(e) => app.set_status(StatusLevel::Error, format!("Open failed: {e}")),
        },
        Err(e) => app.set_status(
            StatusLevel::Error,
            format!("Create failed (name already taken?): {e}"),
        ),
    }
}

async fn handle_start_navigation(app: &mut App, key: KeyEvent) {
    let library_id = app.active_library_id();

    if key.code == KeyCode::Char('q') && !app.search_mode {
        if app.should_quit {
            app.quit_confirmed = true;
        }
        app.should_quit = true;
    }
    if key.code == KeyCode::Esc && app.should_quit {
        app.should_quit = false;
    }

    // [L] switch libraries: back to the picker without restarting.
    if key.code == KeyCode::Char('L') {
        app.refresh_libraries().await.ok();
        app.current_screen = app::Screens::Picker;
        return;
    }

    if key.code == KeyCode::Char('s') {
        if let Some(ref path) = app.pending_scan_path {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let cancel = app.begin_cancelable();
            app.scan_receiver = Some(rx);
            app.scan_label = "Scanning";
            tokio::spawn(scan_library(
                app.pool.clone(),
                library_id,
                path.clone(),
                app.config.formats.clone(),
                app.config.scan_concurrency,
                cancel,
                tx,
            ));
            app.current_screen = app::Screens::Scanning;
        } else {
            app.set_status(StatusLevel::Warning, "Path not provided.");
        }
    }

    // [h] integrity / health check across the whole library.
    if key.code == KeyCode::Char('h') {
        let (tx, rx) = tokio::sync::mpsc::channel::<ScanEvent>(100);
        let cancel = app.begin_cancelable();
        app.scan_receiver = Some(rx);
        app.scan_label = "Health check";
        let threshold = app.config.low_bitrate_threshold;
        let concurrency = app.config.scan_concurrency;
        tokio::spawn(health_check(
            app.pool.clone(),
            library_id,
            threshold,
            concurrency,
            cancel,
            tx,
        ));
        app.current_screen = app::Screens::Scanning;
    }

    // [u] incremental rescan: re-read tags for files changed since last scan.
    if key.code == KeyCode::Char('u') {
        let (tx, rx) = tokio::sync::mpsc::channel::<ScanEvent>(100);
        let cancel = app.begin_cancelable();
        app.scan_receiver = Some(rx);
        app.scan_label = "Rescanning";
        tokio::spawn(rescan_changed(
            app.pool.clone(),
            library_id,
            app.config.scan_concurrency,
            cancel,
            tx,
        ));
        app.current_screen = app::Screens::Scanning;
    }

    if key.code == KeyCode::Char('r') {
        if let Some(ref path) = app.pending_scan_path {
            db::truncate_tracks(&app.pool, library_id).await.ok();
            let (tx, rx) = tokio::sync::mpsc::channel::<ScanEvent>(100);
            let cancel = app.begin_cancelable();
            app.scan_receiver = Some(rx);
            app.scan_label = "Scanning";
            tokio::spawn(scan_library(
                app.pool.clone(),
                library_id,
                path.clone(),
                app.config.formats.clone(),
                app.config.scan_concurrency,
                cancel,
                tx,
            ));
            app.current_screen = app::Screens::Scanning;
        } else {
            app.set_status(StatusLevel::Warning, "Path not provided.");
        }
    }

    if key.code == KeyCode::Char('v') {
        let (tx, rx) = tokio::sync::oneshot::channel::<ValidateEvent>();
        app.is_validating = true;
        app.validating_receiver = Some(rx);
        tokio::spawn(validate_paths(
            app.pool.clone(),
            library_id,
            app.config.scan_concurrency,
            tx,
        ));
    }

    if key.code == KeyCode::Enter {
        app.current_screen = app::Screens::Main;
    }
}

fn mark_search_dirty(app: &mut App) {
    app.search_dirty = true;
    app.search_last_edit = std::time::Instant::now();
}

/// True when the Library tab is showing the grouped artist/album browse view.
fn library_grouped(app: &App) -> bool {
    app.tabs[app.current_tab].label == "Library" && app.group_mode != app::GroupMode::Off
}

/// Cycles through `options` then back to `None`: None -> first -> ... -> last -> None.
fn next_in_cycle(options: &[String], current: Option<&str>) -> Option<String> {
    match current {
        None => options.first().cloned(),
        Some(cur) => match options.iter().position(|o| o == cur) {
            Some(i) if i + 1 < options.len() => Some(options[i + 1].clone()),
            _ => None,
        },
    }
}

/// Handles keys while the tag editor is open.
async fn handle_edit_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.edit = None;
            app.set_status(StatusLevel::Info, "Edit cancelled.");
        }
        KeyCode::Up => {
            if let Some(e) = &mut app.edit {
                e.focus_prev();
            }
        }
        KeyCode::Down | KeyCode::Tab => {
            if let Some(e) = &mut app.edit {
                e.focus_next();
            }
        }
        KeyCode::Backspace => {
            if let Some(e) = &mut app.edit {
                e.backspace();
            }
        }
        KeyCode::Enter => save_edits(app).await,
        KeyCode::Char(c) => {
            if let Some(e) = &mut app.edit {
                e.push_char(c);
            }
        }
        _ => {}
    }
}

/// Writes the in-progress edit to the audio file and the database.
async fn save_edits(app: &mut App) {
    let Some(edit) = app.edit.take() else {
        return;
    };
    let edits = edit.to_tag_edits();
    let id = edit.track_id;
    let path = edit.file_path.clone();

    // File IO is blocking; run it off the async runtime.
    let edits_for_file = edits.clone();
    let res =
        tokio::task::spawn_blocking(move || crate::track::write_tags(&path, &edits_for_file)).await;

    match res {
        Ok(Ok(())) => {
            db::update_track_tags(&app.pool, id, &edits).await.ok();
            app.reload().await.ok();
            app.set_status(StatusLevel::Success, "Tags saved to file + database.");
        }
        Ok(Err(e)) => app.set_status(StatusLevel::Error, format!("Tag write failed: {e}")),
        Err(e) => app.set_status(StatusLevel::Error, format!("Edit task panicked: {e}")),
    }
}

/// Handles keys while the export picker is open.
async fn handle_export_mode(app: &mut App, key: KeyEvent) {
    use crate::export;

    let label = app.tabs[app.current_tab].label;
    let tracks = &app.tabs[app.current_tab].tracks;

    let result: Option<(
        std::path::PathBuf,
        Result<(), Box<dyn std::error::Error + Send + Sync>>,
    )> = match key.code {
        KeyCode::Esc => {
            app.export_mode = false;
            return;
        }
        KeyCode::Char('c') => {
            let p = export::export_path(label, "csv");
            let r = export::export_csv(tracks, &p);
            Some((p, r))
        }
        KeyCode::Char('j') => {
            let p = export::export_path(label, "json");
            let r = export::export_json(tracks, &p);
            Some((p, r))
        }
        KeyCode::Char('m') => {
            let p = export::export_path(label, "m3u");
            let r = export::export_m3u(tracks, &p);
            Some((p, r))
        }
        KeyCode::Char('d') => {
            let rows = build_duplicate_report(app).await;
            let p = export::export_path("duplicates", "csv");
            let r = export::export_duplicate_report(&rows, &p);
            Some((p, r))
        }
        _ => None,
    };

    if let Some((path, outcome)) = result {
        app.export_mode = false;
        match outcome {
            Ok(()) => app.set_status(
                StatusLevel::Success,
                format!("Exported → {}", path.display()),
            ),
            Err(e) => app.set_status(StatusLevel::Error, format!("Export failed: {e}")),
        }
    }
}

/// Collects every duplicate group's members into report rows.
async fn build_duplicate_report(app: &App) -> Vec<crate::export::DuplicateReportRow> {
    let mut rows = Vec::new();
    for group in &app.duplicates.groups {
        let members =
            db::load_duplicate_members(&app.pool, group.kind, &group.key, app.active_library_id())
                .await
                .unwrap_or_default();
        for m in members {
            rows.push(crate::export::DuplicateReportRow {
                group_kind: group.kind.label(),
                group_key: group.key.clone(),
                title: m.title.unwrap_or_default(),
                artist: m.artist.unwrap_or_default(),
                album: m.album.unwrap_or_default(),
                bitrate: m.bitrate.map(|b| b.to_string()).unwrap_or_default(),
                size_bytes: m.file_size.map(|s| s.to_string()).unwrap_or_default(),
                file_path: m.file_path.display().to_string(),
            });
        }
    }
    rows
}

/// Handles keys while the filter popup is open. Each adjusts the active tab's
/// facet filters and reloads.
async fn handle_filter_mode(app: &mut App, key: KeyEvent) {
    let mut changed = true;
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('f') => {
            app.filter_mode = false;
            return;
        }
        KeyCode::Char('b') => {
            let f = &mut app.current_tab_mut().filter;
            f.min_bitrate = match f.min_bitrate {
                None => Some(128),
                Some(128) => Some(256),
                Some(256) => Some(320),
                _ => None,
            };
        }
        KeyCode::Char('t') => {
            let formats = app.formats_in_library.clone();
            let f = &mut app.current_tab_mut().filter;
            f.format = next_in_cycle(&formats, f.format.as_deref());
        }
        KeyCode::Char('i') => app.current_tab_mut().filter.no_isrc ^= true,
        KeyCode::Char('h') => app.current_tab_mut().filter.unhealthy ^= true,
        KeyCode::Char('c') => app.current_tab_mut().filter.clear(),
        _ => changed = false,
    }
    if changed {
        app.reload_current_tab().await.ok();
    }
}

async fn handle_main_navigation(app: &mut App, key: KeyEvent) {
    // Help overlay swallows all keys except its dismiss keys.
    if app.help_open {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
        ) {
            app.help_open = false;
        }
        return;
    }

    // While enrichment runs, Esc requests cancellation.
    if app.is_enriching && key.code == KeyCode::Esc {
        app.request_cancel();
        return;
    }

    // Tag editor captures keys while open.
    if app.edit.is_some() {
        handle_edit_mode(app, key).await;
        return;
    }

    // Export format picker captures keys while open.
    if app.export_mode {
        handle_export_mode(app, key).await;
        return;
    }

    // Filter popup mode: keys adjust the active tab's facets.
    if app.filter_mode {
        handle_filter_mode(app, key).await;
        return;
    }

    // Search editing mode.
    if app.search_mode {
        match key.code {
            KeyCode::Backspace => {
                app.current_tab_mut().search_query.pop();
                mark_search_dirty(app);
            }
            KeyCode::Esc => {
                app.search_mode = false;
                app.search_dirty = false;
                app.current_tab_mut().search_query.clear();
                app.reload_current_tab().await.ok();
            }
            KeyCode::Enter => {
                app.search_mode = false;
                app.search_dirty = false;
                app.reload_current_tab().await.ok();
            }
            KeyCode::Char(c) => {
                app.current_tab_mut().search_query.push(c);
                mark_search_dirty(app);
            }
            _ => {}
        }
        return;
    }

    // [L] switch libraries: back to the picker without restarting.
    if key.code == KeyCode::Char('L') {
        app.refresh_libraries().await.ok();
        app.current_screen = app::Screens::Picker;
        return;
    }

    // Enter search mode (not meaningful on the Duplicates grid).
    if key.code == KeyCode::Char('/') && app.tabs[app.current_tab].label != "Duplicates" {
        app.search_mode = true;
        return;
    }

    // Open help / filter popups.
    if key.code == KeyCode::Char('?') {
        app.help_open = true;
        return;
    }
    if key.code == KeyCode::Char('f') && app.tabs[app.current_tab].label != "Duplicates" {
        app.filter_mode = true;
        return;
    }

    // [w] toggle background watch mode.
    if key.code == KeyCode::Char('w') {
        app.toggle_watch();
        return;
    }

    // [z] undo the most recent reversible action.
    if key.code == KeyCode::Char('z') {
        match db::take_last_undo(&app.pool).await {
            Ok(Some((log_id, summary, action))) => match action.apply(&app.pool).await {
                Ok(_) => {
                    db::mark_undone(&app.pool, log_id).await.ok();
                    app.reload().await.ok();
                    app.set_status(StatusLevel::Success, format!("Undid: {summary}"));
                }
                Err(e) => app.set_status(StatusLevel::Error, format!("Undo failed: {e}")),
            },
            Ok(None) => app.set_status(StatusLevel::Info, "Nothing to undo."),
            Err(e) => app.set_status(StatusLevel::Error, format!("Undo failed: {e}")),
        }
        return;
    }

    // [x] open the export format picker.
    if key.code == KeyCode::Char('x') {
        app.export_mode = true;
        return;
    }

    // [c] open the statistics screen.
    if key.code == KeyCode::Char('c') {
        match db::compute_stats(&app.pool, app.active_library_id()).await {
            Ok(stats) => {
                app.stats = Some(stats);
                app.current_screen = app::Screens::Stats;
            }
            Err(e) => app.set_status(StatusLevel::Error, format!("Stats failed: {e}")),
        }
        return;
    }

    // [m] edit the tags of the highlighted track.
    if key.code == KeyCode::Char('m') && app.tabs[app.current_tab].label != "Duplicates" {
        load_selected_track(app).await;
        match app
            .properties_of_track
            .as_ref()
            .and_then(app::EditState::from_track)
        {
            Some(state) => app.edit = Some(state),
            None => app.set_status(StatusLevel::Warning, "No track selected to edit."),
        }
        return;
    }

    // [g] cycle grouped browse (Library): off -> artist -> album -> off.
    if key.code == KeyCode::Char('g') {
        if app.tabs[app.current_tab].label == "Library" {
            app.group_mode = app.group_mode.next();
            app.refresh_groups().await.ok();
            let label = app.group_mode.label();
            app.set_status(StatusLevel::Info, format!("Group by: {label}"));
        }
        return;
    }

    // [Enter] in grouped view drills into the selected group via search.
    if key.code == KeyCode::Enter && library_grouped(app) {
        if let Some(group) = app.groups_state.selected().and_then(|i| app.groups.get(i)) {
            let name = group.name.clone();
            app.group_mode = app::GroupMode::Off;
            let tab = app.current_tab_mut();
            tab.search_query = name.clone();
            app.reload_current_tab().await.ok();
            app.set_status(StatusLevel::Info, format!("Filtered to: {name}"));
        }
        return;
    }

    if key.code == KeyCode::Char('p') {
        if app.tabs[app.current_tab].label == "Duplicates" {
            match app.duplicates.focus {
                app::DuplicatePane::Groups => app.duplicates.focus = app::DuplicatePane::Members,
                app::DuplicatePane::Members => app.duplicates.focus = app::DuplicatePane::Groups,
            }
            return;
        }

        app.properties_panel_open = true;
        load_selected_track(app).await;
    }

    if key.code == KeyCode::Up {
        // highlight previous track

        if library_grouped(app) {
            let cur = app.groups_state.selected().unwrap_or(0);
            app.groups_state.select(Some(cur.saturating_sub(1)));
            return;
        }

        if app.tabs[app.current_tab].label == "Duplicates" {
            match app.duplicates.focus {
                app::DuplicatePane::Groups => {
                    let cur = app.duplicates.groups_state.selected().unwrap_or(0);
                    let new_idx = (cur.saturating_sub(1)).max(0);
                    if new_idx != cur {
                        let pool = app.pool.clone();
                        app.duplicates.select_group(&pool, new_idx).await.ok();
                    }
                }
                app::DuplicatePane::Members => app.duplicates.select_member(-1),
            }
            return;
        }

        let active_tab_state = &mut app.tabs[app.current_tab].state;
        if active_tab_state.selected().is_some() {
            active_tab_state.select_previous();
        }
        if app.properties_panel_open {
            load_selected_track(app).await;
        }
    }

    if key.code == KeyCode::Down {
        // highlight next track

        if library_grouped(app) {
            let cur = app.groups_state.selected().unwrap_or(0);
            let max = app.groups.len().saturating_sub(1);
            app.groups_state.select(Some((cur + 1).min(max)));
            return;
        }

        if app.tabs[app.current_tab].label == "Duplicates" {
            match app.duplicates.focus {
                app::DuplicatePane::Groups => {
                    let cur = app.duplicates.groups_state.selected().unwrap_or(0);
                    let max = app.duplicates.groups.len().saturating_sub(1);
                    let new_idx = (cur.saturating_add(1)).min(max);
                    if new_idx != cur {
                        let pool = app.pool.clone();
                        app.duplicates.select_group(&pool, new_idx).await.ok();
                    }
                }
                app::DuplicatePane::Members => app.duplicates.select_member(1),
            }
            return;
        }

        let active_tab_state = &mut app.tabs[app.current_tab].state;
        if active_tab_state.selected().is_some() {
            active_tab_state.select_next();
        }
        if app.properties_panel_open {
            load_selected_track(app).await;
        }
    }

    // Left/Right also move between member columns in the Duplicates grid.
    if app.tabs[app.current_tab].label == "Duplicates"
        && matches!(app.duplicates.focus, app::DuplicatePane::Members)
    {
        if key.code == KeyCode::Left {
            app.duplicates.select_member(-1);
            return;
        }
        if key.code == KeyCode::Right {
            app.duplicates.select_member(1);
            return;
        }
    }

    // cycle app tabs
    if key.code == KeyCode::Tab {
        app.pending_purge = false;

        // is saturating_add completely unnecessary here? YES!
        // is it cool to include? ABSOLUTELY!
        app.current_tab = (usize::saturating_add(app.current_tab, 1)) % app.tabs.len();
        if app.properties_panel_open {
            load_selected_track(app).await;
        }
    }

    // [s] cycle sort field, [S] toggle direction (track-list tabs only)
    if app.tabs[app.current_tab].label != "Duplicates" {
        if key.code == KeyCode::Char('s') {
            app.tabs[app.current_tab].cycle_sort();
            let t = &app.tabs[app.current_tab];
            let arrow = if t.sort_desc { "↓" } else { "↑" };
            let msg = format!("Sort: {} {arrow}", t.sort.label());
            app.set_status(StatusLevel::Info, msg);
            return;
        }
        if key.code == KeyCode::Char('S') {
            app.tabs[app.current_tab].toggle_sort_dir();
            let t = &app.tabs[app.current_tab];
            let arrow = if t.sort_desc { "↓" } else { "↑" };
            app.set_status(
                StatusLevel::Info,
                format!("Sort: {} {arrow}", t.sort.label()),
            );
            return;
        }
    }

    // handle toggle track row selection
    if key.code == KeyCode::Char(' ') {
        let active_tab_data = &mut app.tabs[app.current_tab];

        if let Some(idx) = active_tab_data.state.selected() {
            active_tab_data.tracks[idx].is_selected = !active_tab_data.tracks[idx].is_selected;
        }
    }

    // handle SELECT ALL
    if key.code == KeyCode::Char('i') {
        app.tabs[app.current_tab]
            .tracks
            .iter_mut()
            .for_each(|t| t.is_selected = true);
    }

    // handle DESELECT ALL
    if key.code == KeyCode::Char('o') {
        app.tabs[app.current_tab]
            .tracks
            .iter_mut()
            .for_each(|t| t.is_selected = false);
    }

    // handle INVERT SELECTION
    if key.code == KeyCode::Char('u') {
        app.tabs[app.current_tab]
            .tracks
            .iter_mut()
            .for_each(|t| t.is_selected = !t.is_selected);
    }

    // [d] - In the Trash tab this confirms + purges flagged tracks from disk.
    // Everywhere else it flags the selected (or highlighted) tracks into Trash,
    // which is reversible until purged.
    if key.code == KeyCode::Char('d') {
        match app.tabs[app.current_tab].label {
            "Trash" => {
                if app.pending_purge {
                    purge_marked(app).await;
                    app.pending_purge = false;
                } else if !app.tabs[app.current_tab].tracks.is_empty() {
                    app.pending_purge = true;
                }
            }
            // Duplicates are resolved by choosing a keeper ([k]), not deleting.
            "Duplicates" => {}
            _ => {
                let ids = target_ids(&app.tabs[app.current_tab]);
                for id in &ids {
                    db::set_marked_for_deletion(&app.pool, *id, true).await.ok();
                }
                if !ids.is_empty() {
                    let n = ids.len();
                    let action = crate::undo::UndoAction::SetMarked {
                        ids: ids.clone(),
                        value: false,
                    };
                    db::log_action(&app.pool, "flag", &format!("Flagged {n} track(s)"), &action)
                        .await
                        .ok();
                    app.reload().await.ok();
                    app.set_status(
                        StatusLevel::Success,
                        format!("Flagged {n} track(s) for deletion."),
                    );
                }
            }
        }
        return;
    }

    // [r] - restore from Trash, or retry enrichment dead letters.
    if key.code == KeyCode::Char('r') {
        match app.tabs[app.current_tab].label {
            "Trash" => {
                let ids = target_ids(&app.tabs[app.current_tab]);
                for id in &ids {
                    db::set_marked_for_deletion(&app.pool, *id, false)
                        .await
                        .ok();
                }
                if !ids.is_empty() {
                    let action = crate::undo::UndoAction::SetMarked {
                        ids: ids.clone(),
                        value: true,
                    };
                    db::log_action(
                        &app.pool,
                        "restore",
                        &format!("Restored {} track(s)", ids.len()),
                        &action,
                    )
                    .await
                    .ok();
                    app.reload().await.ok();
                    app.set_status(
                        StatusLevel::Success,
                        format!("Restored {} track(s).", ids.len()),
                    );
                }
            }
            "Failed" => {
                // Capture previous statuses, then reset to pending.
                let tab = &app.tabs[app.current_tab];
                let ids = target_ids(tab);
                let items: Vec<(i64, String)> = tab
                    .tracks
                    .iter()
                    .filter(|t| t.id.map(|id| ids.contains(&id)).unwrap_or(false))
                    .filter_map(|t| t.id.map(|id| (id, t.status.clone())))
                    .collect();
                for id in &ids {
                    db::update_track_status(&app.pool, *id, "pending")
                        .await
                        .ok();
                }
                if !ids.is_empty() {
                    let action = crate::undo::UndoAction::SetStatus { items };
                    db::log_action(
                        &app.pool,
                        "retry",
                        &format!("Re-queued {} track(s)", ids.len()),
                        &action,
                    )
                    .await
                    .ok();
                    app.reload().await.ok();
                    app.set_status(
                        StatusLevel::Success,
                        format!("Re-queued {} track(s) for enrichment.", ids.len()),
                    );
                }
            }
            _ => {}
        }
        return;
    }

    // [k] - in the Duplicates tab, keep the highlighted member and flag the
    // rest of the group into Trash.
    if key.code == KeyCode::Char('k') {
        if app.tabs[app.current_tab].label == "Duplicates" {
            let pool = app.pool.clone();
            if let Ok(flagged) = app.duplicates.keep_selected(&pool).await {
                if !flagged.is_empty() {
                    let n = flagged.len();
                    let action = crate::undo::UndoAction::SetMarked {
                        ids: flagged,
                        value: false,
                    };
                    db::log_action(
                        &app.pool,
                        "keep",
                        &format!("Resolved duplicate ({n} flagged)"),
                        &action,
                    )
                    .await
                    .ok();
                }
            }
            app.reload().await.ok();
        }
        return;
    }

    // [Shift+K] in the Duplicates tab, dismiss the current group without
    // flagging anything. All members are kept; the group is simply hidden.
    if key.code == KeyCode::Char('K') {
        if app.tabs[app.current_tab].label == "Duplicates" {
            app.duplicates.skip_all(&app.pool).await.ok();
            app.set_status(
                StatusLevel::Success,
                "Duplicate group dismissed (all files kept).",
            );
            app.reload().await.ok();
        }
        return;
    }

    // [e] - enrich the checkbox-selected tracks (or the highlighted one) on the
    // Enrichment tab. Use [i] to select all first to enrich every pending track.
    if key.code == KeyCode::Char('e') {
        if app.tabs[app.current_tab].label == "Enrichment" && !app.is_enriching {
            if let Some(api_key) = app.config.resolved_acoustid_key.clone() {
                let ids = target_ids(&app.tabs[app.current_tab]);
                if ids.is_empty() {
                    app.set_status(StatusLevel::Warning, "No tracks to enrich.");
                    return;
                }
                let (tx, rx) = tokio::sync::mpsc::channel(8);
                let cancel = app.begin_cancelable();
                app.enrich_receiver = Some(rx);
                app.is_enriching = true;
                app.enrich_progress = Some((0, ids.len()));
                app.set_status(
                    StatusLevel::Info,
                    format!("Enriching {} track(s)…", ids.len()),
                );
                tokio::spawn(enrich_pending(
                    app.pool.clone(),
                    app.active_library_id(),
                    ids,
                    api_key,
                    cancel,
                    tx,
                ));
            } else {
                app.set_status(
                    StatusLevel::Error,
                    "No AcoustID API key set (ACOUSTID_API_KEY in .env).",
                );
            }
        }
        return;
    }

    // [PageUp]/[PageDown] - scroll the properties panel when it is open.
    if app.properties_panel_open {
        if key.code == KeyCode::PageDown {
            app.properties_scroll = app.properties_scroll.saturating_add(5);
            return;
        }
        if key.code == KeyCode::PageUp {
            app.properties_scroll = app.properties_scroll.saturating_sub(5);
            return;
        }
    }

    if key.code == KeyCode::Char('q') && !app.search_mode {
        if app.should_quit {
            app.quit_confirmed = true;
        }
        app.should_quit = true;
        return;
    }
    // DO NOT PUT ANY HANDLERS BELOW THIS ONE
    if key.code == KeyCode::Esc {
        if app.should_quit {
            app.should_quit = false;
            return;
        }
        if app.pending_purge {
            app.pending_purge = false;
            return;
        }
        // Clear an active search/filter before leaving the screen.
        let tab = &app.tabs[app.current_tab];
        if !tab.search_query.is_empty() || tab.filter.is_active() {
            let tab = app.current_tab_mut();
            tab.search_query.clear();
            tab.filter.clear();
            app.reload_current_tab().await.ok();
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
    let active_tab_data = &app.tabs[app.current_tab];

    let selected_track_db_id = active_tab_data
        .state
        .selected()
        .and_then(|idx| active_tab_data.tracks.get(idx))
        .and_then(|t| t.id)
        .unwrap_or_default();

    app.properties_of_track = load_track_full(&app.pool, selected_track_db_id)
        .await
        .ok()
        .flatten();
    // Reset scroll so each newly viewed track starts from the top.
    app.properties_scroll = 0;
}

fn draw_tab_bar(f: &mut Frame, app: &mut App, area: Rect) {
    app.tab_bar_area = area;
    let titles: Vec<&str> = app.tabs.iter().map(|t| t.label).collect();
    let mut block = Block::default().borders(Borders::ALL).light_green();
    if app.watch_enabled {
        block = block.title_top(Line::from(" ● WATCH ").light_cyan().bold().right_aligned());
    }
    let tab_bar = ratatui::widgets::Tabs::new(titles)
        .select(app.current_tab)
        .block(block)
        .highlight_style(Style::default().bold().light_green().reversed());
    f.render_widget(tab_bar, area);
}

/// One-line transient status bar shown at the bottom of the main screen.
fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let Some(msg) = &app.status_message else {
        return;
    };
    let (prefix, style) = match msg.level {
        StatusLevel::Info => ("🛈", Style::default().light_blue()),
        StatusLevel::Success => ("✓", Style::default().light_green()),
        StatusLevel::Warning => ("!", Style::default().light_yellow()),
        StatusLevel::Error => ("✗", Style::default().light_red()),
    };
    let line = Line::from(vec![
        Span::styled(format!(" {prefix} "), style.add_modifier(Modifier::BOLD)),
        Span::styled(msg.text.clone(), style),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_shortcuts_row(f: &mut Frame, app: &mut App, area: Rect) {
    let text = match app.tabs[app.current_tab].label {
        "Duplicates" => "[TAB] tabs · [P] pane · [↑/↓] nav · [K]eep one · shift+[K]eep all · [?] help",
        "Enrichment" => {
            "[TAB] tabs · [E]nrich · [R]etry · [SPACE]lect · [D] flag · [M] edit · [/] search · [?] help"
        }
        "Trash" => "[TAB] tabs · [SPACE]lect · [R]estore · [D] purge · [Z] undo · [?] help",
        "Failed" => {
            "[TAB] tabs · [R]etry enrichment · [SPACE]lect · [M] edit · [D] flag · [?] help"
        }
        _ => {
            "[TAB] tabs · [/] search · [F]ilter · [S]ort · [G]roup · [M] edit · [X] export · [C] stats · [D] flag · [?] help"
        }
    };

    f.render_widget(Paragraph::new(text).blue(), area);
}

fn draw_search_bar(f: &mut Frame, app: &mut App, area: Rect) {
    let query = app.tabs[app.current_tab].search_query.clone();
    let block = if app.search_mode {
        Paragraph::new(format!("{query}█"))
            .block(Block::bordered())
            .light_yellow()
    } else if !query.is_empty() {
        Paragraph::new(format!("search: {query}    ([/] edit · [esc] clear)"))
            .block(Block::bordered())
            .light_cyan()
    } else {
        Paragraph::new("press [/] to search · [f]ilter · [?] help")
            .block(Block::bordered())
            .gray()
    };

    f.render_widget(block, area);
}

fn draw_table_content(f: &mut Frame, app: &mut App, area: Rect) {
    app.table_area = area;

    if library_grouped(app) {
        draw_group_view(f, app, area);
        return;
    }

    let active_tab_data = &mut app.tabs[app.current_tab];

    if active_tab_data.label == "Duplicates" {
        draw_duplicates_panel(f, app, area);
        return;
    }

    if active_tab_data.tracks.len() == 0 {
        f.render_widget(
            Paragraph::new(format!("No {} Tracks Found", active_tab_data.label)).light_magenta(),
            area,
        );
        return;
    }

    let items = active_tab_data.tracks.iter().map(|i| {
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
            // Surface a health problem (red) in place of the status when present.
            match &i.health_issue {
                Some(issue) => Cell::new(format!("⚠ {issue}")).style(Style::default().light_red()),
                None => Cell::new(i.status.as_str()),
            },
        ]);
        if i.is_selected {
            row.style(Style::default().light_green())
        } else {
            row
        }
    });

    let list = Table::new(
        items,
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
                    "{}: [{}/{}] - Selected: [{}/{}]",
                    active_tab_data.label,
                    active_tab_data.state.selected().unwrap_or(0) + 1,
                    active_tab_data.tracks.len(),
                    active_tab_data
                        .tracks
                        .iter()
                        .filter(|f| f.is_selected)
                        .count(),
                    active_tab_data.tracks.len(),
                )
                .light_magenta(),
            )
            .title_bottom(
                Line::from({
                    let arrow = if active_tab_data.sort_desc {
                        "↓"
                    } else {
                        "↑"
                    };
                    let mut s = format!(" sort: {} {arrow} ", active_tab_data.sort.label());
                    if active_tab_data.filter.is_active() {
                        s.push_str(&format!("| filter: {} ", active_tab_data.filter.describe()));
                    }
                    s
                })
                .blue()
                .right_aligned(),
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

    f.render_stateful_widget(list, area, &mut active_tab_data.state);

    // Vertical scrollbar reflecting the selection position within the list.
    let total = active_tab_data.tracks.len();
    if total > area.height.saturating_sub(3) as usize {
        let mut sb_state = ratatui::widgets::ScrollbarState::new(total)
            .position(active_tab_data.state.selected().unwrap_or(0));
        let scrollbar =
            ratatui::widgets::Scrollbar::new(ratatui::widgets::ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None);
        f.render_stateful_widget(
            scrollbar,
            area.inner(ratatui::layout::Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut sb_state,
        );
    }
}

fn draw_group_view(f: &mut Frame, app: &mut App, area: Rect) {
    if app.groups.is_empty() {
        f.render_widget(Paragraph::new("No groups").light_magenta(), area);
        return;
    }

    let rows = app.groups.iter().map(|g| {
        Row::new(vec![
            Cell::new(g.name.clone()),
            Cell::new(format!("{}", g.count)),
            Cell::new(format_size(g.total_size.max(0) as u64, DECIMAL)),
            Cell::new(
                format_track_duration(Some(g.total_duration.max(0) as u32)).unwrap_or_default(),
            ),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(60),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec![
            Cell::new("Name"),
            Cell::new("Tracks"),
            Cell::new("Size"),
            Cell::new("Duration"),
        ])
        .bold()
        .bottom_margin(1),
    )
    .block(
        Block::default()
            .title(
                format!(
                    "Library grouped by {} [{}/{}]",
                    app.group_mode.label(),
                    app.groups_state.selected().unwrap_or(0) + 1,
                    app.groups.len(),
                )
                .light_magenta(),
            )
            .title_bottom(
                Line::from(" [↑/↓] move · [enter] open · [g] cycle grouping ")
                    .blue()
                    .right_aligned(),
            )
            .borders(Borders::ALL),
    )
    .row_highlight_style(Style::default().reversed());

    f.render_stateful_widget(table, area, &mut app.groups_state);
}

fn draw_properties_panel(f: &mut Frame, app: &mut App, area: Rect) {
    if let Some(track) = &app.properties_of_track {
        let block = Block::default()
            .title(Line::from("Track Properties").bold().light_magenta())
            .title_bottom(Line::from("[PgUp/PgDn] scroll").blue().right_aligned())
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

        // Clamp scroll so it can't run past the end of the content.
        let max_scroll = (lines.len() as u16).saturating_sub(inner.height);
        let scroll = app.properties_scroll.min(max_scroll);

        f.render_widget(block, area);
        f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
    }
}

fn draw_confirmation_prompt(
    f: &mut Frame,
    area: Rect,
    title: &'static str,
    message: &'static str,
    command_text: &'static str,
) {
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

    let block = Block::default().borders(Borders::ALL).title(title).red();
    let text = Paragraph::new(vec![
        Line::raw(""),
        Line::raw(""),
        Line::from(message).light_red().centered(),
        Line::raw(""),
        Line::from(command_text).light_red().centered(),
    ])
    .alignment(ratatui::layout::Alignment::Center)
    .centered()
    .block(block);
    f.render_widget(text, inner[1]);
}

fn draw_duplicates_panel(f: &mut Frame, app: &mut App, area: Rect) {
    if app.duplicates.groups.is_empty() {
        f.render_widget(Paragraph::new("No Duplicates Found").light_magenta(), area);
        return;
    }

    let panes = Layout::horizontal([
        Constraint::Fill(1), // group list
        Constraint::Fill(2), // members grid
    ])
    .split(area);

    draw_duplicate_groups_list(f, app, panes[0]);
    draw_duplicate_members_grid(f, app, panes[1]);
}

fn draw_duplicate_groups_list(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = app.duplicates.groups.iter().map(|g| {
        let kind = g.kind.label();
        Row::new(vec![
            Cell::new(format!("×{}", g.count)),
            Cell::new(kind),
            Cell::new(g.title.as_deref().unwrap_or("Unknown")),
            Cell::new(g.artist.as_deref().unwrap_or("Unknown")),
            Cell::new(g.album.as_deref().unwrap_or("Unknown")),
        ])
    });

    let focused = matches!(app.duplicates.focus, app::DuplicatePane::Groups);
    let border_style = if focused {
        Style::default().light_green()
    } else {
        Style::default()
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),      // ×N badge
            Constraint::Length(5),      // match kind (hash/isrc)
            Constraint::Percentage(38), // title
            Constraint::Percentage(24), // artist
            Constraint::Percentage(34), // album
        ],
    )
    .header(
        Row::new(vec![
            Cell::new("Cnt"),
            Cell::new("Kind"),
            Cell::new("Title"),
            Cell::new("Artist"),
            Cell::new("Album"),
        ])
        .bold()
        .bottom_margin(1),
    )
    .block(
        Block::default()
            .title(
                format!(
                    "Duplicate Groups: [{}/{}]",
                    app.duplicates.groups_state.selected().unwrap_or(0) + 1,
                    app.duplicates.groups.len(),
                )
                .light_magenta(),
            )
            .borders(Borders::ALL)
            .border_style(border_style),
    )
    .row_highlight_style(Style::default().reversed());

    f.render_stateful_widget(table, area, &mut app.duplicates.groups_state);
}

// Builds one tag-comparison row. The keeper-candidate column is highlighted so
// the user can see which file [k] will keep.
fn tag_row<F>(label: &str, members: &[TrackSummary], selected: usize, get: F) -> Row<'static>
where
    F: Fn(&TrackSummary) -> String,
{
    let mut cells = vec![Cell::new(label.to_string()).bold()];
    for (i, m) in members.iter().enumerate() {
        let cell = Cell::new(get(m));
        cells.push(if i == selected {
            cell.style(Style::default().light_green().reversed())
        } else {
            cell
        });
    }
    Row::new(cells)
}

fn draw_duplicate_members_grid(f: &mut Frame, app: &mut App, area: Rect) {
    let members = &app.duplicates.selected_members;

    if members.is_empty() {
        f.render_widget(Paragraph::new("No members loaded").light_magenta(), area);
        return;
    }

    let selected = app.duplicates.selected_member.min(members.len() - 1);

    let paths: Vec<std::path::PathBuf> = members.iter().map(|m| m.file_path.clone()).collect();
    let prefix = common_path_prefix(&paths);

    // Header row: "Tag" + one cell per file. The keeper candidate is starred.
    let mut header_cells = vec![Cell::new("Tag").bold()];
    for (i, m) in members.iter().enumerate() {
        let name = m
            .file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?");
        let star = if i == selected { "★ " } else { "" };
        let cell = Cell::new(format!("{}[{}] {}", star, i + 1, name)).bold();
        header_cells.push(if i == selected {
            cell.light_green()
        } else {
            cell
        });
    }

    let dash = || "-".to_string();

    let rows = vec![
        tag_row("Title", members, selected, |m| {
            m.title.clone().unwrap_or_else(dash)
        }),
        tag_row("Artist", members, selected, |m| {
            m.artist.clone().unwrap_or_else(dash)
        }),
        tag_row("Album", members, selected, |m| {
            m.album.clone().unwrap_or_else(dash)
        }),
        tag_row("Format", members, selected, |m| {
            m.file_format.clone().unwrap_or_else(dash)
        }),
        tag_row("Bitrate", members, selected, |m| {
            m.bitrate.map(|b| b.to_string()).unwrap_or_else(dash)
        }),
        tag_row("Size", members, selected, |m| {
            m.file_size
                .map(|s| format_size(s as u64, DECIMAL))
                .unwrap_or_else(dash)
        }),
        tag_row("Path", members, selected, move |m| {
            m.file_path
                .strip_prefix(&prefix)
                .unwrap_or(&m.file_path)
                .display()
                .to_string()
        }),
    ];

    let mut widths = vec![Constraint::Length(10)];
    for _ in members {
        widths.push(Constraint::Fill(1));
    }

    let focused = matches!(app.duplicates.focus, app::DuplicatePane::Members);
    let border_style = if focused {
        Style::default().light_green()
    } else {
        Style::default()
    };

    let title = format!(
        "Members [{}/{}] - ★ kept, rest → Trash ([k] to apply)",
        selected + 1,
        members.len(),
    );

    let table = Table::new(rows, widths)
        .header(Row::new(header_cells).bottom_margin(1))
        .block(
            Block::default()
                .title(title.light_magenta())
                .borders(Borders::ALL)
                .border_style(border_style),
        );

    f.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use sqlx::sqlite::SqlitePoolOptions;

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    // A single pinned connection keeps the in-memory schema alive for the whole
    // pool lifetime (each fresh connection would otherwise get its own DB).
    async fn test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn tab_index(app: &App, label: &str) -> usize {
        app.tabs.iter().position(|t| t.label == label).unwrap()
    }

    /// Assigns any orphan (raw-inserted) tracks to a default library, builds the
    /// app, and opens that library so the tabs are hydrated. Tests insert tracks
    /// before calling this.
    async fn open_default(pool: &sqlx::SqlitePool) -> App {
        db::ensure_default_library(pool, None).await.unwrap();
        let lib = db::list_libraries(pool)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let mut app = App::new(pool.clone(), Config::default()).await.unwrap();
        app.open_library(lib).await.unwrap();
        app
    }

    #[tokio::test]
    async fn d_flags_highlighted_track_into_trash() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO tracks (file_path, title, status) VALUES ('/x/a.flac','A','pending')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut app = open_default(&pool).await;
        app.current_screen = app::Screens::Main;
        app.current_tab = tab_index(&app, "Library");
        app.tabs[app.current_tab].state.select(Some(0));

        handle_main_navigation(&mut app, press('d')).await;

        assert_eq!(db::count_marked(&pool, app.active_library_id()).await.unwrap(), 1);
        // Flagging marks other tabs dirty; the Trash tab reloads lazily when it
        // becomes active (via `ensure_fresh`), then shows the flagged track.
        let trash = tab_index(&app, "Trash");
        app.current_tab = trash;
        app.ensure_fresh().await.unwrap();
        assert_eq!(app.tabs[trash].tracks.len(), 1);
    }

    #[tokio::test]
    async fn keep_flags_other_duplicate_group_members() {
        let pool = test_pool().await;
        // Two byte-identical files (same hash) -> one duplicate group of 2.
        for path in ["/x/a.flac", "/x/b.flac"] {
            sqlx::query(
                "INSERT INTO tracks (file_path, title, status, file_hash) VALUES (?, 'A', 'pending', 'deadbeef')",
            )
            .bind(path)
            .execute(&pool)
            .await
            .unwrap();
        }

        let mut app = open_default(&pool).await;
        app.current_screen = app::Screens::Main;
        app.current_tab = tab_index(&app, "Duplicates");

        assert_eq!(app.duplicates.groups.len(), 1);
        assert_eq!(app.duplicates.selected_members.len(), 2);

        handle_main_navigation(&mut app, press('k')).await;

        // Exactly one member (the non-keeper) is flagged, and the resolved
        // group drops off the list.
        assert_eq!(db::count_marked(&pool, app.active_library_id()).await.unwrap(), 1);
        assert_eq!(app.duplicates.groups.len(), 0);
    }

    #[tokio::test]
    async fn restore_unflags_track_in_trash() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO tracks (file_path, title, status, marked_for_deletion) VALUES ('/x/a.flac','A','pending',1)")
            .execute(&pool)
            .await
            .unwrap();

        let mut app = open_default(&pool).await;
        app.current_screen = app::Screens::Main;
        app.current_tab = tab_index(&app, "Trash");
        app.tabs[app.current_tab].state.select(Some(0));

        assert_eq!(db::count_marked(&pool, app.active_library_id()).await.unwrap(), 1);
        handle_main_navigation(&mut app, press('r')).await;
        assert_eq!(db::count_marked(&pool, app.active_library_id()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn undo_reverts_a_flag() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO tracks (file_path, title, status) VALUES ('/x/a.flac','A','pending')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut app = open_default(&pool).await;
        app.current_screen = app::Screens::Main;
        app.current_tab = tab_index(&app, "Library");
        app.tabs[app.current_tab].state.select(Some(0));

        handle_main_navigation(&mut app, press('d')).await;
        assert_eq!(db::count_marked(&pool, app.active_library_id()).await.unwrap(), 1);

        // Undo restores the flag.
        handle_main_navigation(&mut app, press('z')).await;
        assert_eq!(db::count_marked(&pool, app.active_library_id()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn sort_orders_tracks_by_title() {
        let pool = test_pool().await;
        for t in ["Zebra", "Apple", "Mango"] {
            sqlx::query("INSERT INTO tracks (file_path, title, status) VALUES (?, ?, 'pending')")
                .bind(format!("/x/{t}.flac"))
                .bind(t)
                .execute(&pool)
                .await
                .unwrap();
        }
        let mut app = open_default(&pool).await;
        let i = tab_index(&app, "Library");
        app.tabs[i].sort = app::SortKey::Title;
        app.tabs[i].apply_sort();
        let titles: Vec<&str> = app.tabs[i]
            .tracks
            .iter()
            .filter_map(|t| t.title.as_deref())
            .collect();
        assert_eq!(titles, vec!["Apple", "Mango", "Zebra"]);
    }

    #[tokio::test]
    async fn renders_every_screen_without_panic() {
        use ratatui::{Terminal, backend::TestBackend};

        let pool = test_pool().await;
        for (p, h) in [("/x/a.flac", "h1"), ("/x/b.flac", "h1")] {
            sqlx::query("INSERT INTO tracks (file_path, title, artist, album, status, file_hash, file_format) VALUES (?, 'T', 'Ar', 'Al', 'pending', ?, 'FLAC')")
                .bind(p)
                .bind(h)
                .execute(&pool)
                .await
                .unwrap();
        }

        let mut app = open_default(&pool).await;
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();

        // Start screen, then every main tab.
        term.draw(|f| draw(f, &mut app)).unwrap();
        app.current_screen = app::Screens::Main;
        for i in 0..app.tabs.len() {
            app.current_tab = i;
            term.draw(|f| draw(f, &mut app)).unwrap();
        }

        // Overlays and alternate views.
        app.current_tab = tab_index(&app, "Library");
        app.properties_panel_open = true;
        load_selected_track(&mut app).await;
        term.draw(|f| draw(f, &mut app)).unwrap();

        app.edit = app
            .properties_of_track
            .as_ref()
            .and_then(app::EditState::from_track);
        term.draw(|f| draw(f, &mut app)).unwrap();
        app.edit = None;

        app.help_open = true;
        app.filter_mode = true;
        app.export_mode = true;
        term.draw(|f| draw(f, &mut app)).unwrap();
        app.help_open = false;
        app.filter_mode = false;
        app.export_mode = false;

        app.group_mode = app::GroupMode::Artist;
        app.refresh_groups().await.unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        app.group_mode = app::GroupMode::Off;

        app.current_screen = app::Screens::Scanning;
        app.scan_progress = Some((1, 2));
        term.draw(|f| draw(f, &mut app)).unwrap();

        let lib_id = app.active_library_id();
        app.stats = Some(db::compute_stats(&pool, lib_id).await.unwrap());
        app.current_screen = app::Screens::Stats;
        term.draw(|f| draw(f, &mut app)).unwrap();

        // Library picker + create-library screens.
        app.refresh_libraries().await.unwrap();
        app.current_screen = app::Screens::Picker;
        term.draw(|f| draw(f, &mut app)).unwrap();
        app.current_screen = app::Screens::CreateLibrary;
        app.new_lib_path = "/music/new".to_string();
        app.new_lib_name = "New".to_string();
        term.draw(|f| draw(f, &mut app)).unwrap();
    }

    #[tokio::test]
    async fn libraries_scope_tracks() {
        let pool = test_pool().await;
        let a = db::create_library(&pool, "A", "/a").await.unwrap();
        let b = db::create_library(&pool, "B", "/b").await.unwrap();
        sqlx::query("INSERT INTO tracks (file_path, title, status, library_id) VALUES ('/a/x.flac','X','pending',?)")
            .bind(a.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO tracks (file_path, title, status, library_id) VALUES ('/b/y.flac','Y','pending',?)")
            .bind(b.id)
            .execute(&pool)
            .await
            .unwrap();

        // Each library sees only its own track — no cross-library bleed.
        assert_eq!(db::count_tracks(&pool, None, a.id).await.unwrap(), 1);
        assert_eq!(db::count_tracks(&pool, None, b.id).await.unwrap(), 1);
        let a_tracks = db::load_tracks(&pool, None, None, None, a.id).await.unwrap();
        assert_eq!(a_tracks.len(), 1);
        assert_eq!(a_tracks[0].title.as_deref(), Some("X"));

        // Opening switches the active library and re-hydrates the Library tab.
        let mut app = App::new(pool.clone(), Config::default()).await.unwrap();
        app.open_library(b.clone()).await.unwrap();
        let lib_tab = tab_index(&app, "Library");
        assert_eq!(app.tabs[lib_tab].tracks.len(), 1);
        assert_eq!(app.tabs[lib_tab].tracks[0].title.as_deref(), Some("Y"));
    }

    #[tokio::test]
    async fn stats_and_csv_export() {
        let pool = test_pool().await;
        for t in ["A", "B"] {
            sqlx::query("INSERT INTO tracks (file_path, title, status, file_format, file_size) VALUES (?, ?, 'pending', 'FLAC', 1000)")
                .bind(format!("/x/{t}.flac"))
                .bind(t)
                .execute(&pool)
                .await
                .unwrap();
        }
        db::ensure_default_library(&pool, None).await.unwrap();
        let lib = db::list_libraries(&pool)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let stats = db::compute_stats(&pool, lib.id).await.unwrap();
        assert_eq!(stats.total_tracks, 2);
        assert_eq!(stats.lossless, 2);

        let tracks = db::load_tracks(&pool, None, None, None, lib.id).await.unwrap();
        let path = std::env::temp_dir().join(format!("preamble-test-{}.csv", std::process::id()));
        crate::export::export_csv(&tracks, &path).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("/x/A.flac") && body.contains("/x/B.flac"));
        std::fs::remove_file(&path).ok();
    }
}
