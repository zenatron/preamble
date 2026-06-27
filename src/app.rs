use crate::config::Config;
use crate::db;

use crate::enrich::EnrichEvent;
use crate::reader::{ScanEvent, ValidateEvent, scan_library, watch_dir};
use crate::track::{DuplicateGroupSummary, TrackInfo, TrackSummary};
use ratatui::widgets::TableState;
use sqlx::SqlitePool;

pub struct App {
    pub pool: SqlitePool,
    pub config: Config,

    /// The currently open library. `None` only before one is opened (picker /
    /// create screens). Every per-track query scopes to its id.
    pub active_library: Option<db::Library>,
    /// Libraries listed in the picker (most-recently-opened first).
    pub libraries: Vec<db::Library>,
    /// Highlighted picker row; index `libraries.len()` is the "Create new…" row.
    pub picker_index: usize,
    /// In-progress "create library" form (path + name + focused field).
    pub new_lib_path: String,
    pub new_lib_name: String,
    pub new_lib_focus: NewLibField,

    pub pending_scan_path: Option<std::path::PathBuf>,
    pub status_message: Option<StatusMessage>,

    pub scan_progress: Option<(usize, usize)>, // (processed, total)
    pub scan_receiver: Option<tokio::sync::mpsc::Receiver<ScanEvent>>,
    /// Label shown on the progress screen (Scanning / Health check / Rescanning).
    pub scan_label: &'static str,

    pub is_validating: bool,
    pub spinner_tick: usize,
    pub validating_receiver: Option<tokio::sync::oneshot::Receiver<ValidateEvent>>,

    // enrichment pipeline
    pub is_enriching: bool,
    pub enrich_progress: Option<(usize, usize)>,
    pub enrich_receiver: Option<tokio::sync::mpsc::Receiver<EnrichEvent>>,

    // watch mode (background auto-scan)
    pub watch_enabled: bool,
    pub watch_rx: Option<tokio::sync::mpsc::Receiver<()>>,
    pub watch_scan_rx: Option<tokio::sync::mpsc::Receiver<ScanEvent>>,
    pub watch_scan_active: bool,
    pub watch_pending: bool,
    pub watch_last_event: std::time::Instant,
    watcher: Option<notify::RecommendedWatcher>,

    pub pending_delete: bool,
    pub pending_purge: bool,

    /// Cooperative cancel flag for long-running scans / enrichment.
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,

    pub library_stats: LibraryStats,

    pub tabs: Vec<TabData>,

    pub duplicates: DuplicatesView,
    /// Set when a mutation may have changed the duplicate groups; the view is
    /// reloaded lazily the next time the Duplicates tab is shown.
    pub duplicates_dirty: bool,

    // properties panel
    pub properties_panel_open: bool,
    pub properties_of_track: Option<TrackInfo>, // the whole track
    pub properties_scroll: u16,

    // search bar (query is stored per-tab; these track the editing session)
    pub search_mode: bool,
    pub search_dirty: bool,
    pub search_last_edit: std::time::Instant,

    // export format picker popup
    pub export_mode: bool,

    // filter popup
    pub filter_mode: bool,
    /// Distinct file formats present in the library, for cycling the format facet.
    pub formats_in_library: Vec<String>,

    // help overlay
    pub help_open: bool,

    // manual tag editor (Some while editing a track's tags)
    pub edit: Option<EditState>,

    // statistics screen snapshot
    pub stats: Option<crate::db::Stats>,

    // album/artist grouped browse (Library tab)
    pub group_mode: GroupMode,
    pub groups: Vec<crate::db::GroupRow>,
    pub groups_state: TableState,

    // last-rendered areas, for mapping mouse clicks to rows/tabs
    pub table_area: ratatui::layout::Rect,
    pub tab_bar_area: ratatui::layout::Rect,

    // screens and meta
    pub current_screen: Screens,
    pub current_tab: usize,
    pub should_quit: bool,
    pub quit_confirmed: bool,

