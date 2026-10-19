// src/stress_tests.rs
//
// Robustness and load tests over a synthetic on-disk library built from the
// tiny MP3/FLAC fixtures in `testdata/`. The hostile-tree / concurrency tests
// run with the normal suite; the large load test is `#[ignore]`d:
//
//   STRESS_N=20000 cargo test --release stress_ -- --ignored --nocapture

use super::*;
use crate::config::Config;
use crate::reader::{self, ScanEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

const MP3: &[u8] = include_bytes!("../testdata/tpl.mp3");
const FLAC: &[u8] = include_bytes!("../testdata/tpl.flac");

fn press(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

/// A fresh scratch directory unique to this test + process.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("preamble-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A file-backed WAL database, like the real app uses (an in-memory pool
/// can't exercise concurrent writers).
async fn file_pool(dir: &Path) -> sqlx::SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(dir.join("library.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new().connect_with(options).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

/// Writes a tagged copy of a fixture. Distinct titles give distinct bytes, so
/// every call produces a file with a unique hash.
fn write_track(path: &Path, title: &str, artist: &str, isrc: Option<&str>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let flac = path.extension().is_some_and(|e| e == "flac");
    std::fs::write(path, if flac { FLAC } else { MP3 }).unwrap();

    let mut file = lofty::read_from_path(path).unwrap();
    if file.primary_tag().is_none() {
        let tag_type = file.primary_tag_type();
        file.insert_tag(Tag::new(tag_type));
    }
    let tag = file.primary_tag_mut().unwrap();
    tag.insert_text(ItemKey::TrackTitle, title.to_string());
    tag.insert_text(ItemKey::TrackArtist, artist.to_string());
    tag.insert_text(ItemKey::AlbumTitle, format!("{artist} LP"));
    if let Some(isrc) = isrc {
        tag.insert_text(ItemKey::Isrc, isrc.to_string());
    }
    file.save_to_path(path, WriteOptions::default()).unwrap();
}

/// What a synthetic library should scan to.
struct Expected {
    tracks: usize,
    hash_dupes: usize,
    isrc_dupes: usize,
}

/// Builds `n` unique tracks plus byte-identical copies (every 20th) and
/// same-ISRC re-encodes (every 50th) spread over a nested artist/album tree.
fn build_library(root: &Path, n: usize) -> Expected {
    let mut hash_dupes = 0;
    let mut isrc_dupes = 0;
    for i in 0..n {
        let ext = if i % 3 == 0 { "flac" } else { "mp3" };
        let artist = format!("Artist {:03}", i % 97);
        let path = root.join(format!("{artist}/Album {:02}/{i:06}.{ext}", i % 7));
        let isrc = (i % 50 == 0).then(|| format!("USPR{i:08}"));
        write_track(&path, &format!("Track {i}"), &artist, isrc.as_deref());

        if i % 20 == 0 {
            let copy = root.join(format!("copies/{i:06}.{ext}"));
            std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
            std::fs::copy(&path, &copy).unwrap();
            hash_dupes += 1;
        }
        if let Some(isrc) = isrc {
            let other = root.join(format!("reencodes/{i:06}.mp3"));
            write_track(&other, &format!("Track {i} (remaster)"), &artist, Some(&isrc));
            isrc_dupes += 1;
        }
    }
    Expected {
        tracks: n + hash_dupes + isrc_dupes,
        hash_dupes,
        isrc_dupes,
    }
}

/// Adds the things real music folders contain that aren't clean audio files.
fn add_hostile_entries(root: &Path) {
    std::fs::write(root.join("empty.mp3"), b"").unwrap();
    std::fs::write(root.join("corrupt.flac"), b"fLaC this is not really flac").unwrap();
    std::fs::write(root.join("notes.txt"), b"liner notes").unwrap();
    std::fs::write(root.join("UPPER.MP3"), MP3).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A symlink back to the root: naive recursion never terminates.
        std::os::unix::fs::symlink(root, root.join("loop")).unwrap();
        // A directory we can't list.
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("hidden.mp3"), MP3).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
}

fn unlock(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = root.join("locked");
        if locked.exists() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).ok();
        }
    }
}

