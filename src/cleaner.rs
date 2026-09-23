use crate::{
    config::Config,
    parser::{normalize_series_name, parse_quality, series_names_match},
    postprocess::video_files,
};
use anyhow::Result;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

/// Indice delle migliori qualità presenti su disco nella cartella di una serie.
///
/// Porting di `_best_quality_in_path`/`episode_archive_presence` del legacy:
/// scansiona ricorsivamente l'archivio, raggruppa per `(stagione, episodio)` e
/// tiene il file col punteggio più alto. Serve a non riscaricare una release
/// uguale o peggiore a un file già presente ma non collegato nel DB.
pub fn index_archive(
    series_name: &str,
    archive_path: &Path,
    settings: &std::collections::BTreeMap<String, String>,
) -> crate::models::ArchiveQualityIndex {
    let mut index = crate::models::ArchiveQualityIndex::default();
    if archive_path.as_os_str().is_empty() || !archive_path.is_dir() {
        return index;
    }
    let Ok(pattern) = crate::utils::cached_regex(
        r"(?i)^(?P<name>.+?)[ ._-]+(?:s(?P<s>\d{1,2})e|(?P<ns>\d{1,2})x)(?P<e>\d{1,4})",
    ) else {
        return index;
    };
    let normalized = normalize_series_name(series_name);
    let Ok(files) = video_files(archive_path) else {
        return index;
    };
    for file in files {
        let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(captures) = pattern.captures(name) else {
            continue;
        };
        let Some(file_series) = captures.name("name").map(|value| value.as_str()) else {
            continue;
        };
        if !series_names_match(&normalized, file_series) {
            continue;
        }
        let Some(season) = captures
            .name("s")
            .or_else(|| captures.name("ns"))
            .and_then(|value| value.as_str().parse::<i64>().ok())
        else {
            continue;
        };
        let Some(episode) = captures
            .name("e")
            .and_then(|value| value.as_str().parse::<i64>().ok())
        else {
            continue;
        };
        let quality = parse_quality(name);
        let score = quality.score_with_settings(settings);
        let key = (season, episode);
        let better = index
            .best
            .get(&key)
            .map(|(_, existing)| score > *existing)
            .unwrap_or(true);
        if better {
            index.best.insert(key, (quality, score));
        }
    }
    index
}

/// Motivi di upgrade considerati "forti" anche quando la differenza di score è
/// sotto la soglia configurata (parità con EXTTO `_HARD_UPGRADE_REASONS`).
const HARD_UPGRADE_REASONS: [&str; 4] = ["resolution", "source", "hdr", "repack"];

/// True se `new` migliora `old` per un motivo forte (risoluzione, sorgente,
/// HDR o repack), indipendentemente dal delta di score.
fn hard_upgrade(new: &crate::models::Quality, old: &crate::models::Quality, min_diff: i64) -> bool {
    matches!(
        new.upgrade_reason(old, new.score(), old.score(), min_diff),
        Some(reason) if HARD_UPGRADE_REASONS.contains(&reason)
    )
}

/// Esito del confronto lingua di un file rispetto a quella preferita.
#[derive(PartialEq, Eq, Clone, Copy)]
enum LanguageMatch {
    /// Il file dichiara esplicitamente la lingua preferita.
    Preferred,
    /// Il file dichiara esplicitamente un'altra lingua (nessuna preferita).
    Other,
    /// Nessuna informazione di lingua nel nome: neutro, non si tocca.
    Unknown,
}

/// Classifica la lingua dichiarata nel nome file. `Unknown` quando il nome non
/// contiene tag lingua (in quel caso non si applica alcuna preferenza).
fn language_match(name: &str, preferred: &str) -> LanguageMatch {
    let preferred = preferred.trim().to_ascii_lowercase();
    if preferred.is_empty() {
        return LanguageMatch::Unknown;
    }
    let quality = parse_quality(name);
    let mut detected: Vec<String> = Vec::new();
    if quality.is_ita {
        detected.push("ita".to_string());
    }
    if !quality.language.trim().is_empty() {
        detected.push(quality.language.to_ascii_lowercase());
    }
    detected.extend(quality.languages.iter().map(|code| code.to_ascii_lowercase()));
    detected.retain(|code| !code.is_empty());
    if detected.is_empty() {
        return LanguageMatch::Unknown;
    }
    let matches = |code: &str| {
        code == preferred
            || (preferred == "ita" && code == "it")
            || (preferred == "eng" && code == "en")
            || (preferred == "spa" && code == "es")
            || (preferred == "deu" && code == "de")
            || (preferred == "fra" && code == "fr")
    };
    if detected.iter().any(|code| matches(code)) {
        LanguageMatch::Preferred
    } else {
        LanguageMatch::Other
    }
}

