use crate::db;

use crate::track::TrackSummary;
use ratatui::widgets::TableState;
use sqlx::SqlitePool;

pub struct App {
    pub pool: SqlitePool,

    pub pending_scan_path: Option<std::path::PathBuf>,

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

    pub current_screen: Screens,
    pub current_tab: Tabs,
    pub should_quit: bool,
}

impl App {
    pub async fn new(
        pool: SqlitePool,
        pending_scan_path: Option<std::path::PathBuf>,
    ) -> Result<Self, sqlx::Error> {
        // Should a new app always be initialized with existing paths?
        // The user can manually refetch the tracks on hand
        Ok(Self {
            pool: pool.clone(),

            pending_scan_path,

            library_stats: LibraryStats {
                total_tracks: db::count_tracks(&pool, None).await? as u32,
                total_pending: db::count_tracks(&pool, Some("pending")).await? as u32,
                total_duplicates: db::count_tracks(&pool, Some("duplicate")).await? as u32,
            },

            // this later will not happen on initial load and instead
            // will be populated ad hoc by the user's control
            // this will allow reloading these states while the app is running
            library_tracks: db::load_tracks(&pool, None).await?,
            pending_tracks: db::load_tracks(&pool, Some("pending")).await?,
            duplicate_tracks: db::load_tracks(&pool, Some("duplicate")).await?,

            library_state: TableState::default(),
            pending_state: TableState::default(),
            duplicate_state: TableState::default(),

            current_screen: Screens::Start, // init with starting screen
            current_tab: Tabs::Library,
            should_quit: false,
        })
    }
}

pub struct LibraryStats {
    // these values are pulled from DB on app startup
    pub total_tracks: u32,
    pub total_pending: u32,
    pub total_duplicates: u32,
}

pub enum Tabs {
    Library,
    Enrichment,
    Duplicates,
}

pub enum Screens {
    Start,
    Main,
    Scanning,
}