    /// Set whenever something on screen may have changed. The event loop only
    /// repaints when this is set (or an operation is animating)
    pub needs_redraw: bool,
}

impl App {
    /// Builds the app shell without an open library. Tabs start empty; call
    /// [`App::open_library`] to hydrate them for a specific library. Keeping
    /// construction library-agnostic lets the picker / create flows and the
    /// in-session `L` switch all funnel through one re-hydration path.
    pub async fn new(pool: SqlitePool, config: Config) -> Result<Self, sqlx::Error> {
        Ok(Self {
            pool: pool.clone(),
            config,

            active_library: None,
            libraries: Vec::new(),
            picker_index: 0,
            new_lib_path: String::new(),
            new_lib_name: String::new(),
            new_lib_focus: NewLibField::Path,

            pending_scan_path: None,
            status_message: None,

            scan_progress: None,
            scan_receiver: None,
            scan_label: "Scanning",

            is_validating: false,
            spinner_tick: 0,
            validating_receiver: None,

            is_enriching: false,
            enrich_progress: None,
            enrich_receiver: None,

            watch_enabled: false,
            watch_rx: None,
            watch_scan_rx: None,
            watch_scan_active: false,
            watch_pending: false,
            watch_last_event: std::time::Instant::now(),
            watcher: None,

            pending_delete: false,
            pending_purge: false,

            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),

            // Zeroed until a library is opened (see `open_library`).
            library_stats: LibraryStats {
                total_tracks: 0,
                total_pending: 0,
                total_duplicates: 0,
                total_missing: 0,
                total_marked: 0,
            },

            // Tab scaffolding; rows are loaded by `open_library`.
            tabs: vec![
                TabData::new("Library", TabSource::Status(None), Vec::new()),
                TabData::new("Enrichment", TabSource::Status(Some("pending")), Vec::new()),
                // Duplicates is rendered specially, not from `tracks`.
                TabData::new("Duplicates", TabSource::Status(None), Vec::new()),
                // Failed - enrichment dead letters, with a retry action.
                TabData::new("Failed", TabSource::DeadLetter, Vec::new()),
                TabData::new("Missing", TabSource::Status(Some("missing")), Vec::new()),
                // Trash - everything flagged for deletion, awaiting purge.
                TabData::new("Trash", TabSource::Marked, Vec::new()),
            ],

            duplicates: DuplicatesView::empty(),
            duplicates_dirty: false,

            // properties panel
            properties_panel_open: false,
            properties_of_track: None,
            properties_scroll: 0,

            search_mode: false,
            search_dirty: false,
            search_last_edit: std::time::Instant::now(),

            export_mode: false,

            filter_mode: false,
            formats_in_library: Vec::new(),

            help_open: false,

            edit: None,
            stats: None,

            group_mode: GroupMode::Off,
            groups: Vec::new(),
            groups_state: TableState::default(),

            table_area: ratatui::layout::Rect::default(),
            tab_bar_area: ratatui::layout::Rect::default(),

            // screens and meta
            current_screen: Screens::Start, // init with starting screen
            current_tab: 0,
            should_quit: false,
            quit_confirmed: false,

            needs_redraw: true,
        })
    }

    /// Id of the active library, or 0 when none is open. Scoped queries with id
    /// 0 return nothing, so callers stay safe before a library is opened.
    pub fn active_library_id(&self) -> i64 {
        self.active_library.as_ref().map(|l| l.id).unwrap_or(0)
    }

    /// Opens `lib`: records it active, hydrates every tab + counts + duplicates +
    /// formats for it, resets transient view state, and lands on the Start
    /// screen. The single re-hydration entry point shared by the picker, the
    /// CLI-path open, and the in-session `L` switch.
    pub async fn open_library(&mut self, lib: db::Library) -> Result<(), sqlx::Error> {
        db::touch_library(&self.pool, lib.id).await.ok();
        self.pending_scan_path = Some(std::path::PathBuf::from(&lib.path));
        self.active_library = Some(lib);
        let library_id = self.active_library_id();

        // Reset per-tab view state so a switch doesn't carry over search/filters.
        self.current_tab = 0;
        for tab in &mut self.tabs {
            tab.search_query.clear();
            tab.filter.clear();
            tab.dirty = false;
        }
        self.search_mode = false;
        self.search_dirty = false;
        self.group_mode = GroupMode::Off;
        self.groups.clear();

        let pool = self.pool.clone();
        for tab in &mut self.tabs {
            // Duplicates is rendered from `self.duplicates`, not its tab rows.
            if tab.label != "Duplicates" {
                reload_tab(&pool, tab, library_id).await?;
            }
        }
        self.duplicates = DuplicatesView::new(&self.pool, library_id).await?;
        self.duplicates_dirty = false;
        self.formats_in_library = db::distinct_formats(&self.pool, library_id).await?;
        self.refresh_counts().await?;

        self.properties_panel_open = false;
        self.properties_of_track = None;
        self.properties_scroll = 0;
        self.current_screen = Screens::Start;
        self.needs_redraw = true;
        Ok(())
    }

    /// Reloads the library list (for the picker) and clamps the selection.
    pub async fn refresh_libraries(&mut self) -> Result<(), sqlx::Error> {
        self.libraries = db::list_libraries(&self.pool).await?;
        if self.picker_index > self.libraries.len() {
            self.picker_index = self.libraries.len();
        }
        Ok(())
    }

    /// True while a long-running operation is animating (progress gauge /
    /// spinner), so the loop should keep repainting even without input.
    pub fn is_busy(&self) -> bool {
        self.scan_receiver.is_some()
            || self.validating_receiver.is_some()
            || self.enrich_receiver.is_some()
            || self.watch_scan_active
    }

    /// Sets the transient bottom-bar status message and mirrors it to the log.
    pub fn set_status(&mut self, level: StatusLevel, text: impl Into<String>) {
        let text = text.into();
        match level {
            StatusLevel::Error => tracing::error!("{text}"),
            StatusLevel::Warning => tracing::warn!("{text}"),
            _ => tracing::info!("{text}"),
        }
        self.status_message = Some(StatusMessage {
            text,
            level,
            at: std::time::Instant::now(),
        });
        self.needs_redraw = true;
    }

    pub fn current_tab_mut(&mut self) -> &mut TabData {
        &mut self.tabs[self.current_tab]
    }

    /// Resets the cancel flag before starting a long operation and hands back a
    /// clone for the worker to poll.
    pub fn begin_cancelable(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.cancel
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.cancel.clone()
    }

    /// Signals any running scan/enrichment to stop at the next checkpoint.
    pub fn request_cancel(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.set_status(StatusLevel::Warning, "Cancelling…");
    }

    /// Toggles background watch mode on/off.
    pub fn toggle_watch(&mut self) {
        if self.watch_enabled {
            self.watcher = None;
            self.watch_rx = None;
            self.watch_enabled = false;
            self.watch_pending = false;
            self.set_status(StatusLevel::Info, "Watch mode off.");
            return;
        }
        let Some(path) = self.pending_scan_path.clone() else {
            self.set_status(StatusLevel::Warning, "No library path to watch.");
            return;
        };
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        match watch_dir(&path, tx) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.watch_rx = Some(rx);
                self.watch_enabled = true;
                self.set_status(StatusLevel::Success, format!("Watching {}", path.display()));
            }
            Err(e) => self.set_status(StatusLevel::Error, format!("Watch failed: {e}")),
        }
    }

    /// Pumps the watch pipeline: drains filesystem signals, debounces them into
    /// a background import, and applies a completed import. Called each frame.
    pub async fn tick_watch(&mut self) {
        if let Some(rx) = &mut self.watch_rx {
            let mut saw = false;
            while rx.try_recv().is_ok() {
                saw = true;
            }
            if saw {
                self.watch_pending = true;
                self.watch_last_event = std::time::Instant::now();
            }
        }

        // Apply a finished background import.
        if let Some(rx) = &mut self.watch_scan_rx {
            if let Ok(ScanEvent::Done) = rx.try_recv() {
                self.watch_scan_rx = None;
                self.watch_scan_active = false;
                self.reload().await.ok();
                self.set_status(StatusLevel::Success, "Watch: library updated.");
            }
        }

        // Kick off a debounced import once the directory settles.
        let idle = self.watch_last_event.elapsed() > std::time::Duration::from_millis(1500);
        if self.watch_pending && idle && !self.watch_scan_active && self.scan_receiver.is_none() {
            self.watch_pending = false;
            if let Some(path) = self.pending_scan_path.clone() {
                let (tx, rx) = tokio::sync::mpsc::channel(100);
                self.watch_scan_rx = Some(rx);
                self.watch_scan_active = true;
                let cancel = self.cancel.clone();
                tokio::spawn(scan_library(
                    self.pool.clone(),
                    self.active_library_id(),
                    path,
                    self.config.formats.clone(),
                    self.config.scan_concurrency,
                    cancel,
                    tx,
                ));
            }
        }
    }

    /// Loads the album/artist group summaries for the grouped Library view.
    pub async fn refresh_groups(&mut self) -> Result<(), sqlx::Error> {
        self.groups = if self.group_mode == GroupMode::Off {
            Vec::new()
        } else {
            db::load_groups(&self.pool, self.group_mode, self.active_library_id()).await?
        };
        self.groups_state
            .select((!self.groups.is_empty()).then_some(0));
        Ok(())
    }

    /// Reloads just the active tab (after a search/filter/sort change).
    pub async fn reload_current_tab(&mut self) -> Result<(), sqlx::Error> {
        let pool = self.pool.clone();
        let library_id = self.active_library_id();
        reload_tab(&pool, &mut self.tabs[self.current_tab], library_id).await
    }

    /// Debounced search: re-runs the active tab's query a short moment after the
    /// last keystroke so fast typing doesn't hit the DB on every character.
    pub async fn commit_search_if_due(&mut self) {
        if self.search_dirty
            && self.search_last_edit.elapsed() > std::time::Duration::from_millis(180)
        {
            self.search_dirty = false;
            self.reload_current_tab().await.ok();
            self.needs_redraw = true;
        }
    }

    /// Clears the status message once it has been on screen long enough.
    pub fn expire_status(&mut self) {
        if let Some(msg) = &self.status_message {
            if msg.at.elapsed() > std::time::Duration::from_secs(6) {
                self.status_message = None;
                // Repaint once to actually erase the message from the screen.
                self.needs_redraw = true;
            }
        }
    }

    /// Refreshes just the summary counts shown in the tab badges. Cheap (a
    /// handful of COUNT queries), so it runs on every mutation even though the
    /// row data is reloaded lazily.
    pub async fn refresh_counts(&mut self) -> Result<(), sqlx::Error> {
        let lib = self.active_library_id();
        self.library_stats.total_tracks = db::count_tracks(&self.pool, None, lib).await? as u32;
        self.library_stats.total_pending =
            db::count_tracks(&self.pool, Some("pending"), lib).await? as u32;
        self.library_stats.total_duplicates =
            db::count_duplicate_groups(&self.pool, lib).await? as u32;
        self.library_stats.total_missing =
            db::count_tracks(&self.pool, Some("missing"), lib).await? as u32;
        self.library_stats.total_marked = db::count_marked(&self.pool, lib).await? as u32;
        Ok(())
    }

    /// True when the active tab is the specially-rendered Duplicates tab.
    fn on_duplicates_tab(&self) -> bool {
        self.tabs[self.current_tab].label == "Duplicates"
    }

    /// Applies a mutation's effects: refreshes counts, reloads the *visible* tab
    /// (and the Duplicates view if it's showing) right away, and marks every
    /// other tab dirty so it reloads the next time it's shown. This replaces the
    /// old behavior of eagerly re-querying all six tabs on every change.
    pub async fn reload(&mut self) -> Result<(), sqlx::Error> {
        self.refresh_counts().await?;

        let pool = self.pool.clone();
        let library_id = self.active_library_id();
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if i == self.current_tab {
                reload_tab(&pool, tab, library_id).await?;
                tab.dirty = false;
            } else {
                tab.dirty = true;
            }
        }

        if self.on_duplicates_tab() {
            self.duplicates.reload(&self.pool, library_id).await?;
            self.duplicates_dirty = false;
        } else {
            self.duplicates_dirty = true;
        }

        self.properties_panel_open = false;
        self.properties_of_track = None;
        self.properties_scroll = 0;
        self.needs_redraw = true;

        Ok(())
    }

    /// Reloads the active tab (and Duplicates view) if a prior mutation marked it
    /// dirty. Called once per event-loop iteration so a tab switch picks up any
    /// pending changes before the next draw.
    pub async fn ensure_fresh(&mut self) -> Result<(), sqlx::Error> {
        if self.tabs[self.current_tab].dirty {
            self.reload_current_tab().await?;
            self.tabs[self.current_tab].dirty = false;
            self.needs_redraw = true;
        }
        if self.on_duplicates_tab() && self.duplicates_dirty {
            self.duplicates
                .reload(&self.pool, self.active_library_id())
                .await?;
            self.duplicates_dirty = false;
            self.needs_redraw = true;
        }
        Ok(())
    }
}

