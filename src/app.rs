use crate::db;

use ratatui::widgets::ListState;
use sqlx::SqlitePool;
use crate::track::{TrackSummary};

pub struct App {
    pub pool: SqlitePool,
    pub tracks: Vec<TrackSummary>,
    // pub selected: usize,
    // pub pagination_offset: usize,
    pub list_state: ListState,
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
            tracks: db::load_tracks(&pool).await?,
            // selected: 0,
            // pagination_offset: 0,
            list_state: ListState::default(),
            current_tab: Tabs::Library,
            should_quit: false,
        })
    }
}