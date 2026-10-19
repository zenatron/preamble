// src/screenshots.rs
//
// Renders the real TUI, headlessly, into SVG screenshots for the README and
// website. Each scene is drawn by the normal `draw` into a ratatui
// TestBackend over a demo library, and the cell buffer is written out as SVG:
// text as monospace runs, and block / box-drawing glyphs as exact shapes so
// borders and the logo stay seamless at any scale.
//
//   SHOT_DIR=/tmp/shots cargo test screenshots -- --ignored
//
// The library is made up: every artist, album and title below is invented.

use super::*;
use crate::config::Config;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use sqlx::sqlite::SqlitePoolOptions;
use std::fmt::Write as _;

const COLS: u16 = 132;
const ROWS: u16 = 38;

// Cell geometry for the SVG (Menlo's advance is 0.602em).
const FONT: f32 = 16.0;
const CW: f32 = FONT * 0.602;
const CH: f32 = 21.0;
const BAR: f32 = 34.0; // window title bar
const PAD: f32 = 14.0;

const BG: &str = "#141113";
const FG: &str = "#ddd6c9";

fn rgb(c: Color, fallback: &'static str) -> String {
    match c {
        Color::Reset => fallback.into(),
        Color::Black => "#1c1a17".into(),
        Color::Red => "#e8504f".into(),
        Color::Green => "#8cc47a".into(),
        Color::Yellow => "#f0b43c".into(),
        Color::Blue => "#6aa8e0".into(),
        Color::Magenta => "#c987e0".into(),
        Color::Cyan => "#3cc4d0".into(),
        Color::Gray => "#b9b2a6".into(),
        Color::DarkGray => "#6c665b".into(),
        Color::LightRed => "#ff6b6b".into(),
        Color::LightGreen => "#a8e08f".into(),
        Color::LightYellow => "#ffe27a".into(),
        Color::LightBlue => "#8ec5f0".into(),
        Color::LightMagenta => "#e0a3f0".into(),
        Color::LightCyan => "#8be8f0".into(),
        Color::White => "#f4efe6".into(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(_) => fallback.into(),
    }
}

/// Resolved colours for a cell, with REVERSED applied.
fn colours(cell: &ratatui::buffer::Cell) -> (String, String) {
    let mut fg = rgb(cell.fg, FG);
    let mut bg = rgb(cell.bg, BG);
    if cell.modifier.contains(Modifier::REVERSED) {
        std::mem::swap(&mut fg, &mut bg);
    }
    (fg, bg)
}

/// Shapes for glyphs a font would draw with gaps: (x0, y0, x1, y1) as
/// fractions of the cell, filled.
fn block_rects(ch: char) -> Option<Vec<(f32, f32, f32, f32)>> {
    let eighths = |n: f32| vec![(0.0, 1.0 - n / 8.0, 1.0, 1.0)];
    let left = |n: f32| vec![(0.0, 0.0, n / 8.0, 1.0)];
    Some(match ch {
        '█' => vec![(0.0, 0.0, 1.0, 1.0)],
        '▀' => vec![(0.0, 0.0, 1.0, 0.5)],
        '▄' => vec![(0.0, 0.5, 1.0, 1.0)],
        '▁' => eighths(1.0),
        '▂' => eighths(2.0),
        '▃' => eighths(3.0),
        '▅' => eighths(5.0),
        '▆' => eighths(6.0),
        '▇' => eighths(7.0),
        '▏' => left(1.0),
        '▎' => left(2.0),
        '▍' => left(3.0),
        '▌' => left(4.0),
        '▋' => left(5.0),
        '▊' => left(6.0),
        '▉' => left(7.0),
        _ => return None,
    })
}

/// Box-drawing glyphs as line segments between the cell centre and edges:
/// (up, down, left, right, double).
fn box_lines(ch: char) -> Option<(bool, bool, bool, bool, bool)> {
    Some(match ch {
        '─' => (false, false, true, true, false),
        '│' => (true, true, false, false, false),
        '┌' | '╭' => (false, true, false, true, false),
        '┐' | '╮' => (false, true, true, false, false),
        '└' | '╰' => (true, false, false, true, false),
        '┘' | '╯' => (true, false, true, false, false),
        '├' => (true, true, false, true, false),
        '┤' => (true, true, true, false, false),
        '┬' => (false, true, true, true, false),
        '┴' => (true, false, true, true, false),
        '┼' => (true, true, true, true, false),
        '║' => (true, true, false, false, true),
        '═' => (false, false, true, true, true),
        _ => return None,
    })
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Writes `buf` as an SVG inside a terminal-window frame titled `title`.
fn to_svg(buf: &Buffer, title: &str) -> String {
    let w = COLS as f32 * CW + 2.0 * PAD;
    let h = ROWS as f32 * CH + 2.0 * PAD + BAR;
    let ox = PAD;
    let oy = BAR + PAD;
    let mut out = String::new();
    let _ = write!(
        out,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.1} {h:.1}" width="{w:.0}" height="{h:.0}">
<defs><clipPath id="win"><rect width="{w:.1}" height="{h:.1}" rx="12"/></clipPath></defs>
<g clip-path="url(#win)">
<rect width="{w:.1}" height="{h:.1}" fill="{BG}"/>
<rect width="{w:.1}" height="{BAR}" fill="#1f1b1d"/>
<rect y="{BAR}" width="{w:.1}" height="1" fill="#2c2629"/>
<circle cx="20" cy="17" r="6" fill="#ff5f57"/><circle cx="40" cy="17" r="6" fill="#febc2e"/><circle cx="60" cy="17" r="6" fill="#28c840"/>
<text x="{cx:.1}" y="22" text-anchor="middle" font-family="Menlo, monospace" font-size="13" fill="#8f877a">{title}</text>
"##,
        cx = w / 2.0,
        title = escape(title),
    );

    // Backgrounds, merged into runs per row.
    for y in 0..ROWS {
        let mut x = 0;
        while x < COLS {
            let (_, bg) = colours(&buf[(x, y)]);
            let start = x;
            while x < COLS && colours(&buf[(x, y)]).1 == bg {
                x += 1;
            }
            if bg != BG {
                let _ = writeln!(
                    out,
                    r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{CH:.2}" fill="{bg}"/>"#,
                    ox + start as f32 * CW,
                    oy + y as f32 * CH,
                    (x - start) as f32 * CW + 0.4,
                );
            }
        }
    }

    // Glyphs: shapes for blocks / box drawing, text runs for the rest.
    for y in 0..ROWS {
        let mut x = 0;
        while x < COLS {
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            let ch = sym.chars().next().unwrap_or(' ');
            let (fg, _) = colours(cell);
            let px = ox + x as f32 * CW;
            let py = oy + y as f32 * CH;

            if let Some(rects) = block_rects(ch) {
                for (x0, y0, x1, y1) in rects {
                    let _ = writeln!(
                        out,
                        r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="{fg}"/>"#,
                        px + x0 * CW,
                        py + y0 * CH,
                        (x1 - x0) * CW + 0.4,
                        (y1 - y0) * CH + 0.4,
                    );
                }
                x += 1;
                continue;
            }
            if let Some((up, down, left, right, double)) = box_lines(ch) {
                let (mx, my) = (px + CW / 2.0, py + CH / 2.0);
                let offsets: &[f32] = if double { &[-1.6, 1.6] } else { &[0.0] };
                for &o in offsets {
                    let mut seg = |x1: f32, y1: f32, x2: f32, y2: f32| {
                        let _ = writeln!(
                            out,
                            r#"<line x1="{x1:.2}" y1="{y1:.2}" x2="{x2:.2}" y2="{y2:.2}" stroke="{fg}" stroke-width="1.3"/>"#
                        );
                    };
                    if up { seg(mx + o, py, mx + o, my + 0.65); }
                    if down { seg(mx + o, my - 0.65, mx + o, py + CH); }
                    if left { seg(px, my + o, mx + 0.65, my + o); }
                    if right { seg(mx - 0.65, my + o, px + CW, my + o); }
                }
                x += 1;
                continue;
            }

            // A run of ordinary text in one style.
            let start = x;
            let style_of = |c: &ratatui::buffer::Cell| (colours(c).0, c.modifier);
            let style = style_of(cell);
            let mut text = String::new();
            while x < COLS {
                let c = &buf[(x, y)];
                let first = c.symbol().chars().next().unwrap_or(' ');
                if block_rects(first).is_some() || box_lines(first).is_some() || style_of(c) != style {
                    break;
                }
                text.push_str(c.symbol());
                x += 1;
            }
            if text.trim().is_empty() {
                continue;
            }
            let weight = if style.1.contains(Modifier::BOLD) { "bold" } else { "normal" };
            let opacity = if style.1.contains(Modifier::DIM) { r#" opacity="0.6""# } else { "" };
            let _ = writeln!(
                out,
                r#"<text x="{:.2}" y="{:.2}" font-family="Menlo, monospace" font-size="{FONT}" font-weight="{weight}" fill="{}" xml:space="preserve"{opacity}>{}</text>"#,
                ox + start as f32 * CW,
                py + CH * 0.74,
                style.0,
                escape(&text),
            );
        }
    }
    out.push_str("</g>\n<rect x=\"0.5\" y=\"0.5\" width=\"");
    let _ = write!(out, "{:.1}\" height=\"{:.1}\" rx=\"12\" fill=\"none\" stroke=\"#3a3335\"/>\n</svg>\n", w - 1.0, h - 1.0);
    out
}

// The demo library

const ALBUMS: &[(&str, &str, u32, &[&str])] = &[
    ("Moth Engine", "Lantern Hours", 2019, &["Paper Satellites", "Slow Orbit", "Kerosene Light", "Low Tide Static", "The Long Way Home", "Afterglow Index"]),
    ("Saltmarsh", "Gulls Over Copper", 2016, &["Brine", "Copper Roofs", "Estuary", "Weathervane", "Harbour Lights"]),
    ("Kite Theory", "Updraft", 2022, &["Thermals", "Ripstop", "Line Tension", "Tailspin", "Updraft"]),
    ("Glasshouse Tapes", "Greenhouse Effects", 2008, &["Condensation", "Fern Room", "Humidity", "Potting Shed"]),
    ("Lumen Oak", "Understory", 2013, &["Canopy", "Mycelium", "Leaf Litter", "Clearing", "Rings"]),
    ("Dune Fever", "Mirage FM", 1998, &["Heat Shimmer", "Oasis Signal", "Sandglass", "Night Caravan"]),
    ("Ostinato Bloom", "Repeating Flowers", 2021, &["Petal Loop", "Second Bloom", "Perennial", "Pollen Count"]),
    ("Paper Lanterns Co.", "Festival Season", 1987, &["Opening Night", "Ferris Wheel", "Last Train Out"]),
];

async fn demo_app() -> App {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .min_connections(1)
        .idle_timeout(None)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let lib = db::create_library(&pool, "Vinyl Shelf", "/Users/you/Music").await.unwrap();

    let formats = [("FLAC", "flac", 1012), ("FLAC", "flac", 924), ("MP3", "mp3", 320), ("FLAC", "flac", 1101), ("OPUS", "opus", 160), ("MP3", "mp3", 256)];
    let statuses = ["enriched", "enriched", "enriched", "pending", "enriched", "manual_review", "pending", "not_found"];
    let mut n = 0usize;
    for (artist, album, year, titles) in ALBUMS {
        for (i, title) in titles.iter().enumerate() {
            let (fmt, ext, kbps) = formats[n % formats.len()];
            let path = format!("/Users/you/Music/{artist}/{album}/{:02} {title}.{ext}", i + 1);
            let secs = 150 + (n * 37 % 190) as i64;
            let size = secs * kbps as i64 * 125;
            let health = match n {
                9 => Some("low_bitrate"),
                22 => Some("decode_error"),
                _ => None,
            };
            sqlx::query(
                "INSERT INTO tracks (file_path, title, artist, album, album_artist, genre, track, track_total,
                 release_year, file_format, file_size, duration, bitrate, sample_rate, bit_depth, channels,
                 status, file_hash, isrc, health_issue, library_id, label, composer)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 2, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&path)
            .bind(title)
            .bind(artist)
            .bind(album)
            .bind(artist)
            .bind(["Dream Pop", "Folk", "Indie", "Ambient"][n % 4])
            .bind(i as i64 + 1)
            .bind(titles.len() as i64)
            .bind(*year as i64)
            .bind(fmt)
            .bind(size)
            .bind(secs * 1000)
            .bind(kbps)
            .bind(if fmt == "FLAC" { 44100 } else { 48000 })
            .bind(if fmt == "FLAC" { Some(24) } else { None })
            .bind(statuses[n % statuses.len()])
            .bind(format!("{:064x}", n * 7919 + 1))
            .bind(format!("QZ4JK{:07}", n))
            .bind(health)
            .bind(lib.id)
            .bind("Small Hours Records")
            .bind(*artist)
            .execute(&pool)
            .await
            .unwrap();
            n += 1;
        }
    }

    // Duplicates: two byte-identical copies, and one recording as FLAC + MP3.
    let dupes: &[(&str, &str, &str, &str, i64, i64, &str)] = &[
        ("/Users/you/Music/_Downloads/Slow Orbit.flac", "Slow Orbit", "FLAC", "hash", 1012, 0, "Moth Engine"),
        ("/Users/you/Music/_Backup/Lantern Hours/02 Slow Orbit.flac", "Slow Orbit", "FLAC", "hash", 1012, 0, "Moth Engine"),
        ("/Users/you/Music/_Old iPod/Thermals.mp3", "Thermals", "MP3", "isrc", 192, 1, "Kite Theory"),
        ("/Users/you/Music/_Old iPod/Brine.mp3", "Brine", "MP3", "isrc", 128, 2, "Saltmarsh"),
    ];
    for (path, title, fmt, kind, kbps, which, artist) in dupes {
        let (hash, isrc) = match (*kind, *which) {
            ("hash", _) => (format!("{:064x}", 7919 + 1), format!("QZ4JK{:07}", 1)),
            (_, 1) => ("f".repeat(64).to_string(), format!("QZ4JK{:07}", 25)),
            _ => ("e".repeat(64).to_string(), format!("QZ4JK{:07}", 6)),
        };
        sqlx::query(
            "INSERT INTO tracks (file_path, title, artist, album, file_format, file_size, duration, bitrate,
             sample_rate, channels, status, file_hash, isrc, library_id, release_year)
             VALUES (?, ?, ?, ?, ?, ?, 214000, ?, 44100, 2, 'duplicate', ?, ?, ?, 2019)",
        )
        .bind(path)
        .bind(title)
        .bind(artist)
        .bind(if *which == 1 { "Updraft" } else if *which == 2 { "Gulls Over Copper" } else { "Lantern Hours" })
        .bind(fmt)
        .bind(214 * kbps * 125)
        .bind(kbps)
        .bind(hash)
        .bind(isrc)
        .bind(lib.id)
        .execute(&pool)
        .await
        .unwrap();
    }
    // Two tracks already waiting in Trash.
    sqlx::query("UPDATE tracks SET marked_for_deletion = 1 WHERE id IN (21, 33)")
        .execute(&pool)
        .await
        .unwrap();

    let mut app = App::new(pool, Config::default()).await.unwrap();
    app.open_library(lib).await.unwrap();
    app
}