/// Loads a tab's tracks honoring its base status filter, remembered search
/// query, facet filters, and sort order, then clamps the selection.
pub async fn reload_tab(
    pool: &SqlitePool,
    tab: &mut TabData,
    library_id: i64,
) -> Result<(), sqlx::Error> {
    let query = (!tab.search_query.is_empty()).then(|| tab.search_query.clone());

    let mut tracks = match tab.source {
        TabSource::Status(status_filter) => {
            db::load_tracks(pool, None, status_filter, query.as_deref(), library_id).await?
        }
        TabSource::Marked => db::load_marked_tracks(pool, query.as_deref(), library_id).await?,
        TabSource::DeadLetter => db::load_dead_letter(pool, query.as_deref(), library_id).await?,
    };

    if tab.filter.is_active() {
        tracks.retain(|track| tab.filter.matches(track));
    }

    tab.tracks = tracks;
    tab.apply_sort();

    if tab.tracks.is_empty() {
        tab.state.select(None);
    } else {
        let sel = tab.state.selected().unwrap_or(0).min(tab.tracks.len() - 1);
        tab.state.select(Some(sel));
    }
    Ok(())
}

/// Where a tab's rows come from.
#[derive(Clone, Copy)]
pub enum TabSource {
    /// Normal status-filtered list (`None` = whole library).
    Status(Option<&'static str>),
    /// Tracks flagged for deletion (the Trash tab).
    Marked,
    /// Enrichment dead letters: `failed` + `not_found`.
    DeadLetter,
}

pub struct TabData {
    pub label: &'static str,
    pub source: TabSource,
    pub tracks: Vec<TrackSummary>,
    pub state: TableState,
    pub sort: SortKey,
    pub sort_desc: bool,
    /// Active facet filters, applied on top of the tab's base query.
    pub filter: Filter,
    /// Remembered search query for this tab.
    pub search_query: String,
    /// When true, this tab's rows are stale and will be reloaded the next time
    /// it becomes the active tab (see `App::ensure_fresh`).
    pub dirty: bool,
}

impl TabData {
    pub fn new(label: &'static str, source: TabSource, tracks: Vec<TrackSummary>) -> Self {
        Self {
            label,
            source,
            tracks,
            state: TableState::default(),
            sort: SortKey::Added,
            sort_desc: false,
            filter: Filter::default(),
            search_query: String::new(),
            dirty: false,
        }
    }