/// Rimuove le cartelle svuotate dopo uno spostamento, fermandosi prima della
/// radice della serie/film (parità con EXTTO).
fn remove_empty_parents(path: &Path, stop_at: &Path) {
    let mut current = path.parent();
    while let Some(directory) = current {
        if directory == stop_at || !directory.starts_with(stop_at) {
            break;
        }
        let empty = fs::read_dir(directory)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if !empty || fs::remove_dir(directory).is_err() {
            break;
        }
        current = directory.parent();
    }
}

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
        Ok(())
    } else {
        let Some(trash) = &cfg.trash_path else {
            anyhow::bail!("trash_path is required when cleanup_action is move");
        };
        // Reuse the cross-device-safe move: a plain `rename` fails with EXDEV
        // when the trash lives on another filesystem (typical with a NAS).
        move_to_trash(file, trash)?;
        Ok(())
    }
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
        r"(?i)^(?P<name>.+?)[ ._-]+(?:s(?P<season>\d{1,2})e|(?P<nseason>\d{1,2})x)(?P<episode>\d{1,4})(?:[ ._-]|$)",
    )?;
    let normalized = normalize_series_name(series);
    let preferred = cfg.default_language();
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
            .or_else(|| captures.name("nseason"))
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
        let old_quality = parse_quality(name);
        // Confronta con lo stesso metro del file nuovo: anche il file esistente
        // va valutato con le impostazioni (`score_res_*`, `score_source_*`, ...).
        // Prima si usava `score()` puro, quindi una soglia personalizzata (es.
        // `score_res_1080p` più bassa) faceva sembrare "inferiore" ogni nuovo
        // 1080p e lo scartava.
        let old_score = old_quality.score_with_settings(&cfg.settings);
        let new_quality = new_file
            .file_name()
            .and_then(|value| value.to_str())
            .map(parse_quality)
            .unwrap_or_default();
        let new_name = new_file
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        // A parità di risoluzione preferisci la lingua voluta: non scartare un
        // file nella lingua preferita e rimuovi quello che non lo è.
        if old_quality.resolution_rank() > 0
            && old_quality.resolution_rank() == new_quality.resolution_rank()
        {
            match (
                language_match(name, &preferred),
                language_match(new_name, &preferred),
            ) {
                (LanguageMatch::Preferred, LanguageMatch::Other) => continue,
                (LanguageMatch::Other, LanguageMatch::Preferred) => {
                    handle_duplicate(&file, cfg)?;
                    remove_empty_parents(&file, archive_path);
                    removed += 1;
                    continue;
                }
                _ => {}
            }
        }
        // Si scarta il vecchio se lo score è chiaramente inferiore, oppure se il
        // nuovo rappresenta un upgrade "forte" (risoluzione/sorgente/HDR/repack).
        if old_score + cfg.cleanup_min_score_diff >= new_score
            && !hard_upgrade(&new_quality, &old_quality, cfg.cleanup_min_score_diff)
        {
            continue;
        }
        handle_duplicate(&file, cfg)?;
        remove_empty_parents(&file, archive_path);
        removed += 1;
    }
    Ok(removed)
}

/// Duplicato chiaramente inferiore rilevato nella libreria.
#[derive(Debug, serde::Serialize)]
pub struct DuplicateCandidate {
    pub series: String,
    pub season: i64,
    pub episode: i64,
    pub path: String,
    pub resolution_rank: i32,
    pub best_rank: i32,
}

