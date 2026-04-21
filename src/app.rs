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

    // vec of all tracks (summary)
    pub library_tracks: Vec<TrackSummary>,
    pub library_state: TableState,

    // vec of tracks with pending state
    pub pending_tracks: Vec<TrackSummary>,
    pub pending_state: TableState,

    // all duplicate tracks
    pub duplicate_tracks: Vec<TrackSummary>,
    pub duplicate_state: TableState,

    // all orphaned tracks in th DB (missing on disk)
    pub missing_tracks: Vec<TrackSummary>,
    pub missing_state: TableState,

    // properties panel
    pub properties_panel_open: bool,
    pub properties_of_track: Option<TrackInfo>, // the whole track

    // search bar
    pub search_mode: bool,
    pub search_query: String,

    // screens and meta
    pub current_screen: Screens,
    pub current_tab: Tabs,
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

            // pull stats from DB on startup
            library_tracks: db::load_tracks(&pool, None, None, None).await?,
            pending_tracks: db::load_tracks(&pool, None, Some("pending"), None).await?,
            duplicate_tracks: db::load_tracks(&pool, None, Some("duplicate"), None).await?,
            missing_tracks: db::load_tracks(&pool, None, Some("missing"), None).await?,

            library_state: TableState::default(),
            pending_state: TableState::default(),
            duplicate_state: TableState::default(),
            missing_state: TableState::default(),

            // properties panel
            properties_panel_open: false,
            properties_of_track: None,

            search_mode: false,
            search_query: String::new(),

            // screens and meta
            current_screen: Screens::Start, // init with starting screen
            current_tab: Tabs::Library,
            should_quit: false,
        })
    }

    pub async fn reload(app: &mut App) -> Result<(), sqlx::Error> {
        app.library_stats.total_tracks = db::count_tracks(&app.pool, None).await? as u32;
        app.library_stats.total_pending =
            db::count_tracks(&app.pool, Some("pending")).await? as u32;
        app.library_stats.total_duplicates =
            db::count_tracks(&app.pool, Some("duplicate")).await? as u32;
        app.library_stats.total_missing =
            db::count_tracks(&app.pool, Some("missing")).await? as u32;

        app.library_tracks = db::load_tracks(&app.pool, None, None, None).await?;
        app.pending_tracks = db::load_tracks(&app.pool, None, Some("pending"), None).await?;
        app.duplicate_tracks = db::load_tracks(&app.pool, None, Some("duplicate"), None).await?;
        app.missing_tracks = db::load_tracks(&app.pool, None, Some("missing"), None).await?;

        app.properties_panel_open = false;
        app.properties_of_track = None;

        Ok(())
    }
}

pub struct LibraryStats {
    // these values are pulled from DB on app startup
    pub total_tracks: u32,
    pub total_pending: u32,
    pub total_duplicates: u32,
    pub total_missing: u32,
}

pub enum Tabs {
    Library,
    Enrichment,
    Duplicates,
    Missing,
}

impl std::fmt::Display for Tabs {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Tabs::Library => write!(f, "library"),
            Tabs::Enrichment => write!(f, "pending"),
            Tabs::Duplicates => write!(f, "duplicates"),
            Tabs::Missing => write!(f, "missing"),
        }
    }
}

pub enum Screens {
    Start,
    Main,
    Scanning,
}
