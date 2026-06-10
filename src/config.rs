// src/config.rs
//
// Runtime configuration for preamble. Loaded from `preamble.toml` in the
// working directory (auto-created with defaults on first run). Secrets such as
// the AcoustID API key are read from the environment / `.env` so they never
// have to live in the committed config file.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_PATH: &str = "preamble.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Default music directory to scan when no path is passed on the CLI.
    pub library_path: Option<PathBuf>,

    /// File extensions (lowercase, without the dot) that count as audio files.
    pub formats: Vec<String>,

    /// Number of files hashed/tag-read concurrently during a scan.
    pub scan_concurrency: usize,

    /// AcoustID application API key. Usually left blank here and supplied via
    /// the `ACOUSTID_API_KEY` environment variable (or a `.env` file).
    pub acoustid_api_key: Option<String>,

    /// `tracing` log level written to `preamble.log` (error/warn/info/debug/trace).
    pub log_level: String,

    /// Start the filesystem watcher (background auto-scan) on launch.
    pub watch: bool,

    /// Directory that purged files are moved into instead of being deleted.
    pub quarantine_dir: PathBuf,

    /// Bitrate (kbps) below which a track is flagged `low_bitrate` by the
    /// health check.
    pub low_bitrate_threshold: u32,

    /// Skipped during (de)serialization - resolved at load time from the
    /// config field above or the environment.
    #[serde(skip)]
    pub resolved_acoustid_key: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            library_path: None,
            formats: default_formats(),
            scan_concurrency: 8,
            acoustid_api_key: None,
            log_level: "info".to_string(),
            watch: false,
            quarantine_dir: PathBuf::from("quarantine"),
            low_bitrate_threshold: 128,
            resolved_acoustid_key: None,
        }
    }
}

fn default_formats() -> Vec<String> {
    [
        "flac", "mp3", "m4a", "mp4", "ogg", "opus", "aiff", "wav", "wv", "ape", "mpc", "aac",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Config {
    /// Loads config from `preamble.toml`, creating it with defaults if missing.
    /// A malformed file falls back to defaults rather than crashing the app.
    pub fn load() -> Self {
        load_dotenv(Path::new(".env"));

        let mut config = match std::fs::read_to_string(CONFIG_PATH) {
            Ok(contents) => toml::from_str::<Config>(&contents).unwrap_or_default(),
            Err(_) => {
                let config = Config::default();
                let _ = config.write_default();
                config
            }
        };

        // Normalize extensions: lowercase, strip any leading dot.
        config.formats = config
            .formats
            .iter()
            .map(|f| f.trim().trim_start_matches('.').to_lowercase())
            .filter(|f| !f.is_empty())
            .collect();
        if config.formats.is_empty() {
            config.formats = default_formats();
        }

        if config.scan_concurrency == 0 {
            config.scan_concurrency = 1;
        }

        // The env var wins over the (usually empty) config field so secrets can
        // stay out of the committed file.
        config.resolved_acoustid_key = std::env::var("ACOUSTID_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| {
                config
                    .acoustid_api_key
                    .clone()
                    .filter(|k| !k.trim().is_empty())
            });

        config
    }

    fn write_default(&self) -> std::io::Result<()> {
        let header = "# preamble configuration\n\
            # library_path          : default directory scanned when no CLI path is given\n\
            # formats               : audio file extensions to scan (lowercase, no dot)\n\
            # scan_concurrency      : files hashed/read in parallel during a scan\n\
            # acoustid_api_key      : optional; prefer the ACOUSTID_API_KEY env var / .env\n\
            # log_level             : error | warn | info | debug | trace (-> preamble.log)\n\
            # watch                 : auto-scan new files in the background on launch\n\
            # quarantine_dir        : purged files are moved here instead of deleted\n\
            # low_bitrate_threshold : kbps below which the health check flags a track\n\n";
        let body = toml::to_string_pretty(self).unwrap_or_default();
        std::fs::write(CONFIG_PATH, format!("{header}{body}"))
    }

    /// True if a usable AcoustID key was resolved (config or environment).
    pub fn has_acoustid_key(&self) -> bool {
        self.resolved_acoustid_key.is_some()
    }
}

/// Initializes file-based logging to `preamble.log`. The returned guard must be
/// kept alive for the duration of the program so buffered logs are flushed.
pub fn init_logging(config: &Config) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{EnvFilter, fmt};

    let file = tracing_appender::rolling::never(".", "preamble.log");
    let (writer, guard) = tracing_appender::non_blocking(file);

    // RUST_LOG overrides the configured level when present.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("preamble={}", config.log_level)));

    fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .with_target(false)
        .init();

    guard
}

/// reads `KEY=VALUE` lines and sets them in the process, no need for dotenv dep
fn load_dotenv(path: &Path) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !key.is_empty() && std::env::var_os(key).is_none() {
            // SAFETY: called once at startup before any threads read the env.
            // This should be okay since no other threads will be reading or writing env vars
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
}
