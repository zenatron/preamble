use crate::db;

use ratatui::widgets::{ListState, TableState};
use sqlx::SqlitePool;
use crate::track::{TrackSummary};

pub struct App {
    pub pool: SqlitePool,

    // vec of all tracks (summary)
    pub library_tracks: Vec<TrackSummary>,
    pub library_state: TableState,

    // vec of tracks with pending state
    pub pending_tracks: Vec<TrackSummary>,
    pub pending_state: TableState,

    // all duplicate tracks
    pub duplicate_state: TableState,
    
    pub current_tab: Tabs,
    pub should_quit: bool,
}

pub enum Tabs {
    Library,
    Enrichment,
    Duplicates,
}

impl App {
    pub async fn new(pool: SqlitePool) -> Result<Self, sqlx::Error> {
        // Should a new app always be initialized with existing paths?
        // The user can manually refetch the tracks on hand
        Ok(Self {
            pool: pool.clone(),

            library_tracks: db::load_tracks(&pool, None).await?,
            pending_tracks: db::load_tracks(&pool, Some("pending")).await?,
            
            library_state: TableState::default(),
            pending_state: TableState::default(),
            duplicate_state: TableState::default(),
            
            current_tab: Tabs::Library,
            should_quit: false,
        })
    }
}