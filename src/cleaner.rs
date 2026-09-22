use crate::{
    config::Config,
    parser::{normalize_series_name, parse_quality, series_names_match},
    postprocess::video_files,
};
use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn local_path(path: &Path) -> bool {
    let value = path.to_string_lossy().to_ascii_lowercase();
    !["http://", "https://", "ftp://", "smb://", "nfs://"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

fn duplicate_target(trash: &Path, file: &Path) -> PathBuf {
    let name = file.file_name().unwrap_or_default();
    let candidate = trash.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = file
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("duplicate");
    let extension = file
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let mut index = 1;
    loop {
        let candidate = trash.join(format!("{stem}__duplicate_{index}{extension}"));
        if !candidate.exists() {
            return candidate;
        }
        index += 1;
    }
}

fn handle_duplicate(file: &Path, cfg: &Config) -> Result<()> {
    if cfg.cleanup_action == "delete" {
        fs::remove_file(file)?;
    } else {
        let Some(trash) = &cfg.trash_path else {
            anyhow::bail!("trash_path is required when cleanup_action is move");
        };
        fs::create_dir_all(trash)?;
        fs::rename(file, duplicate_target(trash, file))?;
    }
    Ok(())
}

/// Moves a file or directory to the trash with a unique name; falls back to a
/// recursive copy + remove when the trash lives on another filesystem.
pub fn move_to_trash(source: &Path, trash: &Path) -> Result<PathBuf> {
    fs::create_dir_all(trash)?;
    let target = duplicate_target(trash, source);
    if fs::rename(source, &target).is_ok() {
        return Ok(target);
    }
    copy_recursive(source, &target)?;
    if source.is_dir() {
        fs::remove_dir_all(source)?;
    } else {
        fs::remove_file(source)?;
    }
    Ok(target)
}

fn copy_recursive(source: &Path, target: &Path) -> Result<()> {
    if source.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, target)?;
    }
    Ok(())
}

pub fn resolve_existing_target(
    source: &Path,
    target: &Path,
    new_score: i64,
    cfg: &Config,
) -> Result<bool> {
    if !target.exists() {
        return Ok(false);
    }
    let old_score = target
        .file_name()
        .and_then(|value| value.to_str())
        .map(crate::parser::parse_quality)
        .map(|quality| quality.score_with_settings(&cfg.settings))
        .unwrap_or(0);
    if old_score + cfg.upgrade_min_score_diff >= new_score {
        handle_duplicate(source, cfg)?;
        return Ok(true);
    }
    if !cfg.cleanup_upgrades {
        anyhow::bail!(
            "refusing to overwrite existing file without cleanup_upgrades: {}",
            target.display()
        );
    }
    handle_duplicate(target, cfg)?;
    Ok(false)
}

pub fn cleanup_old_episode(
    cfg: &Config,
    series: &str,
    season: i64,
    episode: i64,
    new_score: i64,
    new_file: &Path,
    archive_path: &Path,
) -> Result<usize> {
    if !cfg.cleanup_upgrades || !local_path(archive_path) || !archive_path.is_dir() {
        return Ok(0);
    }
    let pattern = crate::utils::cached_regex(
        r"(?i)^(?P<name>.+?)[ ._-]+s(?P<season>\d{1,2})e(?P<episode>\d{1,4})(?:[ ._-]|$)",
    )?;
    let normalized = normalize_series_name(series);
    let mut removed = 0;
    for file in video_files(archive_path)? {
        if file == new_file || file.file_name() == new_file.file_name() {
            continue;
        }
        let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(captures) = pattern.captures(name) else {
            continue;
        };
        if captures
            .name("season")
            .and_then(|value| value.as_str().parse::<i64>().ok())
            != Some(season)
            || captures
                .name("episode")
                .and_then(|value| value.as_str().parse::<i64>().ok())
                != Some(episode)
        {
            continue;
        }
        let file_series = captures
            .name("name")
            .map(|value| value.as_str())
            .unwrap_or_default();
        if !series_names_match(&normalized, file_series) {
            continue;
        }
        let old_score = parse_quality(name).score();
        if old_score + cfg.cleanup_min_score_diff >= new_score {
            continue;
        }
        handle_duplicate(&file, cfg)?;
        removed += 1;
    }
    Ok(removed)
}

