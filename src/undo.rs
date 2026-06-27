// src/undo.rs
//
// Reversible action history. Each mutating action stores an `UndoAction` (the
// operation that reverses it) in the `action_log` table; pressing undo pops the
// most recent one and applies it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::db;
use crate::track::TrackInfo;

type UndoResult = Result<String, Box<dyn std::error::Error + Send + Sync>>;

/// A purged track captured so it can be restored from quarantine.
#[derive(Serialize, Deserialize)]
pub struct RestoredTrack {
    pub track: TrackInfo,
    pub original_path: PathBuf,
    pub quarantine_path: PathBuf,
    /// The library the track was purged from, so undo restores it in place.
    /// Defaults to 0 for payloads written before multi-library support.
    #[serde(default)]
    pub library_id: i64,
}

/// The operation that undoes a logged action.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum UndoAction {
    /// Re-set the deletion flag on a set of tracks.
    SetMarked { ids: Vec<i64>, value: bool },
    /// Restore previous statuses.
    SetStatus { items: Vec<(i64, String)> },
    /// Move purged files back from quarantine and re-insert their rows.
    RestoreRows { tracks: Vec<RestoredTrack> },
}

impl UndoAction {
    pub async fn apply(&self, pool: &SqlitePool) -> UndoResult {
        match self {
            UndoAction::SetMarked { ids, value } => {
                for id in ids {
                    db::set_marked_for_deletion(pool, *id, *value).await?;
                }
                Ok(format!("restored {} flag(s)", ids.len()))
            }
            UndoAction::SetStatus { items } => {
                for (id, status) in items {
                    db::update_track_status(pool, *id, status).await?;
                }
                Ok(format!("reverted {} status change(s)", items.len()))
            }
            UndoAction::RestoreRows { tracks } => {
                let mut restored = 0;
                for rt in tracks {
                    if let Some(parent) = rt.original_path.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    // Move the file back; tolerate it already being in place.
                    if rt.quarantine_path.exists() {
                        move_file(&rt.quarantine_path, &rt.original_path)?;
                    }
                    db::insert_track_pool(pool, &rt.track, rt.library_id).await?;
                    restored += 1;
                }
                Ok(format!("restored {restored} purged track(s)"))
            }
        }
    }
}

/// Moves a file, falling back to copy+remove across filesystems.
pub fn move_file(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
    }
}
