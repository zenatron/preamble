use crate::db;

use crate::reader::{ScanEvent, ValidateEvent};
use crate::track::{TrackInfo, TrackSummary};
use ratatui::widgets::TableState;
use sqlx::SqlitePool;

pub struct App {
    pub pool: SqlitePool,

    pub pending_scan_path: Option<std::path::PathBuf>,
    pub status_message: Option<String>,

    pub scan_progress: Option<(usize, usize)>, // (processed, total)
    pub scan_receiver: Option<tokio::sync::mpsc::Receiver<ScanEvent>>,

    pub is_validating: bool,
    pub spinner_tick: usize,
    pub validating_receiver: Option<tokio::sync::oneshot::Receiver<ValidateEvent>>,

    pub pending_delete: bool,

    pub library_stats: LibraryStats,

    pub tabs: Vec<TabData>,

    // properties panel
    pub properties_panel_open: bool,
    pub properties_of_track: Option<TrackInfo>, // the whole track

    // search bar
    pub search_mode: bool,
    pub search_query: String,

    // screens and meta
    pub current_screen: Screens,
    pub current_tab: usize,
    pub should_quit: bool,
}

impl App {
    pub async fn new(
        pool: SqlitePool,
        pending_scan_path: Option<std::path::PathBuf>,
    ) -> Result<Self, sqlx::Error> {
        // The user can manually refetch the tracks on hand
        Ok(Self {
            pool: pool.clone(),

            pending_scan_path,
            status_message: None,

            scan_progress: None,
            scan_receiver: None,

            is_validating: false,
            spinner_tick: 0,
            validating_receiver: None,

            pending_delete: false,

            // pull stats from DB on startup
            library_stats: LibraryStats {
                total_tracks: db::count_tracks(&pool, None).await? as u32,
                total_pending: db::count_tracks(&pool, Some("pending")).await? as u32,
                total_duplicates: db::count_tracks(&pool, Some("duplicate")).await? as u32,
                total_missing: db::count_tracks(&pool, Some("missing")).await? as u32,
            },

            tabs: vec![
                // Library
                TabData {
                    label: "Library",
                    status_filter: None,
                    tracks: db::load_tracks(&pool, None, None, None).await?,
                    state: TableState::default(),
                },
                // Enrichment
                TabData {
                    label: "Enrichment",
                    status_filter: Some("pending"),
                    tracks: db::load_tracks(&pool, None, Some("pending"), None).await?,
                    state: TableState::default(),
                },
                // Duplicate
                TabData {
                    label: "Duplicates",
                    status_filter: Some("duplicate"),
                    tracks: db::load_tracks(&pool, None, Some("duplicate"), None).await?,
                    state: TableState::default(),
                },
                // Missing
                TabData {
                    label: "Missing",
                    status_filter: Some("missing"),
                    tracks: db::load_tracks(&pool, None, Some("missing"), None).await?,
                    state: TableState::default(),
                },
            ],

            // properties panel
            properties_panel_open: false,
            properties_of_track: None,

            search_mode: false,
            search_query: String::new(),

            // screens and meta
            current_screen: Screens::Start, // init with starting screen
            current_tab: 0,
            should_quit: false,
        })
    }

    pub async fn reload(&mut self) -> Result<(), sqlx::Error> {
        self.library_stats.total_tracks = db::count_tracks(&self.pool, None).await? as u32;
        self.library_stats.total_pending =
            db::count_tracks(&self.pool, Some("pending")).await? as u32;
        self.library_stats.total_duplicates =
            db::count_tracks(&self.pool, Some("duplicate")).await? as u32;
        self.library_stats.total_missing =
            db::count_tracks(&self.pool, Some("missing")).await? as u32;

        for tab in &mut self.tabs {
            tab.tracks = db::load_tracks(&self.pool, None, tab.status_filter, None).await?;
        }

        self.properties_panel_open = false;
        self.properties_of_track = None;

        Ok(())
    }
}

pub struct TabData {
    pub label: &'static str,
    pub status_filter: Option<&'static str>,
    pub tracks: Vec<TrackSummary>,
    pub state: TableState,
}

pub struct LibraryStats {
    // these values are pulled from DB on app startup
    pub total_tracks: u32,
    pub total_pending: u32,
    pub total_duplicates: u32,
    pub total_missing: u32,
}

pub enum Screens {
    Start,
    Main,
    Scanning,
}