    /// Re-applies the active sort to the loaded tracks in place.
    pub fn apply_sort(&mut self) {
        sort_tracks(&mut self.tracks, self.sort, self.sort_desc);
    }

    /// Advances to the next sort key (wrapping) and re-sorts.
    pub fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.apply_sort();
    }

    /// Flips sort direction and re-sorts.
    pub fn toggle_sort_dir(&mut self) {
        self.sort_desc = !self.sort_desc;
        self.apply_sort();
    }
}

/// Facet filters applied in memory on top of a tab's base query.
#[derive(Default, Clone)]
pub struct Filter {
    pub format: Option<String>,
    pub min_bitrate: Option<u32>,
    pub no_isrc: bool,
    pub unhealthy: bool,
}

impl Filter {
    pub fn is_active(&self) -> bool {
        self.format.is_some() || self.min_bitrate.is_some() || self.no_isrc || self.unhealthy
    }

    pub fn clear(&mut self) {
        *self = Filter::default();
    }

    /// Returns true if the track passes all active facets.
    pub fn matches(&self, t: &TrackSummary) -> bool {
        if let Some(fmt) = &self.format {
            if t.file_format
                .as_deref()
                .map(|f| f.eq_ignore_ascii_case(fmt))
                != Some(true)
            {
                return false;
            }
        }
        if let Some(min) = self.min_bitrate {
            if t.bitrate.unwrap_or(0) < min {
                return false;
            }
        }
        if self.no_isrc && t.isrc.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
            return false;
        }
        if self.unhealthy && t.health_issue.is_none() {
            return false;
        }
        true
    }

    /// Short human-readable description for the filter indicator.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(f) = &self.format {
            parts.push(format!("fmt={f}"));
        }
        if let Some(b) = self.min_bitrate {
            parts.push(format!("≥{b}kbps"));
        }
        if self.no_isrc {
            parts.push("no-isrc".to_string());
        }
        if self.unhealthy {
            parts.push("unhealthy".to_string());
        }
        parts.join(" ")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Added,
    Title,
    Artist,
    Album,
    Format,
    Size,
    Duration,
    Bitrate,
    Status,
}