/// Cerca i duplicati a risoluzione **strettamente più bassa** per episodio
/// nella cartella di una serie. Volutamente conservativo: non segnala file con
/// la stessa risoluzione (es. versioni in lingue diverse) né file dalla
/// risoluzione non riconosciuta. Serve a ripulire le librerie ereditate dove,
/// accanto al 1080p, è rimasto il vecchio 480p/720p.
pub fn find_inferior_duplicates_in_dir(
    series: &str,
    archive_path: &Path,
    protected: &std::collections::HashSet<PathBuf>,
    preferred_language: &str,
) -> Result<Vec<DuplicateCandidate>> {
    if !local_path(archive_path) || !archive_path.is_dir() {
        return Ok(Vec::new());
    }
    let pattern = crate::utils::cached_regex(
        r"(?i)^(?P<name>.+?)[ ._-]+(?:s(?P<s>\d{1,2})e|(?P<ns>\d{1,2})x)(?P<e>\d{1,4})",
    )?;
    let normalized = normalize_series_name(series);
    // Raggruppa per (cartella, stagione, episodio): il confronto avviene solo
    // tra file nella stessa cartella, così non si toccano i file delle cartelle
    // di pack/sorgente che stanno ancora in seed.
    type GroupKey = (PathBuf, i64, i64);
    let mut groups: HashMap<GroupKey, Vec<(i32, PathBuf)>> = HashMap::new();
    for file in video_files(archive_path)? {
        // Non toccare file appartenti a torrent ancora in sessione (seed/seed
        // in corso): verrebbero invalidati.
        if protected.contains(&file) {
            continue;
        }
        let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(captures) = pattern.captures(name) else {
            continue;
        };
        let Some(file_series) = captures.name("name").map(|value| value.as_str()) else {
            continue;
        };
        if !series_names_match(&normalized, file_series) {
            continue;
        }
        let Some(season) = captures
            .name("s")
            .or_else(|| captures.name("ns"))
            .and_then(|value| value.as_str().parse::<i64>().ok())
        else {
            continue;
        };
        let Some(episode) = captures
            .name("e")
            .and_then(|value| value.as_str().parse::<i64>().ok())
        else {
            continue;
        };
        let rank = parse_quality(name).resolution_rank();
        let directory = file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| archive_path.to_path_buf());
        groups
            .entry((directory, season, episode))
            .or_default()
            .push((rank, file));
    }
    let mut candidates = Vec::new();
    for ((_, season, episode), files) in groups {
        if files.len() < 2 {
            continue;
        }
        let best = files.iter().map(|(rank, _)| *rank).max().unwrap_or(0);
        if best <= 0 {
            continue;
        }
        // Alla risoluzione migliore esiste almeno una versione nella lingua
        // preferita? Se sì, le altre versioni a pari risoluzione sono candidate
        // (es. 1080p ITA tenuto, 1080p EN da ripulire).
        let best_has_preferred = files.iter().any(|(rank, file)| {
            *rank == best
                && file
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|name| language_match(name, preferred_language) == LanguageMatch::Preferred)
                    .unwrap_or(false)
        });
        for (rank, file) in files {
            let name = file
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let lower_resolution = rank > 0 && rank < best;
            // Segnala solo chi dichiara *esplicitamente* un'altra lingua: i file
            // senza tag lingua restano intatti.
            let wrong_language = best_has_preferred
                && rank == best
                && language_match(name, preferred_language) == LanguageMatch::Other;
            if lower_resolution || wrong_language {
                candidates.push(DuplicateCandidate {
                    series: series.to_string(),
                    season,
                    episode,
                    path: file.display().to_string(),
                    resolution_rank: rank,
                    best_rank: best,
                });
            }
        }
    }
    Ok(candidates)
}