pub fn discard_if_inferior(
    cfg: &Config,
    series: &str,
    season: i64,
    episode: i64,
    new_score: i64,
    new_file: &Path,
    archive_path: &Path,
) -> Result<bool> {
    if !cfg.cleanup_upgrades || !local_path(archive_path) || !archive_path.is_dir() {
        return Ok(false);
    }
    let pattern = crate::utils::cached_regex(
        r"(?i)^(?P<name>.+?)[ ._-]+s(?P<season>\d{1,2})e(?P<episode>\d{1,4})(?:[ ._-]|$)",
    )?;
    let normalized = normalize_series_name(series);
    for file in video_files(archive_path)? {
        if file == new_file || file.file_name() == new_file.file_name() {
            continue;
        }
        let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(captures) = pattern.captures(name) else {
            continue;
        };
        if captures
            .name("season")
            .and_then(|value| value.as_str().parse::<i64>().ok())
            != Some(season)
            || captures
                .name("episode")
                .and_then(|value| value.as_str().parse::<i64>().ok())
                != Some(episode)
        {
            continue;
        }
        let file_series = captures
            .name("name")
            .map(|value| value.as_str())
            .unwrap_or_default();
        if !series_names_match(&normalized, file_series) {
            continue;
        }
        let old_score = parse_quality(name).score();
        if old_score >= new_score.saturating_add(cfg.cleanup_min_score_diff) {
            handle_duplicate(new_file, cfg)?;
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn cleanup_old_movie(
    cfg: &Config,
    movie: &str,
    year: Option<i64>,
    new_score: i64,
    new_file: &Path,
    archive_path: &Path,
) -> Result<usize> {
    if !cfg.cleanup_upgrades || !local_path(archive_path) || !archive_path.is_dir() {
        return Ok(0);
    }
    let words = normalize_series_name(movie)
        .split_whitespace()
        .filter(|word| word.len() > 1)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut removed = 0;
    for file in video_files(archive_path)? {
        if file == new_file || file.file_name() == new_file.file_name() {
            continue;
        }
        let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let normalized_name = normalize_series_name(name);
        if !words.iter().all(|word| normalized_name.contains(word)) {
            continue;
        }
        if let Some(year) = year {
            let has_year = crate::utils::cached_regex(r"\b(19\d{2}|20\d{2})\b")?
                .captures(name)
                .and_then(|capture| capture.get(1))
                .and_then(|value| value.as_str().parse::<i64>().ok())
                .is_some_and(|old_year| (old_year - year).abs() <= 1);
            if !has_year {
                continue;
            }
        }
        let old_score = parse_quality(name).score();
        if old_score.saturating_add(cfg.cleanup_min_score_diff) >= new_score {
            continue;
        }
        handle_duplicate(&file, cfg)?;
        removed += 1;
    }
    Ok(removed)
}

pub fn discard_if_inferior_movie(
    cfg: &Config,
    movie: &str,
    year: Option<i64>,
    new_score: i64,
    new_file: &Path,
    archive_path: &Path,
) -> Result<bool> {
    if !cfg.cleanup_upgrades || !local_path(archive_path) || !archive_path.is_dir() {
        return Ok(false);
    }
    let words = normalize_series_name(movie)
        .split_whitespace()
        .filter(|word| word.len() > 1)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let year_pattern = crate::utils::cached_regex(r"\b(19\d{2}|20\d{2})\b")?;
    for file in video_files(archive_path)? {
        if file == new_file || file.file_name() == new_file.file_name() {
            continue;
        }
        let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let normalized_name = normalize_series_name(name);
        if !words.iter().all(|word| normalized_name.contains(word)) {
            continue;
        }
        if let Some(year) = year {
            let has_year = year_pattern
                .captures(name)
                .and_then(|capture| capture.get(1))
                .and_then(|value| value.as_str().parse::<i64>().ok())
                .is_some_and(|old_year| (old_year - year).abs() <= 1);
            if !has_year {
                continue;
            }
        }
        let old_score = parse_quality(name).score();
        if old_score >= new_score.saturating_add(cfg.cleanup_min_score_diff) {
            handle_duplicate(new_file, cfg)?;
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn moves_only_lower_quality_matching_episode_to_trash() {
        let root = std::env::temp_dir().join(format!(
            "rextto-cleaner-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = root.join("archive");
        let trash = root.join("trash");
        fs::create_dir_all(&archive).unwrap();
        fs::write(archive.join("Example.S01E01.720p.WEB-DL.mkv"), b"old").unwrap();
        let mut cfg = Config::default();
        cfg.cleanup_upgrades = true;
        cfg.trash_path = Some(trash.clone());
        let kept = archive.join("Example - S01E01 - Title.mkv");
        fs::write(&kept, b"new").unwrap();
        assert_eq!(
            cleanup_old_episode(
                &cfg,
                "Example",
                1,
                1,
                parse_quality("Example.S01E01.1080p.WEB-DL.mkv").score(),
                &kept,
                &archive
            )
            .unwrap(),
            1
        );
        assert!(trash.join("Example.S01E01.720p.WEB-DL.mkv").is_file());
        assert!(kept.is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discards_new_episode_when_existing_file_is_better() {
        let root = std::env::temp_dir().join(format!(
            "rextto-cleaner-inferior-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = root.join("archive");
        let trash = root.join("trash");
        fs::create_dir_all(&archive).unwrap();
        fs::write(archive.join("Example.S01E01.1080p.WEB-DL.mkv"), b"old").unwrap();
        let new_file = archive.join("Example - S01E01 - Title.mkv");
        fs::write(&new_file, b"new").unwrap();
        let mut cfg = Config::default();
        cfg.cleanup_upgrades = true;
        cfg.trash_path = Some(trash.clone());
        assert!(discard_if_inferior(
            &cfg,
            "Example",
            1,
            1,
            parse_quality("Example.S01E01.720p.WEB-DL.mkv").score(),
            &new_file,
            &archive
        )
        .unwrap());
        assert!(!new_file.exists());
        assert!(trash.join("Example - S01E01 - Title.mkv").is_file());
        assert!(archive.join("Example.S01E01.1080p.WEB-DL.mkv").is_file());
        let _ = fs::remove_dir_all(root);
    }
}
