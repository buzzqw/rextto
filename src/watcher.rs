//! Watched folders: drop a `.torrent` or `.magnet` file into a directory and
//! Rextto adds it automatically.
//!
//! This is qBittorrent's "watched folders" and BiglyBT's `TorrentFolderWatcher`
//! distilled to what Rextto needs: a list of directories, an optional recursive
//! scan, and a "delete after import" flag. The scanner and the magnet reader
//! are pure functions so they can be unit-tested without a running daemon.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatchedFolder {
    /// Directory to scan.
    pub path: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Also scan subdirectories.
    #[serde(default)]
    pub recursive: bool,
    /// Remove the file after a successful add (default true). When false the
    /// worker keeps a processed-file memory so it is not added twice.
    #[serde(default = "default_true")]
    pub delete_after: bool,
}

/// Loads watched folders from the `watched_folders` settings key.
pub fn load_watched_folders(settings: &BTreeMap<String, String>) -> Vec<WatchedFolder> {
    settings
        .get("watched_folders")
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_default()
}

pub fn save_watched_folders(data_dir: &Path, folders: &[WatchedFolder]) -> Result<()> {
    let json = serde_json::to_string(folders)?;
    crate::config::Config::save_setting(data_dir, "watched_folders", &json)
}

pub fn validate_watched_folders(folders: &[WatchedFolder]) -> Option<String> {
    if folders.len() > 50 {
        return Some("too many watched folders (max 50)".into());
    }
    for folder in folders {
        if folder.path.trim().is_empty() || folder.path.len() > 4096 {
            return Some("a watched folder has an empty or oversized path".into());
        }
    }
    None
}

fn is_candidate(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|name| name.to_string_lossy()) else {
        return false;
    };
    if name.starts_with('.') {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".part") || lower.ends_with(".tmp") {
        return false;
    }
    lower.ends_with(".torrent") || lower.ends_with(".magnet")
}

fn visit(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry
            .file_type()
            .map(|kind| kind.is_dir())
            .unwrap_or(false);
        if is_dir {
            if recursive {
                visit(&path, recursive, out);
            }
            continue;
        }
        if is_candidate(&path) {
            out.push(path);
        }
    }
}

/// Lists the `.torrent`/`.magnet` files in a watched folder, sorted for a
/// stable processing order.
pub fn scan_folder(folder: &WatchedFolder) -> Vec<PathBuf> {
    let root = Path::new(folder.path.trim());
    if folder.path.trim().is_empty() || !root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    visit(root, folder.recursive, &mut out);
    out.sort();
    out
}

/// Reads the magnet URI from a `.magnet` file (the file may contain extra
/// text; the first `magnet:?` token wins).
pub fn magnet_from_file(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let start = text.find("magnet:?")?;
    let candidate = &text[start..];
    let end = candidate
        .find(|character: char| {
            character.is_whitespace() || matches!(character, '"' | '\'' | '<' | '>')
        })
        .unwrap_or(candidate.len());
    let magnet = candidate[..end].trim();
    (!magnet.is_empty()).then(|| magnet.to_string())
}

/// Removes a successfully imported file, or renames it to `*.imported` when
/// `delete` is false so the scanner does not pick it up again.
pub fn consume(path: &Path, delete: bool) -> std::io::Result<()> {
    if delete {
        std::fs::remove_file(path)
    } else {
        let target = path.with_extension(format!(
            "{}imported",
            path.extension()
                .map(|extension| format!("{}.", extension.to_string_lossy()))
                .unwrap_or_default()
        ));
        std::fs::rename(path, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rextto-watch-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scans_only_torrent_and_magnet_files_without_noise() {
        let root = temp_dir("scan");
        std::fs::write(root.join("a.torrent"), b"d4:infod4:name3:fooee").unwrap();
        std::fs::write(root.join("b.magnet"), b"magnet:?xt=urn:btih:abc").unwrap();
        std::fs::write(root.join("c.part"), b"partial").unwrap();
        std::fs::write(root.join(".hidden.torrent"), b"hidden").unwrap();
        std::fs::write(root.join("notes.txt"), b"nope").unwrap();
        let nested = root.join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("d.torrent"), b"d4:infod4:name3:baree").unwrap();

        let flat = WatchedFolder {
            path: root.to_string_lossy().into_owned(),
            enabled: true,
            recursive: false,
            delete_after: true,
        };
        let found = scan_folder(&flat);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|path| path.file_name().unwrap() != "c.part"));

        let deep = WatchedFolder {
            recursive: true,
            ..flat.clone()
        };
        assert_eq!(scan_folder(&deep).len(), 3);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn disabled_or_missing_folders_yield_nothing() {
        let missing = WatchedFolder {
            path: "/definitely/not/here".into(),
            enabled: true,
            ..Default::default()
        };
        assert!(scan_folder(&missing).is_empty());
        let empty = WatchedFolder::default();
        assert!(scan_folder(&empty).is_empty());
    }

    #[test]
    fn reads_magnet_uri_from_file() {
        let root = temp_dir("magnet");
        let file = root.join("release.magnet");
        std::fs::write(&file, "Download it here:\nmagnet:?xt=urn:btih:deadbeef&dn=Test\n").unwrap();
        assert_eq!(
            magnet_from_file(&file).unwrap(),
            "magnet:?xt=urn:btih:deadbeef&dn=Test"
        );
        let empty = root.join("empty.magnet");
        std::fs::write(&empty, "no magnet here").unwrap();
        assert!(magnet_from_file(&empty).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn consume_deletes_or_marks_files() {
        let root = temp_dir("consume");
        let deleted = root.join("one.torrent");
        std::fs::write(&deleted, b"x").unwrap();
        consume(&deleted, true).unwrap();
        assert!(!deleted.exists());

        let kept = root.join("two.torrent");
        std::fs::write(&kept, b"x").unwrap();
        consume(&kept, false).unwrap();
        assert!(!kept.exists());
        assert!(root.join("two.torrent.imported").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