/// Runs a scan to completion. Returns whether the worker reported `Done`
/// (rather than dying silently, which would strand the UI on the scan screen).
async fn scan(pool: &sqlx::SqlitePool, library_id: i64, root: &Path) -> bool {
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let handle = tokio::spawn(reader::scan_library(
        pool.clone(),
        library_id,
        root.to_path_buf(),
        Config::default().formats,
        8,
        Arc::new(AtomicBool::new(false)),
        tx,
    ));
    let mut done = false;
    let wait = tokio::time::timeout(Duration::from_secs(600), async {
        while let Some(ev) = rx.recv().await {
            if matches!(ev, ScanEvent::Done) {
                done = true;
            }
        }
    });
    wait.await.expect("scan hung");
    let _ = handle.await;
    done
}

async fn library_at(pool: &sqlx::SqlitePool, name: &str, root: &Path) -> db::Library {
    db::create_library(pool, name, &root.to_string_lossy())
        .await
        .unwrap()
}

fn tab_index(app: &App, label: &str) -> usize {
    app.tabs.iter().position(|t| t.label == label).unwrap()
}

#[tokio::test]
async fn scan_survives_hostile_tree() {
    let dir = scratch("hostile");
    let root = dir.join("music");
    let expected = build_library(&root, 30);
    add_hostile_entries(&root);
    let pool = file_pool(&dir).await;
    let lib = library_at(&pool, "hostile", &root).await;

    let done = tokio::time::timeout(Duration::from_secs(60), scan(&pool, lib.id, &root))
        .await
        .expect("scan never finished (symlink loop?)");
    unlock(&root);

    assert!(done, "scan worker died without reporting Done");
    let n = db::count_tracks(&pool, None, lib.id).await.unwrap() as usize;
    // +1 for UPPER.MP3: extensions match case-insensitively.
    assert_eq!(n, expected.tracks + 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn concurrent_scans_of_one_library_both_finish() {
    // A watch-mode import and a manual scan can overlap.
    let dir = scratch("concurrent");
    let root = dir.join("music");
    let expected = build_library(&root, 200);
    let pool = file_pool(&dir).await;
    let lib = library_at(&pool, "concurrent", &root).await;

    let (a, b) = tokio::join!(scan(&pool, lib.id, &root), scan(&pool, lib.id, &root));
    assert!(a && b, "an overlapping scan aborted");
    let n = db::count_tracks(&pool, None, lib.id).await.unwrap() as usize;
    assert_eq!(n, expected.tracks);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn nested_libraries_each_see_their_files() {
    let dir = scratch("nested");
    let root = dir.join("music");
    build_library(&root, 40);
    let pool = file_pool(&dir).await;
    let inner_root = root.join("Artist 001");
    let inner = library_at(&pool, "inner", &inner_root).await;
    let outer = library_at(&pool, "outer", &root).await;

    assert!(scan(&pool, inner.id, &inner_root).await);
    assert!(scan(&pool, outer.id, &root).await, "outer scan aborted on overlap");
    let inner_n = db::count_tracks(&pool, None, inner.id).await.unwrap();
    let outer_n = db::count_tracks(&pool, None, outer.id).await.unwrap();
    assert!(inner_n > 0);
    // The outer library includes everything under it, inner files too.
    let on_disk = db::load_tracks(&pool, None, None, None, outer.id).await.unwrap();
    assert_eq!(outer_n as usize, on_disk.len());
    assert!(on_disk.iter().any(|t| t.file_path.starts_with(&inner_root)));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn undo_stays_in_its_library() {
    let dir = scratch("undo-scope");
    let pool = file_pool(&dir).await;
    let a = library_at(&pool, "A", &dir.join("a")).await;
    let b = library_at(&pool, "B", &dir.join("b")).await;
    for (lib, path) in [(&a, "/a/1.mp3"), (&b, "/b/1.mp3")] {
        sqlx::query("INSERT INTO tracks (file_path, title, status, library_id) VALUES (?, 'x', 'pending', ?)")
            .bind(path)
            .bind(lib.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    // Flag a track in A, then switch to B and press undo.
    let mut app = App::new(pool.clone(), Config::default()).await.unwrap();
    app.open_library(a.clone()).await.unwrap();
    app.current_screen = app::Screens::Main;
    app.tabs[0].state.select(Some(0));
    handle_main_navigation(&mut app, press('d')).await;
    assert_eq!(db::count_marked(&pool, a.id).await.unwrap(), 1);

    app.open_library(b.clone()).await.unwrap();
    app.current_screen = app::Screens::Main;
    handle_main_navigation(&mut app, press('z')).await;
    assert_eq!(
        db::count_marked(&pool, a.id).await.unwrap(),
        1,
        "undo in library B reverted an action from library A"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn purge_undo_round_trip_restores_trash() {
    let dir = scratch("purge");
    let root = dir.join("music");
    let expected = build_library(&root, 25);
    let pool = file_pool(&dir).await;
    let lib = library_at(&pool, "purge", &root).await;
    assert!(scan(&pool, lib.id, &root).await);

    let config = Config {
        quarantine_dir: dir.join("quarantine"),
        ..Config::default()
    };
    let mut app = App::new(pool.clone(), config).await.unwrap();
    app.open_library(lib.clone()).await.unwrap();
    app.current_screen = app::Screens::Main;

    // Flag everything, purge it, then undo the purge.
    handle_main_navigation(&mut app, press('i')).await;
    handle_main_navigation(&mut app, press('d')).await;
    app.current_tab = tab_index(&app, "Trash");
    app.ensure_fresh().await.unwrap();
    handle_main_navigation(&mut app, press('d')).await;
    handle_main_navigation(&mut app, press('d')).await;
    assert_eq!(db::count_tracks(&pool, None, lib.id).await.unwrap(), 0);
    let quarantined = std::fs::read_dir(dir.join("quarantine")).unwrap().count();
    assert_eq!(quarantined, expected.tracks);

    handle_main_navigation(&mut app, press('z')).await;
    let n = db::count_tracks(&pool, None, lib.id).await.unwrap() as usize;
    assert_eq!(n, expected.tracks);
    // Undoing a purge returns tracks to where they were: flagged, in Trash.
    assert_eq!(db::count_marked(&pool, lib.id).await.unwrap() as usize, n);
    let tracks = db::load_tracks(&pool, None, None, None, lib.id).await.unwrap();
    assert!(tracks.iter().all(|t| t.file_path.exists()));

    // And the flag before the purge is still undoable afterwards.
    handle_main_navigation(&mut app, press('z')).await;
    assert_eq!(db::count_marked(&pool, lib.id).await.unwrap(), 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn failed_worker_does_not_strand_the_ui() {
    // A library whose path doesn't exist: the scan finds nothing, but must
    // still release the UI back to the start screen.
    let dir = scratch("stranded");
    let pool = file_pool(&dir).await;
    let lib = library_at(&pool, "gone", &dir.join("does-not-exist")).await;
    let mut app = App::new(pool.clone(), Config::default()).await.unwrap();
    app.open_library(lib).await.unwrap();
    // Simulate a worker that dies before sending Done.
    let (tx, rx) = tokio::sync::mpsc::channel::<ScanEvent>(1);
    app.scan_receiver = Some(rx);
    app.current_screen = app::Screens::Scanning;
    drop(tx);
    app.pump_workers().await.unwrap();
    assert!(app.scan_receiver.is_none());
    assert!(matches!(app.current_screen, app::Screens::Start));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
#[ignore = "load test; run explicitly"]
async fn stress_large_library() {
    let n: usize = std::env::var("STRESS_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000);
    let dir = scratch("stress");
    let root = dir.join("music");

    let t = Instant::now();
    let expected = build_library(&root, n);
    println!("built {} files in {:?}", expected.tracks, t.elapsed());

    let pool = file_pool(&dir).await;
    let lib = library_at(&pool, "stress", &root).await;

    let t = Instant::now();
    assert!(scan(&pool, lib.id, &root).await);
    let scan_time = t.elapsed();
    let total = db::count_tracks(&pool, None, lib.id).await.unwrap() as usize;
    println!("scan: {total} tracks in {scan_time:?}");
    assert_eq!(total, expected.tracks);
    let dupes = db::count_tracks(&pool, Some("duplicate"), lib.id).await.unwrap() as usize;
    assert_eq!(dupes, expected.hash_dupes + expected.isrc_dupes);
    let groups = db::count_duplicate_groups(&pool, lib.id).await.unwrap() as usize;
    println!("duplicates: {dupes} rows in {groups} groups");

    let t = Instant::now();
    assert!(scan(&pool, lib.id, &root).await);
    println!("no-op rescan: {:?}", t.elapsed());
    assert_eq!(db::count_tracks(&pool, None, lib.id).await.unwrap() as usize, total);

    let config = Config {
        quarantine_dir: dir.join("quarantine"),
        ..Config::default()
    };
    let t = Instant::now();
    let mut app = App::new(pool.clone(), config).await.unwrap();
    app.open_library(lib.clone()).await.unwrap();
    app.current_screen = app::Screens::Main;
    println!("open library: {:?}", t.elapsed());

    let t = Instant::now();
    app.tabs[0].search_query = "track 1".into();
    app.reload_current_tab().await.unwrap();
    println!("search ({} hits): {:?}", app.tabs[0].tracks.len(), t.elapsed());
    app.tabs[0].search_query.clear();
    app.reload_current_tab().await.unwrap();

    let t = Instant::now();
    for _ in 0..8 {
        handle_main_navigation(&mut app, press('s')).await;
    }
    println!("8 sort cycles: {:?}", t.elapsed());

    let t = Instant::now();
    handle_main_navigation(&mut app, press('i')).await;
    handle_main_navigation(&mut app, press('d')).await;
    println!("flag all: {:?}", t.elapsed());
    assert_eq!(db::count_marked(&pool, lib.id).await.unwrap() as usize, total);

    let t = Instant::now();
    handle_main_navigation(&mut app, press('z')).await;
    println!("undo flag-all: {:?}", t.elapsed());
    assert_eq!(db::count_marked(&pool, lib.id).await.unwrap(), 0);

    // Render every tab at a realistic size.
    let t = Instant::now();
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 60)).unwrap();
    for i in 0..app.tabs.len() {
        app.current_tab = i;
        app.ensure_fresh().await.unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
    }
    println!("load + render all tabs: {:?}", t.elapsed());

    app.current_tab = 0;
    app.ensure_fresh().await.unwrap();
    let t = Instant::now();
    for _ in 0..60 {
        handle_main_navigation(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)).await;
        term.draw(|f| draw(f, &mut app)).unwrap();
    }
    println!("60 frames scrolling the Library: {:?}", t.elapsed());

    app.current_tab = 0;
    app.ensure_fresh().await.unwrap();
    handle_main_navigation(&mut app, press('i')).await;
    handle_main_navigation(&mut app, press('d')).await;
    app.current_tab = tab_index(&app, "Trash");
    app.ensure_fresh().await.unwrap();
    let t = Instant::now();
    handle_main_navigation(&mut app, press('d')).await;
    handle_main_navigation(&mut app, press('d')).await;
    println!("purge all: {:?}", t.elapsed());
    assert_eq!(db::count_tracks(&pool, None, lib.id).await.unwrap(), 0);

    let t = Instant::now();
    handle_main_navigation(&mut app, press('z')).await;
    println!("undo purge: {:?}", t.elapsed());
    assert_eq!(db::count_tracks(&pool, None, lib.id).await.unwrap() as usize, total);

    let t = Instant::now();
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    tokio::spawn(reader::health_check(
        pool.clone(),
        lib.id,
        128,
        8,
        Arc::new(AtomicBool::new(false)),
        tx,
    ));
    while let Some(ev) = rx.recv().await {
        if matches!(ev, ScanEvent::Done) {
            break;
        }
    }
    println!("health check: {:?}", t.elapsed());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn upgrade_keeps_existing_data() {
    // Build a database on the pre-upgrade schema (every migration but the
    // newest), fill it, then run the full set as a real upgrade would.
    let dir = scratch("upgrade");
    let old = dir.join("old-migrations");
    std::fs::create_dir_all(&old).unwrap();
    let mut files: Vec<_> = std::fs::read_dir("migrations")
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    let newest = files.pop().unwrap();
    assert!(newest.to_string_lossy().contains("per_library_paths"));
    for f in &files {
        std::fs::copy(f, old.join(f.file_name().unwrap())).unwrap();
    }

    let options = SqliteConnectOptions::new()
        .filename(dir.join("library.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new().connect_with(options).await.unwrap();
    sqlx::migrate::Migrator::new(old.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO libraries (id, name, path) VALUES (1, 'Main', '/m')")
        .execute(&pool)
        .await
        .unwrap();
    for i in 1..=10 {
        sqlx::query(
            "INSERT INTO tracks (file_path, title, artist, status, library_id, marked_for_deletion, health_issue)
             VALUES (?, ?, 'Old Artist', 'enriched', 1, ?, ?)",
        )
        .bind(format!("/m/{i}.flac"))
        .bind(format!("Song {i}"))
        .bind(i % 2)
        .bind((i == 3).then_some("low_bitrate"))
        .execute(&pool)
        .await
        .unwrap();
    }
    // Purged rows leave a gap at the top of the id range.
    sqlx::query("DELETE FROM tracks WHERE id > 8").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO action_log (kind, summary, payload) VALUES ('flag', 'old', '{}')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let rows: Vec<(i64, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, file_path, title, marked_for_deletion, health_issue FROM tracks ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 8);
    assert_eq!(rows[2], (3, "/m/3.flac".into(), "Song 3".into(), 1, Some("low_bitrate".into())));

    // Search still works on migrated rows (the FTS index survived the rebuild)
    // and follows edits through the recreated triggers.
    let hits = db::load_tracks(&pool, None, None, Some("song"), 1).await.unwrap();
    assert_eq!(hits.len(), 8);
    sqlx::query("UPDATE tracks SET title = 'Renamed' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(db::load_tracks(&pool, None, None, Some("renamed"), 1).await.unwrap().len(), 1);

    // New ids continue above the purged ones rather than reusing them.
    sqlx::query("INSERT INTO tracks (file_path, status, library_id) VALUES ('/m/new.flac', 'pending', 1)")
        .execute(&pool)
        .await
        .unwrap();
    let max: i64 = sqlx::query_scalar("SELECT MAX(id) FROM tracks").fetch_one(&pool).await.unwrap();
    assert_eq!(max, 11);

    // Same path, other library: now allowed. Same path, same library: not.
    sqlx::query("INSERT INTO libraries (id, name, path) VALUES (2, 'Other', '/')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO tracks (file_path, status, library_id) VALUES ('/m/1.flac', 'pending', 2)")
        .execute(&pool)
        .await
        .unwrap();
    let dup = sqlx::query("INSERT INTO tracks (file_path, status, library_id) VALUES ('/m/1.flac', 'pending', 2)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(db::is_unique_violation(&dup));

    // Pre-upgrade undo entries have no library and are not offered anywhere.
    assert!(db::take_last_undo(&pool, 1).await.unwrap().is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn logo_fits_every_width() {
    for width in [20u16, 50, 58, 80, 200] {
        let backend = ratatui::backend::TestBackend::new(width, LOGO_HEIGHT);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| {
            let area = f.area();
            f.render_widget(
                preamble_logo(area.width).alignment(ratatui::layout::Alignment::Center),
                area,
            )
        })
        .unwrap();
        if std::env::var_os("SHOW_LOGO").is_some() {
            let buf = term.backend().buffer();
            for y in 0..LOGO_HEIGHT {
                let row: String = (0..width).map(|x| buf[(x, y)].symbol()).collect();
                println!("{row}|");
            }
        }
        for line in preamble_logo(width).lines {
            assert!(line.width() <= width as usize, "logo overflows {width} cols");
        }
    }
}
