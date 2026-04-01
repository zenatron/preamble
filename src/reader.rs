// src/reader.rs

use std::collections::HashSet;
use std::fs::{self};
use std::path::{Path, PathBuf};

pub fn collect_new_paths(
    path: &Path,
    paths: &mut Vec<PathBuf>,
    existing: &HashSet<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_new_paths(&path, paths, existing)?;
            } else {
                if path.extension().and_then(|e| e.to_str()) == Some("flac") {
                    if !existing.contains(path.to_str().unwrap_or("")) {
                        paths.push(path);
                    }
                }
            }
        }
    }
    Ok(())
}
