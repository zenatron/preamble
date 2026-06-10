// src/export.rs
//
// Exporters for the current view: CSV, JSON, M3U playlists, and a duplicate
// report. All write timestamped files into the working directory.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::track::TrackSummary;

type ExportResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Unix-seconds stamp used to make export filenames unique.
fn stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Builds an export path like `preamble-<tab>-<stamp>.<ext>` (sanitized).
pub fn export_path(tab: &str, ext: &str) -> PathBuf {
    let tab = tab
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric(), "");
    PathBuf::from(format!("preamble-{tab}-{}.{ext}", stamp()))
}

pub fn export_csv(tracks: &[TrackSummary], path: &Path) -> ExportResult {
    let mut writer = csv::Writer::from_path(path)?;
    for track in tracks {
        writer.serialize(track)?;
    }
    writer.flush()?;
    Ok(())
}

pub fn export_json(tracks: &[TrackSummary], path: &Path) -> ExportResult {
    let json = serde_json::to_string_pretty(tracks)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Extended M3U playlist with `#EXTINF` runtime/artist/title lines.
pub fn export_m3u(tracks: &[TrackSummary], path: &Path) -> ExportResult {
    let mut out = String::from("#EXTM3U\n");
    for t in tracks {
        let secs = t.duration.map(|d| d / 1000).unwrap_or(0);
        let artist = t.artist.as_deref().unwrap_or("");
        let title = t.title.as_deref().unwrap_or("");
        out.push_str(&format!("#EXTINF:{secs},{artist} - {title}\n"));
        out.push_str(&format!("{}\n", t.file_path.display()));
    }
    std::fs::write(path, out)?;
    Ok(())
}

/// One row of the duplicate report.
#[derive(Serialize)]
pub struct DuplicateReportRow {
    pub group_kind: &'static str,
    pub group_key: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub bitrate: String,
    pub size_bytes: String,
    pub file_path: String,
}

pub fn export_duplicate_report(rows: &[DuplicateReportRow], path: &Path) -> ExportResult {
    let mut writer = csv::Writer::from_path(path)?;
    for row in rows {
        writer.serialize(row)?;
    }
    writer.flush()?;
    Ok(())
}