impl SortKey {
    pub fn next(self) -> SortKey {
        use SortKey::*;
        match self {
            Added => Title,
            Title => Artist,
            Artist => Album,
            Album => Format,
            Format => Size,
            Size => Duration,
            Duration => Bitrate,
            Bitrate => Status,
            Status => Added,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Added => "Added",
            SortKey::Title => "Title",
            SortKey::Artist => "Artist",
            SortKey::Album => "Album",
            SortKey::Format => "Format",
            SortKey::Size => "Size",
            SortKey::Duration => "Duration",
            SortKey::Bitrate => "Bitrate",
            SortKey::Status => "Status",
        }
    }
}

/// Sorts a track list by the given key/direction. String keys sort
/// case-insensitively with missing values pushed to the end.
pub fn sort_tracks(tracks: &mut [TrackSummary], key: SortKey, desc: bool) {
    fn opt_str(s: &Option<String>) -> String {
        // Missing values sort last (ascending) by prefixing a high sentinel.
        match s {
            Some(v) => format!("0{}", v.to_lowercase()),
            None => "1".to_string(),
        }
    }
    match key {
        SortKey::Added => tracks.sort_by_key(|t| t.id.unwrap_or(i64::MAX)),
        SortKey::Title => tracks.sort_by_key(|t| opt_str(&t.title)),
        SortKey::Artist => tracks.sort_by_key(|t| opt_str(&t.artist)),
        SortKey::Album => tracks.sort_by_key(|t| opt_str(&t.album)),
        SortKey::Format => tracks.sort_by_key(|t| opt_str(&t.file_format)),
        SortKey::Size => tracks.sort_by_key(|t| t.file_size.unwrap_or(-1)),
        SortKey::Duration => tracks.sort_by_key(|t| t.duration.unwrap_or(0)),
        SortKey::Bitrate => tracks.sort_by_key(|t| t.bitrate.unwrap_or(0)),
        SortKey::Status => tracks.sort_by(|a, b| a.status.cmp(&b.status)),
    }
    if desc {
        tracks.reverse();
    }
}

