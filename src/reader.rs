// src/reader.rs

use std::fs::{self};
use std::path::{Path, PathBuf};

pub fn collect_paths(
    path: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_paths(&path, paths)?;
            } else {
                if path.extension().and_then(|e| e.to_str()) == Some("flac") {
                    paths.push(path);
                }
            }
        }
    }
    Ok(())
}