fn render(app: &mut App) -> Buffer {
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    term.backend().buffer().clone()
}

#[tokio::test]
#[ignore = "writes screenshots; run explicitly"]
async fn screenshots() {
    let dir = std::path::PathBuf::from(
        std::env::var("SHOT_DIR").unwrap_or_else(|_| "target/screenshots".into()),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let title = "preamble — ~/Music";
    let save = |name: &str, buf: &Buffer| {
        std::fs::write(dir.join(format!("{name}.svg")), to_svg(buf, title)).unwrap();
    };

    let mut app = demo_app().await;

    app.current_screen = app::Screens::Start;
    save("start", &render(&mut app));

    app.current_screen = app::Screens::Main;
    let lib = tab_of(&app, "Library");
    app.current_tab = lib;
    app.tabs[lib].sort = app::SortKey::Artist;
    app.tabs[lib].apply_sort();
    for i in [3, 4, 5] {
        app.tabs[lib].tracks[i].is_selected = true;
    }
    app.tabs[lib].state.select(Some(5));
    save("library", &render(&mut app));

    app.properties_panel_open = true;
    load_selected_track(&mut app).await;
    save("properties", &render(&mut app));
    app.properties_panel_open = false;
    for t in &mut app.tabs[lib].tracks {
        t.is_selected = false;
    }

    app.current_tab = tab_of(&app, "Duplicates");
    app.ensure_fresh().await.unwrap();
    app.duplicates.focus = app::DuplicatePane::Members;
    save("duplicates", &render(&mut app));

    app.current_tab = lib;
    app.group_mode = app::GroupMode::Artist;
    app.refresh_groups().await.unwrap();
    save("grouped", &render(&mut app));
    app.group_mode = app::GroupMode::Off;

    app.stats = Some(db::compute_stats(&app.pool, app.active_library_id()).await.unwrap());
    app.current_screen = app::Screens::Stats;
    save("stats", &render(&mut app));
}

fn tab_of(app: &App, label: &str) -> usize {
    app.tabs.iter().position(|t| t.label == label).unwrap()
}