pub struct LibraryStats {
    // these values are pulled from DB on app startup
    pub total_tracks: u32,
    pub total_pending: u32,
    pub total_duplicates: u32,
    pub total_missing: u32,
    pub total_marked: u32,
}

pub enum Screens {
    /// Choose an existing library or create a new one (no library open yet).
    Picker,
    /// Enter a path + name for a new library.
    CreateLibrary,
    Start,
    Main,
    Scanning,
    Stats,
}

/// Which field the create-library form is editing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NewLibField {
    Path,
    Name,
}

#[derive(Clone, Copy)]
pub enum StatusLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GroupMode {
    Off,
    Artist,
    Album,
}

impl GroupMode {
    pub fn next(self) -> GroupMode {
        match self {
            GroupMode::Off => GroupMode::Artist,
            GroupMode::Artist => GroupMode::Album,
            GroupMode::Album => GroupMode::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GroupMode::Off => "off",
            GroupMode::Artist => "artist",
            GroupMode::Album => "album",
        }
    }
}

pub struct StatusMessage {
    pub text: String,
    pub level: StatusLevel,
    pub at: std::time::Instant,
}

/// In-progress manual tag edit for one track. `fields` are (label, value) pairs
/// in a fixed order; `focus` is the field currently being typed into.
pub struct EditState {
    pub track_id: i64,
    pub file_path: std::path::PathBuf,
    pub fields: Vec<(&'static str, String)>,
    pub focus: usize,
}

impl EditState {
    pub fn from_track(track: &TrackInfo) -> Option<Self> {
        let id = track.id?;
        let s = |o: &Option<String>| o.clone().unwrap_or_default();
        let n = |o: &Option<u32>| o.map(|v| v.to_string()).unwrap_or_default();
        Some(Self {
            track_id: id,
            file_path: track.file_path.clone(),
            fields: vec![
                ("Title", s(&track.title)),
                ("Artist", s(&track.artist)),
                ("Album", s(&track.album)),
                ("Album Artist", s(&track.album_artist)),
                ("Genre", s(&track.genre)),
                ("Comment", s(&track.comment)),
                ("Track #", n(&track.track)),
                ("Disc #", n(&track.disc)),
                ("Year", n(&track.release_year)),
            ],
            focus: 0,
        })
    }