/// Sposta in trash i duplicati inferiori trovati da `find_inferior_duplicates_in_dir`.
pub fn cleanup_inferior_duplicates_in_dir(
    cfg: &Config,
    series: &str,
    archive_path: &Path,
    protected: &std::collections::HashSet<PathBuf>,
) -> Result<usize> {
    if !cfg.cleanup_upgrades {
        return Ok(0);
    }
    let preferred = cfg.default_language();
    let mut removed = 0;
    for candidate in
        find_inferior_duplicates_in_dir(series, archive_path, protected, &preferred)?
    {
        let file = PathBuf::from(&candidate.path);
        handle_duplicate(&file, cfg)?;
        remove_empty_parents(&file, archive_path);
        removed += 1;
        tracing::info!(
            series = %candidate.series,
            season = candidate.season,
            episode = candidate.episode,
            file = %candidate.path,
            rank = candidate.resolution_rank,
            best = candidate.best_rank,
            "inferior duplicate moved to trash"
        );
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
        r"(?i)^(?P<name>.+?)[ ._-]+(?:s(?P<season>\d{1,2})e|(?P<nseason>\d{1,2})x)(?P<episode>\d{1,4})(?:[ ._-]|$)",
    )?;
    let normalized = normalize_series_name(series);
    let preferred = cfg.default_language();
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
            .or_else(|| captures.name("nseason"))
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
        let old_quality = parse_quality(name);
        // Confronta con lo stesso metro del file nuovo: anche il file esistente
        // va valutato con le impostazioni (`score_res_*`, `score_source_*`, ...).
        // Prima si usava `score()` puro, quindi una soglia personalizzata (es.
        // `score_res_1080p` più bassa) faceva sembrare "inferiore" ogni nuovo
        // 1080p e lo scartava.
        let old_score = old_quality.score_with_settings(&cfg.settings);
        let new_name = new_file
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let new_quality = parse_quality(new_name);
        // A parità di risoluzione preferisci la lingua voluta: scarta il nuovo
        // solo se l'esistente è nella lingua preferita e il nuovo no.
        if old_quality.resolution_rank() > 0
            && old_quality.resolution_rank() == new_quality.resolution_rank()
        {
            match (
                language_match(name, &preferred),
                language_match(new_name, &preferred),
            ) {
                (LanguageMatch::Preferred, LanguageMatch::Other) => {
                    handle_duplicate(new_file, cfg)?;
                    remove_empty_parents(new_file, archive_path);
                    return Ok(true);
                }
                (LanguageMatch::Other, LanguageMatch::Preferred) => return Ok(false),
                _ => {}
            }
        }
        // Non scartare il nuovo se rappresenta un upgrade "forte" anche quando
        // il suo score non supera la soglia rispetto all'esistente.
        if old_score >= new_score.saturating_add(cfg.cleanup_min_score_diff)
            && !hard_upgrade(&new_quality, &old_quality, cfg.cleanup_min_score_diff)
        {
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
    let preferred = cfg.default_language();
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
        let old_quality = parse_quality(name);
        // Confronta con lo stesso metro del file nuovo: anche il file esistente
        // va valutato con le impostazioni (`score_res_*`, `score_source_*`, ...).
        // Prima si usava `score()` puro, quindi una soglia personalizzata (es.
        // `score_res_1080p` più bassa) faceva sembrare "inferiore" ogni nuovo
        // 1080p e lo scartava.
        let old_score = old_quality.score_with_settings(&cfg.settings);
        let new_name = new_file
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let new_quality = parse_quality(new_name);
        if old_quality.resolution_rank() > 0
            && old_quality.resolution_rank() == new_quality.resolution_rank()
        {
            match (
                language_match(name, &preferred),
                language_match(new_name, &preferred),
            ) {
                (LanguageMatch::Preferred, LanguageMatch::Other) => continue,
                (LanguageMatch::Other, LanguageMatch::Preferred) => {
                    handle_duplicate(&file, cfg)?;
                    remove_empty_parents(&file, archive_path);
                    removed += 1;
                    continue;
                }
                _ => {}
            }
        }
        if old_score.saturating_add(cfg.cleanup_min_score_diff) >= new_score
            && !hard_upgrade(&new_quality, &old_quality, cfg.cleanup_min_score_diff)
        {
            continue;
        }
        handle_duplicate(&file, cfg)?;
        remove_empty_parents(&file, archive_path);
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
    let preferred = cfg.default_language();
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
        let old_quality = parse_quality(name);
        // Confronta con lo stesso metro del file nuovo: anche il file esistente
        // va valutato con le impostazioni (`score_res_*`, `score_source_*`, ...).
        // Prima si usava `score()` puro, quindi una soglia personalizzata (es.
        // `score_res_1080p` più bassa) faceva sembrare "inferiore" ogni nuovo
        // 1080p e lo scartava.
        let old_score = old_quality.score_with_settings(&cfg.settings);
        let new_name = new_file
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let new_quality = parse_quality(new_name);
        if old_quality.resolution_rank() > 0
            && old_quality.resolution_rank() == new_quality.resolution_rank()
        {
            match (
                language_match(name, &preferred),
                language_match(new_name, &preferred),
            ) {
                (LanguageMatch::Preferred, LanguageMatch::Other) => {
                    handle_duplicate(new_file, cfg)?;
                    remove_empty_parents(new_file, archive_path);
                    return Ok(true);
                }
                (LanguageMatch::Other, LanguageMatch::Preferred) => return Ok(false),
                _ => {}
            }
        }
        if old_score >= new_score.saturating_add(cfg.cleanup_min_score_diff)
            && !hard_upgrade(&new_quality, &old_quality, cfg.cleanup_min_score_diff)
        {
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
    fn index_archive_keeps_best_file_per_episode() {
        let root = std::env::temp_dir().join(format!(
            "rextto-index-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = root.join("Serie");
        fs::create_dir_all(&archive).unwrap();
        fs::write(
            archive.join("Example - S01E01 - Titolo - [1080p][h264][AAC][IT].mkv"),
            b"hd",
        )
        .unwrap();
        fs::write(
            archive.join("Example - S01E01 - Titolo - [2160p][h265][DDP][HDR][IT].mkv"),
            b"4k",
        )
        .unwrap();
        fs::write(
            archive.join("Example - S01E02 - Altro - [1080p][h264][AAC][IT].mkv"),
            b"hd",
        )
        .unwrap();
        // Un'altra serie nella stessa cartella non deve entrare nell'indice.
        fs::write(archive.join("Altro - S01E01 - X - [2160p][h265][IT].mkv"), b"x").unwrap();
        let settings = std::collections::BTreeMap::new();
        let index = index_archive("Example", &archive, &settings);
        let (quality, _) = index.best_for(1, 1).expect("E01 presente");
        assert_eq!(quality.resolution, "2160p");
        assert_eq!(index.best_for(1, 2).unwrap().0.resolution, "1080p");
        assert!(index.best_for(1, 3).is_none());
        let _ = fs::remove_dir_all(root);
    }

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
    fn cleanup_recognizes_legacy_nxnn_episode_names() {
        let root = std::env::temp_dir().join(format!(
            "rextto-cleaner-nxnn-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let archive = root.join("archive");
        let trash = root.join("trash");
        fs::create_dir_all(&archive).unwrap();
        let old = archive.join("Example.1x01.720p.WEB-DL.mkv");
        fs::write(&old, b"old").unwrap();
        let kept = archive.join("Example - S01E01 - Title - [1080p][h265].mkv");
        fs::write(&kept, b"new").unwrap();
        let mut cfg = Config::default();
        cfg.cleanup_upgrades = true;
        cfg.trash_path = Some(trash.clone());
        assert_eq!(
            cleanup_old_episode(
                &cfg, "Example", 1, 1,
                parse_quality("Example.S01E01.1080p.WEB-DL.mkv").score(),
                &kept, &archive,
            ).unwrap(),
            1,
        );
        assert!(trash.join("Example.1x01.720p.WEB-DL.mkv").is_file());
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

    #[test]
    fn finds_and_trashes_only_lower_resolution_duplicates() {
        let root = std::env::temp_dir().join(format!(
            "rextto-cleaner-dup-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = root.join("archive");
        let trash = root.join("trash");
        fs::create_dir_all(&archive).unwrap();
        // Episodio 1: 1080p + vecchio 480p -> il 480p è un duplicato inferiore.
        fs::write(
            archive.join("Example - S01E01 - Pilot - [1080p][h265][AAC 5.1].mkv"),
            b"new",
        )
        .unwrap();
        fs::write(
            archive.join("Example - S01E01 - Pilot - [480p][XviD][MP3].avi"),
            b"old",
        )
        .unwrap();
        // Episodio 2: due 1080p senza tag lingua -> nessun candidato.
        fs::write(
            archive.join("Example - S02E02 - Two - [1080p][h264][EAC3].mkv"),
            b"one",
        )
        .unwrap();
        fs::write(
            archive.join("Example - S02E02 - Two - [1080p][h265][EAC3 5.1].mkv"),
            b"two",
        )
        .unwrap();
        let mut cfg = Config::default();
        cfg.cleanup_upgrades = true;
        cfg.cleanup_action = "move".into();
        cfg.trash_path = Some(trash.clone());
        let no_protected = std::collections::HashSet::new();
        let candidates =
            find_inferior_duplicates_in_dir("Example", &archive, &no_protected, "").unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].episode, 1);
        // Un file protetto (torrent in sessione) non viene mai segnalato.
        let mut protected = std::collections::HashSet::new();
        protected.insert(archive.join("Example - S01E01 - Pilot - [480p][XviD][MP3].avi"));
        assert!(
            find_inferior_duplicates_in_dir("Example", &archive, &protected, "")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            cleanup_inferior_duplicates_in_dir(&cfg, "Example", &archive, &no_protected).unwrap(),
            1
        );
        assert!(!archive
            .join("Example - S01E01 - Pilot - [480p][XviD][MP3].avi")
            .exists());
        assert!(archive
            .join("Example - S01E01 - Pilot - [1080p][h265][AAC 5.1].mkv")
            .is_file());
        // Le due versioni 1080p senza lingua restano entrambe.
        assert!(archive
            .join("Example - S02E02 - Two - [1080p][h264][EAC3].mkv")
            .is_file());
        assert!(archive
            .join("Example - S02E02 - Two - [1080p][h265][EAC3 5.1].mkv")
            .is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hard_source_upgrade_trashes_old_below_score_threshold() {
        let root = std::env::temp_dir().join(format!(
            "rextto-cleaner-hard-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = root.join("archive");
        let trash = root.join("trash");
        fs::create_dir_all(&archive).unwrap();
        // Vecchio 1080p HDTV: score ~1100. Nuovo 1080p WEB-DL: ~1250. Con
        // min_diff alto lo score non basta, ma hdtv->webdl è un upgrade "forte".
        fs::write(archive.join("Example.S01E01.1080p.HDTV.x264.mkv"), b"old").unwrap();
        let new_file = archive.join("Example - S01E01 - Title - [1080p][webdl][h264].mkv");
        fs::write(&new_file, b"new").unwrap();
        let mut cfg = Config::default();
        cfg.cleanup_upgrades = true;
        cfg.cleanup_min_score_diff = 500;
        cfg.trash_path = Some(trash.clone());
        let new_score = parse_quality("Example - S01E01 - Title - [1080p][webdl][h264].mkv").score();
        assert_eq!(
            cleanup_old_episode(&cfg, "Example", 1, 1, new_score, &new_file, &archive).unwrap(),
            1
        );
        assert!(!archive.join("Example.S01E01.1080p.HDTV.x264.mkv").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prefers_language_at_same_resolution() {
        let root = std::env::temp_dir().join(format!(
            "rextto-cleaner-lang-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = root.join("archive");
        fs::create_dir_all(&archive).unwrap();
        fs::write(
            archive.join("Example - S01E01 - Pilot - [1080p][h264][EAC3][IT+EN].mkv"),
            b"ita",
        )
        .unwrap();
        fs::write(
            archive.join("Example - S01E01 - Pilot - [1080p][h265][EAC3 5.1][EN].mkv"),
            b"eng",
        )
        .unwrap();
        let none = std::collections::HashSet::new();
        // Preferenza ITA: il file EN a pari risoluzione è candidato.
        let candidates =
            find_inferior_duplicates_in_dir("Example", &archive, &none, "ita").unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].path.contains("[EN]"));
        // Senza preferenza lingua, nessun candidato a pari risoluzione.
        assert!(
            find_inferior_duplicates_in_dir("Example", &archive, &none, "")
                .unwrap()
                .is_empty()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discards_new_release_without_preferred_language() {
        let root = std::env::temp_dir().join(format!(
            "rextto-cleaner-lang2-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = root.join("archive");
        let trash = root.join("trash");
        fs::create_dir_all(&archive).unwrap();
        // Esistente ITA 1080p; arriva un EN 1080p: va scartato il nuovo.
        fs::write(
            archive.join("Example - S01E01 - Pilot - [1080p][h264][EAC3][IT+EN].mkv"),
            b"ita",
        )
        .unwrap();
        let new_file = archive.join("Example - S01E01 - Pilot - [1080p][h265][EAC3 5.1][EN].mkv");
        fs::write(&new_file, b"eng").unwrap();
        let mut cfg = Config::default();
        cfg.cleanup_upgrades = true;
        cfg.trash_path = Some(trash.clone());
        let new_score = parse_quality("Example - S01E01 - Pilot - [1080p][h265][EAC3 5.1][EN].mkv").score();
        assert!(discard_if_inferior(
            &cfg, "Example", 1, 1, new_score, &new_file, &archive
        )
        .unwrap());
        assert!(!new_file.exists(), "il nuovo EN va nel trash");
        assert!(archive
            .join("Example - S01E01 - Pilot - [1080p][h264][EAC3][IT+EN].mkv")
            .is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lowered_resolution_setting_does_not_make_equal_new_release_inferior() {
        let root = std::env::temp_dir().join(format!(
            "rextto-cleaner-res-setting-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = root.join("archive");
        let trash = root.join("trash");
        fs::create_dir_all(&archive).unwrap();
        // Esistente e nuovo sono entrambi 1080p IT+EN: il nuovo non è inferiore.
        fs::write(
            archive.join("Example - S01E01 - Pilot - [1080p][h265][AAC][IT+EN].mkv"),
            b"old",
        )
        .unwrap();
        let new_file = archive.join("Example - S01E01 - Pilot - [WEB-DL][1080p][h264][AAC][IT+EN].mkv");
        fs::write(&new_file, b"new").unwrap();
        let mut cfg = Config::default();
        cfg.cleanup_upgrades = true;
        cfg.cleanup_min_score_diff = 50;
        cfg.trash_path = Some(trash.clone());
        // Impostazione utente che abbassa il peso del 1080p: se il confronto
        // mescola `score()` e `score_with_settings()` il nuovo 1080p sembra
        // "inferiore" e viene scartato per errore.
        cfg.settings
            .insert("score_res_1080p".into(), "500".into());
        let new_score = parse_quality("Example - S01E01 - Pilot - [WEB-DL][1080p][h264][AAC][IT+EN].mkv")
            .score_with_settings(&cfg.settings);
        assert!(!discard_if_inferior(
            &cfg, "Example", 1, 1, new_score, &new_file, &archive
        )
        .unwrap());
        assert!(new_file.is_file(), "il nuovo file non va scartato");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn removes_emptied_subdirectories_after_cleanup() {
        let root = std::env::temp_dir().join(format!(
            "rextto-cleaner-emptydir-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = root.join("archive");
        let trash = root.join("trash");
        let season = archive.join("Season 1");
        fs::create_dir_all(&season).unwrap();
        fs::write(
            season.join("Example - S01E01 - Pilot - [480p][XviD][MP3].avi"),
            b"old",
        )
        .unwrap();
        let new_file = archive.join("Example - S01E01 - Pilot - [1080p][h265][AAC].mkv");
        fs::write(&new_file, b"new").unwrap();
        let mut cfg = Config::default();
        cfg.cleanup_upgrades = true;
        cfg.trash_path = Some(trash.clone());
        let new_score = parse_quality("Example - S01E01 - Pilot - [1080p][h265][AAC].mkv").score();
        assert_eq!(
            cleanup_old_episode(&cfg, "Example", 1, 1, new_score, &new_file, &archive).unwrap(),
            1
        );
        assert!(!season.exists(), "la sottocartella svuotata va rimossa");
        assert!(archive.is_dir(), "la radice della serie resta");
        let _ = fs::remove_dir_all(root);
    }
}