    fn value(&self, label: &str) -> String {
        self.fields
            .iter()
            .find(|(l, _)| *l == label)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    /// Builds the tag-edit payload from the current field values.
    pub fn to_tag_edits(&self) -> crate::track::TagEdits {
        crate::track::TagEdits {
            title: self.value("Title"),
            artist: self.value("Artist"),
            album: self.value("Album"),
            album_artist: self.value("Album Artist"),
            genre: self.value("Genre"),
            comment: self.value("Comment"),
            track: self.value("Track #"),
            disc: self.value("Disc #"),
            year: self.value("Year"),
        }
    }

    pub fn focus_prev(&mut self) {
        self.focus = self.focus.saturating_sub(1);
    }

    pub fn focus_next(&mut self) {
        if self.focus + 1 < self.fields.len() {
            self.focus += 1;
        }
    }

    pub fn push_char(&mut self, c: char) {
        if let Some((_, v)) = self.fields.get_mut(self.focus) {
            v.push(c);
        }
    }

    pub fn backspace(&mut self) {
        if let Some((_, v)) = self.fields.get_mut(self.focus) {
            v.pop();
        }
    }
}

pub struct DuplicatesView {
    pub groups: Vec<DuplicateGroupSummary>,
    pub groups_state: TableState,
    pub selected_members: Vec<TrackSummary>,
    /// Index (column) of the highlighted member - the keeper candidate. The
    /// members grid is transposed (one column per file), so member selection is
    /// a column index rather than a TableState row.
    pub selected_member: usize,
    pub focus: DuplicatePane,
    pub column_offset: usize,
    /// Library these groups belong to, so member loads stay scoped.
    pub library_id: i64,
}

pub enum DuplicatePane {
    Groups,
    Members,
}

impl DuplicatesView {
    /// An empty view used before any library is opened.
    pub fn empty() -> Self {
        Self {
            groups: Vec::new(),
            groups_state: TableState::default(),
            selected_members: Vec::new(),
            selected_member: 0,
            focus: DuplicatePane::Groups,
            column_offset: 0,
            library_id: 0,
        }
    }

    pub async fn new(pool: &SqlitePool, library_id: i64) -> Result<Self, sqlx::Error> {
        let groups = db::load_duplicate_groups(pool, library_id).await?;
        let mut groups_state = TableState::default();
        let mut selected_member = 0;
        let selected_members = if !groups.is_empty() {
            groups_state.select(Some(0));
            let members =
                db::load_duplicate_members(pool, groups[0].kind, &groups[0].key, library_id).await?;
            selected_member = suggest_keeper(&members).unwrap_or(0);
            members
        } else {
            Vec::new()
        };
        Ok(Self {
            groups,
            groups_state,
            selected_members,
            selected_member,
            focus: DuplicatePane::Groups,
            column_offset: 0,
            library_id,
        })
    }
    pub async fn reload(&mut self, pool: &SqlitePool, library_id: i64) -> Result<(), sqlx::Error> {
        *self = Self::new(pool, library_id).await?;
        Ok(())
    }

    pub async fn select_group(&mut self, pool: &SqlitePool, idx: usize) -> Result<(), sqlx::Error> {
        if idx >= self.groups.len() {
            return Ok(());
        }
        self.groups_state.select(Some(idx));
        self.selected_members = db::load_duplicate_members(
            pool,
            self.groups[idx].kind,
            &self.groups[idx].key,
            self.library_id,
        )
        .await?;
        // Pre-highlight the suggested keeper so the user can accept or override.
        self.selected_member = suggest_keeper(&self.selected_members).unwrap_or(0);
        self.column_offset = 0;
        Ok(())
    }

    /// Moves the keeper-candidate highlight between member columns.
    pub fn select_member(&mut self, delta: isize) {
        if self.selected_members.is_empty() {
            return;
        }
        let max = self.selected_members.len() - 1;
        let cur = self.selected_member as isize;
        self.selected_member = (cur + delta).clamp(0, max as isize) as usize;
    }

    /// Resolves the currently selected group by keeping the highlighted member
    /// and flagging every other member of the group for deletion. Returns the
    /// ids that were flagged (for the undo log).
    pub async fn keep_selected(&mut self, pool: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
        let Some(keeper) = self.selected_members.get(self.selected_member) else {
            return Ok(Vec::new());
        };
        let Some(keeper_id) = keeper.id else {
            return Ok(Vec::new());
        };
        let ids: Vec<i64> = self.selected_members.iter().filter_map(|m| m.id).collect();
        db::mark_group_except(pool, &ids, keeper_id).await?;
        let flagged: Vec<i64> = ids.into_iter().filter(|id| *id != keeper_id).collect();
        Ok(flagged)
    }
}

/// Ranks duplicate-group members and returns the index of the best candidate
/// to keep: highest audio bitrate, then largest file, then most-complete tags.
/// Used to pre-select a sensible default keeper.
pub fn suggest_keeper(members: &[TrackSummary]) -> Option<usize> {
    members
        .iter()
        .enumerate()
        .max_by_key(|(_, m)| {
            let tag_completeness = m.title.is_some() as u8
                + m.artist.is_some() as u8
                + m.album.is_some() as u8
                + m.isrc.is_some() as u8;
            (
                m.bitrate.unwrap_or(0),
                m.file_size.unwrap_or(0),
                tag_completeness,
            )
        })
        .map(|(idx, _)| idx)
}
