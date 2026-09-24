use crate::{
    config::Config,
    models::{Release, TorrentEvent},
    tmdb::TmdbClient,
};
use anyhow::{bail, Result};
use regex::Regex;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn tag_matches_release(tag: &str, release: &Release) -> bool {
    let tag = tag.trim().to_ascii_lowercase();
    if tag.is_empty() {
        return false;
    }
    if tag == release.source.to_ascii_lowercase() || tag == release.kind.to_ascii_lowercase() {
        return true;
    }
    let aliases: &[&str] = match release.kind.as_str() {
        "movie" => &["film", "movies", "movie"],
        "series" => &["serie", "series", "serie tv", "tv", "show", "tv show"],
        _ => &[],
    };
    aliases.contains(&tag.as_str())
}

pub fn destination_for(release: &Release, cfg: &Config) -> Option<PathBuf> {
    if let Some(raw) = cfg.settings.get("tag_dir_rules") {
        if let Ok(rules) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
            if let Some(final_dir) = rules
                .iter()
                .find(|rule| {
                    let tag = rule
                        .get("tag")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    tag_matches_release(tag, release)
                })
                .and_then(|rule| rule.get("final_dir").and_then(serde_json::Value::as_str))
                .filter(|path| !path.trim().is_empty())
            {
                return Some(PathBuf::from(final_dir));
            }
        }
    }
    if release.kind == "series" {
        if let Some(series) = release
            .series
            .as_deref()
            .and_then(|name| cfg.find_series_match(name, release.season))
        {
            // `archive_path` configurato oppure cartella auto-rilevata sotto
            // `archive_root` (una cartella per serie, come il legacy).
            if let Some(destination) = cfg.resolve_archive_path(series) {
                if series.season_subfolders {
                    if let Some(season) = release.season {
                        return Some(destination.join(format!("Stagione {season:02}")));
                    }
                }
                return Some(destination);
            }
        }
    }
    cfg.archive_root
        .clone()
        .or_else(|| Some(cfg.libtorrent_dir.clone()))
}

/// Returns the per-tag temporary download directory for a release, if configured.
pub fn download_dir_for(release: &Release, cfg: &Config) -> Option<PathBuf> {
    let raw = cfg.settings.get("tag_dir_rules")?;
    let rules = serde_json::from_str::<Vec<serde_json::Value>>(raw).ok()?;
    rules
        .iter()
        .find(|rule| {
            let tag = rule
                .get("tag")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            tag_matches_release(tag, release)
        })
        .and_then(|rule| rule.get("temp_dir").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

pub fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

pub fn size_of_path(path: &Path) -> Result<i64> {
    if !path.exists() {
        bail!("completed torrent path does not exist: {}", path.display());
    }
    let mut total = 0_i64;
    if path.is_file() {
        return Ok(path.metadata()?.len().min(i64::MAX as u64) as i64);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        // Use the entry type without following symlinks: a torrent containing a
        // symlink back into an ancestor would otherwise recurse forever.
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            total = total.saturating_add(size_of_path(&child)?);
        } else if file_type.is_file() {
            total = total.saturating_add(child.metadata()?.len().min(i64::MAX as u64) as i64);
        }
    }
    Ok(total)
}

/// Path of the completed download inside the torrent's `save_path`.
///
/// `event.name` comes from libtorrent and is not trusted: a crafted or
/// absolute name must never escape `save_path`. We therefore keep only the
/// final component, and — crucially — never fall back to the shared
/// `save_path` root when the named entry is missing. The fallback used to make
/// callers (trash/remove/rename) act on the whole download directory, deleting
/// unrelated, still-active torrents' data.
pub fn completion_path(event: &TorrentEvent) -> PathBuf {
    let root = PathBuf::from(&event.save_path);
    let name = Path::new(&event.name)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty() && name != "." && name != "..");
    match name {
        Some(name) => root.join(name),
        // No usable name: return a path that cannot collide with real data so
        // callers see a missing path instead of the shared root.
        None => root.join(format!(".rextto-missing-{}", event.hash)),
    }
}

pub fn validate_destination(destination: &Path) -> Result<()> {
    if destination.as_os_str().is_empty() {
        bail!("empty post-processing destination");
    }
    if destination.to_string_lossy().contains('\0') {
        bail!("invalid post-processing destination");
    }
    fs::create_dir_all(destination)?;
    Ok(())
}

pub fn validate_destination_from(source: &Path, destination: &Path) -> Result<()> {
    validate_destination(destination)?;
    let source_root = fs::canonicalize(source)?;
    let destination_root = fs::canonicalize(destination)?;
    if destination_root == source_root || destination_root.starts_with(&source_root) {
        bail!(
            "refusing to write post-processing output inside its source tree: {}",
            destination.display()
        );
    }
    Ok(())
}

fn copy_files(files: &[PathBuf], source: &Path, destination: &Path) -> Result<Vec<PathBuf>> {
    validate_destination_from(source, destination)?;
    let mut copied = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("season pack file has no name"))?;
        let target = destination.join(name);
        if target.exists() {
            // Il file è già in libreria: NON sovrascrivere e non fallire. Un
            // errore qui bloccava il completamento del pack, lasciando gli
            // episodi non marcati → il pack veniva riapprovato/riscaricato ad
            // ogni ciclo. Si continua con l'esistente.
            copied.push(target);
            continue;
        }
        copy_file_atomically(file, &target)?;
        copied.push(target);
    }
    if copied.is_empty() {
        bail!("season pack contains no video files: {}", source.display());
    }
    Ok(copied)
}

pub fn copy_pack_files(source: &Path, destination: &Path) -> Result<Vec<PathBuf>> {
    let files = video_files(source)?;
    copy_files(&files, source, destination)
}

/// Returns only pack files whose season and episode range agree with the
/// release metadata. This prevents a mislabeled pack (for example S05 files
/// advertised as S06) from being copied into the NAS before it is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackSourceFile {
    pub path: PathBuf,
    pub season: i64,
    pub episode: i64,
}

/// Resolves the episode identity of every file that can safely be imported from
/// a pack. Standard `SxxEyy`/`NxNN` names are preferred. As a compatibility
/// fallback, a partial pack may contain files named only `01.mkv` or
/// `Episode 01.mkv`: those are accepted only when their explicit numbers are
/// all members of the release range. A complete, unlabelled pack remains
/// deliberately rejected because there is no safe episode mapping.
pub fn matching_pack_files(source: &Path, release: &Release) -> Result<Vec<PackSourceFile>> {
    let episode_pattern =
        crate::utils::cached_regex(r"(?i)(?:s(?P<season>\d{1,2})e|(?P<nseason>\d{1,2})x)(?P<episode>\d{1,4})")?;
    let files = video_files(source)?;
    let mut matched = files
        .iter()
        .filter_map(|file| {
            let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
                return None;
            };
            let Some(capture) = episode_pattern.captures(name) else {
                return None;
            };
            let season = capture
                .name("season")
                .or_else(|| capture.name("nseason"))
                .and_then(|value| value.as_str().parse::<i64>().ok());
            let episode = capture
                .name("episode")
                .and_then(|value| value.as_str().parse::<i64>().ok());
            let episode = episode.filter(|episode| {
                    release.episode_range.is_empty()
                        || release.episode_range.contains(&0)
                        || release.episode_range.contains(episode)
                });
            match (season, episode) {
                (Some(season), Some(episode)) if Some(season) == release.season => {
                    Some(PackSourceFile { path: file.clone(), season, episode })
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    let expected = release
        .episode_range
        .iter()
        .copied()
        .filter(|episode| *episode > 0)
        .collect::<std::collections::BTreeSet<_>>();
    let bare_episode = crate::utils::cached_regex(r"(?i)^(?:e(?:pisode)?[ ._-]?)?0*(\d{1,4})$")?;
    // A complete pack has no trustworthy explicit episode list. In that case,
    // import only files carrying their own season/episode identity.
    if expected.is_empty() || release.season.is_none() {
        return Ok(matched);
    }
    // A partial pack is complete only when every declared episode is accounted
    // for. Combine explicit SxxEyy/NxNN names with the narrowly accepted bare
    // numeric fallback; never silently import half a declared range.
    let mut found = matched
        .iter()
        .map(|file| file.episode)
        .collect::<std::collections::BTreeSet<_>>();
    for file in files {
        if matched.iter().any(|item| item.path == file) {
            continue;
        }
        let Some(stem) = file.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(capture) = bare_episode.captures(stem) else {
            continue;
        };
        let Some(episode) = capture.get(1).and_then(|value| value.as_str().parse::<i64>().ok()) else {
            return Ok(Vec::new());
        };
        if !expected.contains(&episode) || !found.insert(episode) {
            return Ok(Vec::new());
        }
        matched.push(PackSourceFile {
            path: file,
            season: release.season.unwrap_or_default(),
            episode,
        });
    }
    matched.sort_by_key(|file| file.episode);
    matched.dedup_by_key(|file| file.episode);
    Ok((found == expected).then_some(matched).unwrap_or_default())
}

pub fn copy_matching_pack_files(
    source: &Path,
    destination: &Path,
    release: &Release,
) -> Result<Vec<PathBuf>> {
    let files = matching_pack_files(source, release)?;
    if files.is_empty() {
        bail!(
            "season pack files do not match declared season: {}",
            source.display()
        );
    }
    copy_files(
        &files.into_iter().map(|file| file.path).collect::<Vec<_>>(),
        source,
        destination,
    )
}

#[derive(Debug, Clone)]
pub struct PackFileResult {
    pub episode: i64,
    pub path: PathBuf,
    pub size_bytes: i64,
    pub quality_score: i64,
    pub discarded: bool,
}

/// Best-quality video file in `dir` that matches the given season/episode.
///
/// Used to recover the real path of a pack episode whose copy-time name no
/// longer exists because a rename moved it, and to decide whether the
/// destination already holds an equivalent or better file.
pub fn best_episode_file(dir: &Path, season: i64, episode: i64) -> Option<PathBuf> {
    let mut best: Option<(i64, PathBuf)> = None;
    for file in video_files(dir).ok()? {
        let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !filename_matches_episode(name, season, episode) {
            continue;
        }
        let score = crate::parser::parse_quality(name).score();
        match &best {
            Some((current, _)) if *current >= score => {}
            _ => best = Some((score, file)),
        }
    }
    best.map(|(_, file)| file)
}

/// Copies one season-pack episode into the destination through a temporary
/// `.rextto-part` file.
///
/// A partial copy must never be visible as a real episode: the scanner only
/// looks at video extensions, so the expensive transfer happens under the
/// ignored suffix and only the finished file is atomically renamed to its
/// source name. If the destination already holds the same episode at an equal
/// or better quality (a previous import that was then renamed), the existing
/// file is returned instead of copying again.
pub fn stage_pack_file(
    file: &PackSourceFile,
    source: &Path,
    destination: &Path,
    cfg: &Config,
    release_quality_score: i64,
) -> Result<Option<PathBuf>> {
    let Some(name) = file.path.file_name().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    let season = file.season;
    let episode = file.episode;
    if let Some(existing) = best_episode_file(destination, season, episode) {
        let existing_score = existing
            .file_name()
            .and_then(|value| value.to_str())
            .map(crate::parser::parse_quality)
            .map(|quality| quality.score_with_settings(&cfg.settings))
            .unwrap_or(0);
        let incoming_score = crate::parser::parse_quality(name)
            .score_with_settings(&cfg.settings);
        if existing_score >= incoming_score.max(release_quality_score) {
            return Ok(Some(existing));
        }
    }
    // Files named only `01.mkv` are supported for a validated partial pack,
    // but cannot be written directly into a series root: S01E01 and S02E01
    // would otherwise collide before their final rename. Give these opaque
    // names a unique, temporary episode identity.
    let has_episode_identity = crate::utils::cached_regex(
        r"(?i)(?:s\d{1,2}e|\d{1,2}x)\d{1,4}",
    )?
    .is_match(name);
    let target_name = if has_episode_identity {
        name.to_owned()
    } else {
        format!(".rextto-pack-S{season:02}E{episode:02}-{name}")
    };
    let target = destination.join(target_name);
    if target.exists() {
        return Ok(Some(target));
    }
    validate_destination_from(source, destination)?;
    let temp = destination.join(format!("{name}.rextto-part"));
    let _ = fs::remove_file(&temp);
    let mut input = fs::File::open(&file.path)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let copied_bytes = std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    drop(output);
    let source_bytes = file.path.metadata()?.len();
    if copied_bytes != source_bytes || temp.metadata()?.len() != source_bytes {
        let _ = fs::remove_file(&temp);
        bail!(
            "season pack copy size mismatch for {}: source={} copied={} target={}",
            file.path.display(),
            source_bytes,
            copied_bytes,
            temp.metadata().map(|value| value.len()).unwrap_or(0),
        );
    }
    fs::rename(&temp, &target)?;
    Ok(Some(target))
}

pub async fn process_pack_files(
    files: &[(PathBuf, PackSourceFile)],
    release: &Release,
    cfg: &Config,
    tmdb: &TmdbClient,
) -> Result<Vec<PackFileResult>> {
    let mut results = Vec::new();
    for (file, pack_file) in files {
        let season = pack_file.season;
        let episode = pack_file.episode;
        let mut episode_release = release.clone();
        episode_release.is_pack = false;
        episode_release.episode = Some(episode);
        episode_release.episode_range = vec![episode];
        // A repair/rename pass may have moved the file between the copy and
        // now: never trust a path that no longer exists, recover the current
        // file for the episode from the destination directory instead.
        let actual = if file.exists() {
            file.clone()
        } else {
            let directory = file.parent().unwrap_or_else(|| Path::new("."));
            best_episode_file(directory, season, episode).unwrap_or_else(|| file.clone())
        };
        let renamed = rename_episode(&actual, &episode_release, cfg, tmdb).await?;
        let final_path = renamed.unwrap_or(actual);
        let archive = final_path.parent().unwrap_or_else(|| Path::new("."));
        let score = episode_release.quality.score_with_settings(&cfg.settings);
        let discarded = crate::cleaner::discard_if_inferior(
            cfg,
            release.series.as_deref().unwrap_or_default(),
            season,
            episode,
            score,
            &final_path,
            archive,
        )?;
        if !discarded {
            let _ = crate::cleaner::cleanup_old_episode(
                cfg,
                release.series.as_deref().unwrap_or_default(),
                season,
                episode,
                score,
                &final_path,
                archive,
            )?;
        }
        // Score of the file actually kept, so an upgraded 2160p episode is not
        // recorded with the stale 1080p quality of its old title.
        let quality_score = final_path
            .file_name()
            .and_then(|value| value.to_str())
            .map(crate::parser::parse_quality)
            .map(|quality| quality.score_with_settings(&cfg.settings))
            .filter(|value| *value > 0)
            .unwrap_or(score);
        results.push(PackFileResult {
            episode,
            size_bytes: size_of_path(&final_path).unwrap_or(0),
            path: final_path,
            quality_score,
            discarded,
        });
    }
    Ok(results)
}

pub async fn rename_episode(
    path: &Path,
    release: &Release,
    cfg: &Config,
    tmdb: &TmdbClient,
) -> Result<Option<PathBuf>> {
    let Some((source, target)) = episode_target(path, release, cfg, tmdb).await? else {
        return Ok(None);
    };
    if same_path(&source, &target) {
        // Il video è già corretto: ripulisci comunque i sidecar spuri/doppi.
        apply_sidecars(&source, &target, cfg);
        return Ok(Some(target));
    }
    if crate::cleaner::resolve_existing_target(
        &source,
        &target,
        release.quality.score_with_settings(&cfg.settings),
        cfg,
    )? {
        apply_sidecars(&source, &target, cfg);
        return Ok(Some(target));
    }
    move_across_devices(&source, &target)?;
    // Rinomina anche i file associati (thumbnail, sottotitoli, ...) e metti i
    // doppioni/spuri nel trash.
    apply_sidecars(&source, &target, cfg);
    Ok(Some(target))
}

pub async fn preview_episode_rename(
    path: &Path,
    release: &Release,
    cfg: &Config,
    tmdb: &TmdbClient,
) -> Result<Option<PathBuf>> {
    // Segnala solo i file che cambierebbero davvero nome.
    match episode_target(path, release, cfg, tmdb).await? {
        Some((source, target)) if !same_path(&source, &target) => Ok(Some(target)),
        _ => Ok(None),
    }
}

/// Vero se il nome file contiene la stagione/episodio indicati (S01E02, 1x02, …).
fn filename_matches_episode(name: &str, season: i64, episode: i64) -> bool {
    let lower = name.to_lowercase();
    lower.contains(&format!("s{season:02}e{episode:02}"))
        || lower.contains(&format!("s{season}e{episode}"))
        || lower.contains(&format!("{season}x{episode:02}"))
        || lower.contains(&format!("{season}x{episode}"))
}

async fn episode_target(
    path: &Path,
    release: &Release,
    cfg: &Config,
    tmdb: &TmdbClient,
) -> Result<Option<(PathBuf, PathBuf)>> {
    if !cfg.rename_episodes || release.is_pack || release.kind != "series" {
        return Ok(None);
    }
    let Some(season) = release.season else {
        return Ok(None);
    };
    let Some(episode) = release.episode else {
        return Ok(None);
    };
    let Some(series_name) = release.series.as_deref() else {
        return Ok(None);
    };
    // Non usare find_series_match: rifiuterebbe le puntate di stagioni non
    // monitorate (es. seasons="5+") impedendo di rinominare i file presenti.
    let Some(series) = cfg.find_series_by_name(series_name) else {
        return Ok(None);
    };
    let tmdb_id = if series.tmdb_id.trim().is_empty() {
        tmdb.resolve_series_id(&series.name)
            .await?
            .unwrap_or_default()
    } else {
        series.tmdb_id.clone()
    };
    let title = tmdb
        .episode_title(&tmdb_id, season, episode)
        .await?
        .unwrap_or_else(|| format!("Episodio {episode}"));
    let files = video_files(path)?;
    let source = if files.len() == 1 {
        files[0].clone()
    } else {
        // archive_path può essere la cartella della serie: scegli il file che
        // corrisponde alla stagione/episodio richiesti.
        let matches: Vec<PathBuf> = files
            .into_iter()
            .filter(|file| {
                file.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| filename_matches_episode(name, season, episode))
                    .unwrap_or(false)
            })
            .collect();
        if matches.len() != 1 {
            return Ok(None);
        }
        matches[0].clone()
    };
    let source = &source;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mkv");
    let title = safe_component(&title);
    let media = read_media_tags(source);
    let resolution = media
        .resolution
        .as_deref()
        .unwrap_or(&release.quality.resolution);
    let codec = media
        .video_codec
        .as_deref()
        .unwrap_or(&release.quality.codec);
    let audio = media
        .audio_codec
        .as_deref()
        .unwrap_or(&release.quality.audio);
    let hdr = media.hdr.as_deref().unwrap_or(&release.quality.hdr);
    let default_language = cfg.default_language();
    let language = media
        .languages
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            (!release.quality.language.trim().is_empty())
                .then_some(release.quality.language.as_str())
        })
        .unwrap_or(&default_language);
    // legacy `{Audio}` (and `{AudioCodec}`) join codec and channels: "AC3 5.1".
    let audio_full = [Some(audio), media.channels.as_deref()]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let series_name = sanitize_invalid(&series.name);
    let source_tag = source_label(&release.quality.source);
    let target_stem = match cfg.rename_format.as_str() {
        "standard" => format!(
            "{} - S{:02}E{:02} - {} [{}][{}]",
            series_name, season, episode, title, resolution, codec
        ),
        "full" | "completo" => format!(
            "{} - S{:02}E{:02} - {} [{}][{}][{}][{}][{}]",
            series_name, season, episode, title, resolution, audio, hdr, codec, language
        ),
        "custom" => {
            // Un placeholder senza valore (vuoto o "unknown") diventa vuoto, così
            // il blocco che lo avvolge — es. `[{Source}]` — viene rimosso del
            // tutto da `cleanup_filename` invece di lasciare `[]` o `[unknown]`.
            let token = |value: &str| -> String {
                let value = value.trim();
                if value.is_empty() || value.eq_ignore_ascii_case("unknown") {
                    String::new()
                } else {
                    value.to_string()
                }
            };
            let language_token = token(language);
            let source_token = token(&source_tag);
            cfg.rename_template
                .replace("{Serie}", &series_name)
                .replace("{Stagione}", &format!("S{:02}", season))
                .replace("{Episodio}", &format!("E{:02}", episode))
                .replace("{Titolo}", &title)
                .replace("{Source}", &source_token)
                .replace("{Sorgente}", &source_token)
                .replace("{Gruppo}", &token(&release.quality.group))
                .replace("{Risoluzione}", &token(resolution))
                .replace("{VideoCodec}", &token(codec))
                .replace("{Audio}", &token(&audio_full))
                .replace("{AudioCodec}", &token(&audio_full))
                .replace("{Canali}", &token(media.channels.as_deref().unwrap_or("")))
                .replace("{HDR}", &token(hdr))
                .replace("{Lingue}", &language_token)
        }
        _ => format!("{} - S{:02}E{:02} - {}", series_name, season, episode, title),
    };
    let target_stem = cleanup_filename(&sanitize_invalid(&target_stem));
    let target_name = format!("{}.{}", target_stem, extension);
    let target = source.parent().unwrap_or(path).join(target_name);
    Ok(Some((source.clone(), target)))
}

pub async fn rename_movie(
    path: &Path,
    release: &Release,
    cfg: &Config,
    tmdb: &TmdbClient,
) -> Result<Option<PathBuf>> {
    if !cfg.rename_episodes || release.kind != "movie" {
        return Ok(None);
    }
    let configured = release.title.clone();
    let movie = cfg.find_movie_match_manual(&release.title, release.year);
    let configured_name = movie
        .map(|value| value.name.clone())
        .unwrap_or_else(|| configured.replace('.', " "));
    let tmdb_item = tmdb.search_movie(&configured_name, release.year).await?;
    let official_title = tmdb_item
        .as_ref()
        .and_then(|value| value.title.clone())
        .unwrap_or(configured_name);
    let official_year = release.year;
    let files = video_files(path)?;
    if files.len() != 1 {
        return Ok(None);
    }
    let source = &files[0];
    if source
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.to_ascii_lowercase().contains("sample"))
    {
        return Ok(None);
    }
    let media = read_media_tags(source);
    let resolution = media
        .resolution
        .as_deref()
        .unwrap_or(&release.quality.resolution);
    let codec = media
        .video_codec
        .as_deref()
        .unwrap_or(&release.quality.codec);
    let audio = media
        .audio_codec
        .as_deref()
        .unwrap_or(&release.quality.audio);
    let hdr = media.hdr.as_deref().unwrap_or(&release.quality.hdr);
    let audio_full = [Some(audio), media.channels.as_deref()]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let official_title = sanitize_invalid(&official_title);
    let year = official_year
        .map(|value| value.to_string())
        .unwrap_or_default();
    // legacy does NOT apply the series template to movies: the file name is
    // always `Title (Year)` plus movie-appropriate tags. Applying the series
    // template leaked literal `{Serie}{Stagione}{Episodio}` into the name.
    let year_tag = if year.is_empty() {
        String::new()
    } else {
        format!(" ({year})")
    };
    let stem = match cfg.rename_format.as_str() {
        "standard" => format!(
            "{official_title}{year_tag} [{}][{}]",
            resolution, codec
        ),
        "full" | "completo" => format!(
            "{official_title}{year_tag} [{}][{}][{}][{}]",
            resolution, audio_full, hdr, codec
        ),
        // legacy's custom movie rename falls back to title + resolution.
        "custom" => format!("{official_title}{year_tag} [{}]", resolution),
        _ => format!("{official_title}{year_tag}"),
    };
    let stem = cleanup_filename(&sanitize_invalid(&stem));
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mkv");
    let target = source
        .parent()
        .unwrap_or(path)
        .join(format!("{}.{}", stem, extension));
    if same_path(source, &target) {
        return Ok(Some(target));
    }
    if crate::cleaner::resolve_existing_target(
        source,
        &target,
        release.quality.score_with_settings(&cfg.settings),
        cfg,
    )? {
        return Ok(Some(target));
    }
    move_across_devices(source, &target)?;
    Ok(Some(target))
}

/// Ripristina il token sorgente in un nome file che l'ha perso, inserendo
/// `[WEB-DL]`/`[HDTV]`… subito prima del primo tag di risoluzione (che nel
/// template segue sempre la sorgente). Ritorna `None` se la sorgente non è
/// nota, è già presente o non c'è un punto sicuro dove inserirla.
pub fn restore_source_token(name: &str, source: &str) -> Option<String> {
    let label = source_label(source);
    let label = label.trim();
    if label.is_empty() || label.eq_ignore_ascii_case("unknown") {
        return None;
    }
    if name.to_ascii_lowercase().contains(&label.to_ascii_lowercase()) {
        return None;
    }
    let marker =
        crate::utils::cached_regex(r"(?i)\[(?:2160p|1080p|720p|576p|480p|360p)\]").ok()?;
    let found = marker.find(name)?;
    let mut output = String::with_capacity(name.len() + label.len() + 2);
    output.push_str(&name[..found.start()]);
    output.push('[');
    output.push_str(label);
    output.push(']');
    output.push_str(&name[found.start()..]);
    Some(output)
}

fn move_across_devices(source: &Path, target: &Path) -> Result<()> {
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
            copy_file_atomically(source, target)?;
            fs::remove_file(source)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// Copy a file through a hidden sibling and publish it with one rename. This
/// keeps a partial cross-filesystem copy from being mistaken for an archived
/// media file after a crash or an interrupted process.
fn copy_file_atomically(source: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("target has no parent: {}", target.display()))?;
    fs::create_dir_all(parent)?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let temporary = parent.join(format!(".{name}.rextto-copy-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut input = fs::File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let copied_bytes = std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        let source_bytes = source.metadata()?.len();
        let target_bytes = temporary.metadata()?.len();
        if copied_bytes != source_bytes || target_bytes != source_bytes {
            bail!(
                "atomic copy size mismatch for {}: source={} copied={} target={}",
                source.display(),
                source_bytes,
                copied_bytes,
                target_bytes,
            );
        }
        fs::rename(&temporary, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Controllo economico del pattern del nome (senza TMDB né MediaInfo): vero se
/// il file segue già il formato di rinomina configurato (prefisso
/// `Serie - SxxEyy - `). Evita di analizzare 100 file quando solo 1 va
/// rinominato, come faceva legacy.
pub fn episode_name_conforms(path: &Path, release: &Release, cfg: &Config) -> bool {
    if !cfg.rename_episodes || release.kind != "series" {
        return false;
    }
    let (Some(season), Some(episode), Some(series_name)) =
        (release.season, release.episode, release.series.as_deref())
    else {
        return false;
    };
    let Some(series) = cfg.find_series_by_name(series_name) else {
        return false;
    };
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase();
    // Use the same sanitised series name as `episode_target`, otherwise a name
    // with invalid characters (e.g. a colon in "Star Trek: ...") never matches
    // the real target and triggers a pointless no-op rename.
    let series_name = sanitize_invalid(&series.name);
    let prefix = match cfg.rename_format.as_str() {
        "custom" => {
            let rendered = cfg
                .rename_template
                .replace("{Serie}", &series_name)
                .replace("{Stagione}", &format!("S{:02}", season))
                .replace("{Episodio}", &format!("E{:02}", episode));
            match rendered.find('{') {
                Some(index) => rendered[..index].to_string(),
                None => rendered,
            }
        }
        _ => format!("{} - S{:02}E{:02} - ", series_name, season, episode),
    };
    // Nomi con gruppi vuoti (es. `[]` o `()`) o placeholder letterali sono
    // artefatti di un template applicato solo in parte: vanno rinominati per
    // ripulirli. Il solo prefisso non basta, ad esempio per `[{Source}]`.
    if crate::utils::cached_regex(r"\[\s*\]|\(\s*\)")
        .map(|regex| regex.is_match(&stem))
        .unwrap_or(false)
        || stem.contains('{')
        || stem.contains('}')
    {
        return false;
    }
    let prefix = prefix.trim().to_lowercase();
    !prefix.is_empty() && stem.starts_with(&prefix)
}

/// Rinomina i file associati al video (thumbnail, sottotitoli, nfo, ...) perché
/// seguano il nuovo nome base. I doppioni (target già esistente) e le copie
/// "(copia N)/(copy N)" vengono spostati nel trash (o eliminati) secondo la
/// configurazione (`cleanup_action`).
pub fn rename_sidecars(
    source: &Path,
    target: &Path,
    cfg: &Config,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let Some(source_stem) = source.file_stem().and_then(|value| value.to_str()) else {
        return Ok(Vec::new());
    };
    let Some(target_stem) = target.file_stem().and_then(|value| value.to_str()) else {
        return Ok(Vec::new());
    };
    if source_stem.is_empty() || target_stem.is_empty() {
        return Ok(Vec::new());
    }
    let target_stem_lower = target_stem.to_lowercase();
    let target_se = parse_se(target_stem);
    let video_exts = ["mkv", "mp4", "avi", "m4v", "mov", "ts"];
    let sidecar_exts = [
        "jpg", "jpeg", "png", "webp", "srt", "sub", "ass", "ssa", "vtt", "idx", "sup", "smi",
        "nfo", "txt",
    ];
    let existing = sidecar_names(parent);
    let mut moved = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            continue;
        }
        let path = entry.path();
        if path == source || !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if video_exts.contains(&extension.as_str()) || !sidecar_exts.contains(&extension.as_str()) {
            continue;
        }
        let lower = name.to_lowercase();
        if lower.starts_with(&target_stem_lower) {
            continue; // già corretto
        }
        let is_duplicate = lower.contains("(copia") || lower.contains("(copy");
        // 1) sidecar che deriva dal nome corrente del video.
        if let Some(remainder) = name.strip_prefix(source_stem) {
            if is_duplicate {
                trash_or_remove(&path, cfg)?;
                moved.push((path, PathBuf::new()));
                continue;
            }
            let new_path = parent.join(format!("{target_stem}{remainder}"));
            if new_path == path {
                continue;
            }
            if new_path.exists() {
                trash_or_remove(&path, cfg)?;
            } else {
                move_across_devices(&path, &new_path)?;
            }
            moved.push((path, new_path));
            continue;
        }
        // 2) sidecar "spurio" di un nome precedente: stesso episodio (SxxEyy/NxNN).
        if target_se.is_none() || parse_se(name) != target_se {
            continue;
        }
        if is_duplicate {
            trash_or_remove(&path, cfg)?;
            moved.push((path, PathBuf::new()));
            continue;
        }
        // Se esiste già il sidecar corretto con la stessa estensione, butta lo spurio.
        let has_counterpart = existing.iter().any(|candidate| {
            candidate != name
                && candidate.to_lowercase().starts_with(&target_stem_lower)
                && candidate.to_lowercase().ends_with(&format!(".{extension}"))
        });
        if has_counterpart {
            trash_or_remove(&path, cfg)?;
            moved.push((path, PathBuf::new()));
            continue;
        }
        let expected = if lower.contains("-thumb") {
            format!("{target_stem}-thumb.{extension}")
        } else {
            format!("{target_stem}.{extension}")
        };
        let new_path = parent.join(expected);
        if new_path != path {
            if new_path.exists() {
                trash_or_remove(&path, cfg)?;
            } else {
                move_across_devices(&path, &new_path)?;
            }
            moved.push((path, new_path));
        }
    }
    Ok(moved)
}

fn sidecar_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// (stagione, episodio) da un nome file, sia `S01E02` sia `1x02`.
fn parse_se(name: &str) -> Option<(i64, i64)> {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        crate::utils::cached_regex(r"(?i)(?:s(?P<s>\d{1,2})e|(?P<ns>\d{1,2})x)(?P<e>\d{1,4})")
            .expect("season/episode pattern")
    });
    let captures = pattern.captures(name)?;
    let season = captures
        .name("s")
        .or_else(|| captures.name("ns"))?
        .as_str()
        .parse()
        .ok()?;
    let episode = captures.name("e")?.as_str().parse().ok()?;
    Some((season, episode))
}

/// Applica la rinomina dei sidecar e logga l'esito.
pub fn apply_sidecars(source: &Path, target: &Path, cfg: &Config) {
    match rename_sidecars(source, target, cfg) {
        Ok(sidecars) => {
            for (from, to) in &sidecars {
                if to.as_os_str().is_empty() {
                    tracing::debug!(from = %from.display(), "rename sidecar: duplicate moved to trash");
                } else {
                    tracing::debug!(from = %from.display(), to = %to.display(), "rename sidecar");
                }
            }
        }
        Err(error) => tracing::warn!(%error, "rename sidecar failed"),
    }
}

/// Moves sidecars belonging to a video that was discarded as a duplicate.
/// The video cleanup path cannot import this module, so the rename workflow
/// calls this explicitly after `discard_if_inferior`.
pub fn discard_sidecars(source: &Path, cfg: &Config) -> Result<usize> {
    let Some(stem) = source.file_stem().and_then(|value| value.to_str()) else {
        return Ok(0);
    };
    let Some(parent) = source.parent() else {
        return Ok(0);
    };
    let sidecar_exts = [
        "jpg", "jpeg", "png", "webp", "srt", "sub", "ass", "ssa", "vtt", "idx", "sup", "smi",
        "nfo", "xml",
    ];
    let mut moved = 0;
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let path = entry.path();
        if path == source || !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with(stem)
            || !sidecar_exts.contains(&extension.to_ascii_lowercase().as_str())
        {
            continue;
        }
        trash_or_remove(&path, cfg)?;
        moved += 1;
    }
    Ok(moved)
}

fn trash_or_remove(path: &Path, cfg: &Config) -> Result<()> {
    if cfg.cleanup_action == "delete" {
        return Ok(fs::remove_file(path)?);
    }
    match cfg.trash_path.as_deref() {
        Some(trash) => {
            crate::cleaner::move_to_trash(path, trash)?;
        }
        None => {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

#[derive(Default)]
struct MediaTags {
    resolution: Option<String>,
    video_codec: Option<String>,
    audio_codec: Option<String>,
    channels: Option<String>,
    hdr: Option<String>,
    languages: Option<String>,
}

fn read_media_tags(path: &Path) -> MediaTags {
    let output = Command::new("mediainfo")
        .args(["--Output=JSON", &path.to_string_lossy()])
        .output();
    let Ok(output) = output else {
        return MediaTags::default();
    };
    if !output.status.success() {
        return MediaTags::default();
    }
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .map(|value| parse_media_tags(&value))
        .unwrap_or_default()
}

fn media_text(track: Option<&serde_json::Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        track
            .and_then(|value| value.get(*key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

/// legacy `_resolution_tag`: width/height thresholds with an `i` suffix for
/// interlaced sources.
fn media_resolution(video: Option<&serde_json::Value>) -> Option<String> {
    let parse = |keys: &[&str]| {
        media_text(video, keys).and_then(|value| {
            value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<i64>().ok())
        })
    };
    let height = parse(&["Height"]);
    let width = parse(&["Width"]);
    if height.is_none() && width.is_none() {
        return None;
    }
    let height = height.unwrap_or(0);
    let width = width.unwrap_or(0);
    let suffix = if media_text(video, &["Scan_type"])
        .is_some_and(|value| value.to_ascii_lowercase() == "interlaced")
    {
        "i"
    } else {
        "p"
    };
    let base = if width >= 3800 || height >= 2000 {
        "2160"
    } else if width >= 1900 || height >= 1000 {
        "1080"
    } else if width >= 1200 || height >= 700 {
        "720"
    } else if height >= 540 {
        "576"
    } else {
        "480"
    };
    Some(format!("{base}{suffix}"))
}

/// legacy `_hdr_tag`: DV HDR10 / DV / HDR10Plus / HDR10 / HLG / HDR.
fn media_hdr(video: Option<&serde_json::Value>) -> Option<String> {
    let format = media_text(video, &["HDR_Format"]).unwrap_or_default();
    let format_string = media_text(video, &["HDR_Format_String"]).unwrap_or_default();
    let compatibility = media_text(video, &["HDR_Format_Compatibility"]).unwrap_or_default();
    let transfer = media_text(video, &["Transfer_characteristics"]).unwrap_or_default();
    let all = format!("{format} {format_string} {compatibility}")
        .trim()
        .to_string();
    if all.is_empty() && transfer.is_empty() {
        return None;
    }
    let dv = all.contains("Dolby Vision");
    let hdr10_plus = all.contains("HDR10+") || all.contains("HDR10 Plus");
    let hdr10 = all.contains("HDR10");
    let hlg = transfer.contains("HLG") || all.contains("HLG");
    let pq = transfer.contains("PQ");
    let value = if dv && hdr10 {
        "DV HDR10"
    } else if dv {
        "DV"
    } else if hdr10_plus {
        "HDR10Plus"
    } else if hdr10 {
        "HDR10"
    } else if hlg {
        "HLG"
    } else if pq {
        "HDR"
    } else {
        return None;
    };
    Some(value.to_string())
}

/// legacy `_video_codec_tag`.
fn media_video_codec(video: Option<&serde_json::Value>) -> Option<String> {
    let format = media_text(video, &["Format"]).unwrap_or_default().to_lowercase();
    let codec = media_text(video, &["CodecID"]).unwrap_or_default().to_lowercase();
    if format == "hevc" || codec.contains("hevc") {
        return Some("h265".into());
    }
    if format == "avc" || codec.contains("avc") {
        return Some("h264".into());
    }
    if format == "av1" || codec.contains("av01") {
        return Some("AV1".into());
    }
    if format.contains("xvid") || codec.contains("xvid") {
        return Some("XviD".into());
    }
    if format.contains("divx") {
        return Some("DivX".into());
    }
    if format.contains("vc-1") || format.contains("vc1") {
        return Some("VC-1".into());
    }
    (!format.is_empty()).then(|| format.to_uppercase())
}

/// legacy `_audio_codec_tag`.
fn media_audio_codec(audio: Option<&serde_json::Value>) -> Option<String> {
    let format = media_text(audio, &["Format"]).unwrap_or_default().to_lowercase();
    let commercial = media_text(audio, &["Commercial_Name"]).unwrap_or_default().to_lowercase();
    let profile = media_text(audio, &["Format_Profile"]).unwrap_or_default().to_lowercase();
    let codec = media_text(audio, &["CodecID"]).unwrap_or_default().to_lowercase();
    let atmos = profile.contains("atmos") || commercial.contains("atmos") || format.contains("joc");
    let value = if commercial.contains("truehd") || format.contains("truehd") {
        if atmos { "TrueHD Atmos" } else { "TrueHD" }
    } else if format.contains("e-ac-3")
        || format.contains("eac-3")
        || codec.contains("eac3")
        || commercial.contains("dolby digital plus")
    {
        if atmos { "EAC3 Atmos" } else { "EAC3" }
    } else if format.contains("ac-3")
        || codec.contains("ac3")
        || (commercial.contains("dolby digital") && !commercial.contains("plus"))
    {
        "AC3"
    } else if format.contains("dts") || codec.contains("dts") {
        if profile.contains("ma") || profile.contains("master") {
            "DTS-MA"
        } else if profile.contains("x") && !profile.contains("x:") {
            "DTS-X"
        } else {
            "DTS"
        }
    } else if format.contains("aac") || codec.contains("aac") {
        "AAC"
    } else if format.contains("flac") {
        "FLAC"
    } else if format.contains("opus") {
        "Opus"
    } else if format.contains("mp3") || format.contains("mpeg") {
        "MP3"
    } else if format.contains("pcm") {
        "PCM"
    } else if !format.is_empty() {
        return Some(format.to_uppercase());
    } else {
        return None;
    };
    Some(value.to_string())
}

/// legacy `_channels_tag`.
fn media_channels(audio: Option<&serde_json::Value>) -> Option<String> {
    let channels = media_text(audio, &["Channel_s"])?
        .split_whitespace()
        .next()?
        .parse::<i64>()
        .ok()?;
    Some(match channels {
        1 => "Mono".to_string(),
        2 => "Stereo".to_string(),
        6 => "5.1".to_string(),
        8 => "7.1".to_string(),
        value => format!("{value}ch"),
    })
}

/// legacy `_LANG_MAP` (primary subtag only, ISO 639-1 style).
fn media_language(language: &str) -> Option<String> {
    let primary = language
        .to_lowercase()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let mapped = match primary.as_str() {
        "italian" | "italiano" | "it" | "ita" => "IT",
        "english" | "inglese" | "en" | "eng" => "EN",
        "french" | "francese" | "fr" | "fra" => "FR",
        "spanish" | "spagnolo" | "es" | "spa" => "ES",
        "german" | "tedesco" | "de" | "deu" => "DE",
        "portuguese" | "pt" | "por" => "PT",
        "russian" | "ru" | "rus" => "RU",
        "japanese" | "ja" | "jpn" => "JA",
        "chinese" | "zh" | "zho" => "ZH",
        "arabic" | "ar" | "ara" => "AR",
        other if !other.is_empty() => return Some(other.to_ascii_uppercase()),
        _ => return None,
    };
    Some(mapped.to_string())
}

/// legacy `_SOURCE_LABEL`.
fn source_label(source: &str) -> String {
    match source.to_ascii_lowercase().as_str() {
        "bluray" => "BluRay".into(),
        "webdl" => "WEB-DL".into(),
        "webrip" => "WEBRip".into(),
        "hdtv" => "HDTV".into(),
        "dvdrip" => "DVDRip".into(),
        "remux" => "REMUX".into(),
        other => other.to_string(),
    }
}

/// legacy `_sanitize`: remove path-invalid characters (does not replace them).
fn sanitize_invalid(name: &str) -> String {
    name.chars()
        .filter(|character| !matches!(character, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// legacy post-template cleanup: drop empty brackets, collapse spaces and trim
/// trailing separators.
fn cleanup_filename(stem: &str) -> String {
    let without_empty = Regex::new(r"\[\s*\]|\(\s*\)")
        .map(|regex| regex.replace_all(stem, "").to_string())
        .unwrap_or_else(|_| stem.to_string());
    let collapsed = Regex::new(r"\s+")
        .map(|regex| regex.replace_all(&without_empty, " ").trim().to_string())
        .unwrap_or(without_empty);
    Regex::new(r"[\s\-_]+$")
        .map(|regex| regex.replace(&collapsed, "").to_string())
        .unwrap_or(collapsed)
}

fn parse_media_tags(value: &serde_json::Value) -> MediaTags {
    let tracks = value
        .get("media")
        .and_then(|value| value.get("track"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let video = tracks
        .iter()
        .find(|track| track.get("@type").and_then(serde_json::Value::as_str) == Some("Video"));
    let audio_tracks = tracks
        .iter()
        .filter(|track| track.get("@type").and_then(serde_json::Value::as_str) == Some("Audio"))
        .collect::<Vec<_>>();
    // legacy prefers the audio track flagged Default=Yes.
    let audio = audio_tracks
        .iter()
        .copied()
        .find(|track| {
            media_text(Some(track), &["Default"])
                .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "yes" | "true" | "1"))
        })
        .or_else(|| audio_tracks.first().copied());
    let mut languages: Vec<String> = Vec::new();
    for track in &audio_tracks {
        if let Some(language) = media_text(Some(track), &["Language", "Language_String"]) {
            if let Some(normalized) = media_language(&language) {
                if !languages.contains(&normalized) {
                    languages.push(normalized);
                }
            }
        }
    }
    MediaTags {
        resolution: media_resolution(video),
        video_codec: media_video_codec(video),
        audio_codec: media_audio_codec(audio),
        channels: media_channels(audio),
        hdr: media_hdr(video),
        languages: (!languages.is_empty()).then(|| languages.join("+")),
    }
}

/// Single video file for a season/episode inside a directory, if exactly one
/// matches. Used when a completed torrent's file was already moved and renamed
/// in the archive, so its original torrent name no longer exists.
pub fn find_episode_file(dir: &Path, season: i64, episode: i64) -> Option<PathBuf> {
    let files = video_files(dir).ok()?;
    let mut matches = files.into_iter().filter(|file| {
        file.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| filename_matches_episode(name, season, episode))
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

pub fn video_files(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "mkv" | "mp4" | "avi" | "m4v" | "mov" | "ts" | "webm" | "wmv"
                )
            })
            .map(|_| vec![path.to_path_buf()])
            .unwrap_or_default());
    }
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            continue;
        }
        let child = entry.path();
        if child.is_dir() {
            files.extend(video_files(&child)?);
        } else if child
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "mkv" | "mp4" | "avi" | "m4v" | "mov" | "ts" | "webm" | "wmv"
                )
            })
        {
            files.push(child);
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, SeriesConfig};

    #[test]
    fn media_tags_match_extto_forms() {
        let value = serde_json::json!({"media":{"track":[
            {"@type":"Video","Width":"3840","Height":"2160",
             "HDR_Format":"Dolby Vision / SMPTE ST 2094 App 4",
             "HDR_Format_String":"Dolby Vision / SMPTE ST 2094 App 4 / HDR10",
             "Transfer_characteristics":"PQ","Format":"HEVC","CodecID":"hvc1"},
            {"@type":"Audio","Format":"E-AC-3","Channel_s":"6","Language":"it","Default":"Yes"},
            {"@type":"Audio","Format":"AC-3","Channel_s":"2","Language":"en-US"}
        ]}});
        let tags = parse_media_tags(&value);
        assert_eq!(tags.resolution.as_deref(), Some("2160p"));
        assert_eq!(tags.video_codec.as_deref(), Some("h265"));
        assert_eq!(tags.hdr.as_deref(), Some("DV HDR10"));
        assert_eq!(tags.audio_codec.as_deref(), Some("EAC3"));
        assert_eq!(tags.channels.as_deref(), Some("5.1"));
        assert_eq!(tags.languages.as_deref(), Some("IT+EN"));
        // AC3 5.1 form (legacy's `{Audio}` joins codec and channels).
        let ac3 = serde_json::json!({"media":{"track":[
            {"@type":"Video","Width":"1920","Height":"1080","Format":"AVC"},
            {"@type":"Audio","Format":"AC-3","Channel_s":"6","Language":"it"}
        ]}});
        let tags = parse_media_tags(&ac3);
        assert_eq!(tags.audio_codec.as_deref(), Some("AC3"));
        assert_eq!(tags.channels.as_deref(), Some("5.1"));
        assert_eq!(tags.hdr, None);
    }

    #[test]
    fn atomic_file_copy_publishes_complete_target() {
        let root = std::env::temp_dir().join(format!("rextto-atomic-copy-{}", uuid::Uuid::new_v4()));
        let source = root.join("source.mkv");
        let target = root.join("archive").join("episode.mkv");
        std::fs::create_dir_all(&root).unwrap();
        let content = vec![b'x'; 128 * 1024];
        std::fs::write(&source, &content).unwrap();

        copy_file_atomically(&source, &target).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), content);
        assert!(!std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".rextto-copy-")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn filename_particles_follow_extto() {
        assert_eq!(source_label("webdl"), "WEB-DL");
        assert_eq!(source_label("bluray"), "BluRay");
        assert_eq!(source_label("webrip"), "WEBRip");
        assert_eq!(source_label("hdtv"), "HDTV");
        assert_eq!(source_label("dvdrip"), "DVDRip");
        // Empty brackets (with or without inner spaces) are removed.
        assert_eq!(
            cleanup_filename("Show - S01E01 - T - [][1080p][DV] "),
            "Show - S01E01 - T - [1080p][DV]"
        );
        assert_eq!(
            cleanup_filename("Show - [  ][1080p][]  "),
            "Show - [1080p]"
        );
        // Double spaces collapse and dangling separators are trimmed.
        assert_eq!(cleanup_filename("A  -  B - "), "A - B");
        assert_eq!(sanitize_invalid("Luci: spente? / test"), "Luci spente  test");
        // Apostrophes are preserved (legacy test_sanitize_preserva_apostrofo).
        assert_eq!(sanitize_invalid("Widow's Bay"), "Widow's Bay");
        assert_eq!(sanitize_invalid(""), "");
    }

    #[test]
    fn restores_source_before_resolution() {
        assert_eq!(
            restore_source_token("Show - S01E01 - Titolo - [1080p][h264][AAC][IT].mkv", "webdl"),
            Some("Show - S01E01 - Titolo - [WEB-DL][1080p][h264][AAC][IT].mkv".to_string())
        );
        // Già presente: niente da fare.
        assert_eq!(
            restore_source_token("Show - S01E01 - T - [WEB-DL][720p].mkv", "webdl"),
            None
        );
        // Sorgente sconosciuta o nessun tag risoluzione: niente.
        assert_eq!(
            restore_source_token("Show - S01E01 - T - [1080p].mkv", "unknown"),
            None
        );
        assert_eq!(restore_source_token("Show - S01E01 - T.mkv", "webdl"), None);
    }

    #[test]
    fn rejects_names_with_empty_bracket_artifacts() {
        let mut cfg = Config::default();
        cfg.rename_episodes = true;
        cfg.rename_format = "custom".into();
        cfg.rename_template =
            "{Serie} - {Stagione}{Episodio} - {Titolo} - [{Source}][{Risoluzione}][{VideoCodec}][{HDR}][{Audio}][{Lingue}]"
                .into();
        cfg.series.push(SeriesConfig {
            name: "Only Murders in the Building".into(),
            enabled: true,
            ..Default::default()
        });
        let release = Release { torrent_url: None,
            title: "Only Murders in the Building S01E01".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "rss".into(),
            quality: Default::default(),
            kind: "series".into(),
            series: Some("Only Murders in the Building".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: false,
            episode_range: vec![1],
            year: None,
            discovered_at: chrono::Utc::now(),
        };
        // Gruppi vuoti `[]`: NON conforme, va rinominato per ripulirlo.
        let broken = Path::new(
            "Only Murders in the Building - S01E01 - True Crime - [][480p][h264][][AAC][IT].mkv",
        );
        assert!(!episode_name_conforms(broken, &release, &cfg));
        // Un placeholder non sostituito non è un nome conforme: altrimenti il
        // controllo del solo prefisso salterebbe la rinomina.
        let unrendered_placeholder = Path::new(
            "Only Murders in the Building - S01E01 - True Crime - [{Source}][480p][h264][AAC][IT].mkv",
        );
        assert!(!episode_name_conforms(
            unrendered_placeholder,
            &release,
            &cfg
        ));
        // Nome già pulito: conforme.
        let clean = Path::new(
            "Only Murders in the Building - S01E01 - True Crime - [480p][h264][AAC][IT].mkv",
        );
        assert!(episode_name_conforms(clean, &release, &cfg));
    }

    #[test]
    fn conform_check_sanitizes_invalid_series_name_characters() {
        // A colon cannot appear in a path: the real rename target uses the
        // sanitised name, so the cheap conform check must use it too, otherwise
        // it triggers a pointless no-op rename.
        let mut cfg = Config::default();
        cfg.rename_episodes = true;
        cfg.rename_format = "standard".into();
        cfg.series.push(SeriesConfig {
            name: "Star Trek: Strange New Worlds".into(),
            enabled: true,
            ..Default::default()
        });
        let release = Release { torrent_url: None,
            title: "Star Trek Strange New Worlds S03E10".into(),
            magnet: String::new(),
            source: "archive".into(),
            quality: Default::default(),
            kind: "series".into(),
            series: Some("Star Trek: Strange New Worlds".into()),
            season: Some(3),
            episode: Some(10),
            is_pack: false,
            episode_range: vec![10],
            year: None,
            discovered_at: chrono::Utc::now(),
        };
        let file = Path::new(
            "/x/Star Trek Strange New Worlds - S03E10 - Nuove forme - [2160p][h265].mkv",
        );
        assert!(episode_name_conforms(file, &release, &cfg));
    }

    #[test]
    fn renames_sidecars_and_trashes_duplicates() {
        let root = std::env::temp_dir().join(format!("rextto-sidecar-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("Show.01x01.Pilot.ITA.WEBRIP.mkv");
        fs::write(&source, b"video").unwrap();
        fs::write(root.join("Show.01x01.Pilot.ITA.WEBRIP-thumb.jpg"), b"thumb").unwrap();
        fs::write(root.join("Show.01x01.Pilot.ITA.WEBRIP.srt"), b"sub").unwrap();
        fs::write(
            root.join("Show.01x01.Pilot.ITA.WEBRIP-thumb (copia 1).jpg"),
            b"dup",
        )
        .unwrap();
        let target = root.join("Show - S01E01 - Pilot.mkv");
        let mut cfg = Config::default();
        cfg.cleanup_action = "delete".into();
        let moved = rename_sidecars(&source, &target, &cfg).unwrap();
        assert!(root.join("Show - S01E01 - Pilot-thumb.jpg").is_file());
        assert!(root.join("Show - S01E01 - Pilot.srt").is_file());
        assert!(!root
            .join("Show.01x01.Pilot.ITA.WEBRIP-thumb (copia 1).jpg")
            .exists());
        assert_eq!(moved.len(), 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_series_archive_before_global_archive() {
        let mut cfg = Config::default();
        cfg.archive_root = Some("/archive".into());
        cfg.series.push(SeriesConfig {
            name: "Example".into(),
            archive_path: "/nas/example".into(),
            enabled: true,
            ..Default::default()
        });
        let release = Release { torrent_url: None,
            title: "Example S01E01".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "rss".into(),
            quality: Default::default(),
            kind: "series".into(),
            series: Some("Example".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: false,
            episode_range: vec![1],
            year: None,
            discovered_at: chrono::Utc::now(),
        };
        assert_eq!(
            destination_for(&release, &cfg),
            Some(PathBuf::from("/nas/example"))
        );
    }

    #[test]
    fn places_series_episodes_in_configured_season_subfolder() {
        let mut cfg = Config::default();
        cfg.series.push(SeriesConfig {
            name: "Example".into(),
            archive_path: "/nas/example".into(),
            enabled: true,
            season_subfolders: true,
            ..Default::default()
        });
        let release = Release { torrent_url: None,
            title: "Example S02E03".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "rss".into(),
            quality: Default::default(),
            kind: "series".into(),
            series: Some("Example".into()),
            season: Some(2),
            episode: Some(3),
            is_pack: false,
            episode_range: vec![3],
            year: None,
            discovered_at: chrono::Utc::now(),
        };
        assert_eq!(
            destination_for(&release, &cfg),
            Some(PathBuf::from("/nas/example/Stagione 02"))
        );
    }

    #[test]
    fn matches_legacy_extto_tag_names_for_movies_and_series() {
        let mut cfg = Config::default();
        cfg.settings.insert(
            "tag_dir_rules".into(),
            r#"[{"tag":"Film","temp_dir":"","final_dir":"/home/user/film"},{"tag":"Serie TV","temp_dir":"","final_dir":"/home/user/serie"}]"#.into(),
        );
        let movie = Release { torrent_url: None,
            title: "Example Movie".into(),
            magnet: "magnet:?xt=urn:btih:abcdefabcdefabcdefabcdefabcdefabcdefabcd".into(),
            source: "archive".into(),
            quality: Default::default(),
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2024),
            discovered_at: chrono::Utc::now(),
        };
        assert_eq!(
            destination_for(&movie, &cfg),
            Some(PathBuf::from("/home/user/film"))
        );
        let series = Release { torrent_url: None,
            kind: "series".into(),
            series: Some("Example".into()),
            season: Some(1),
            episode: Some(1),
            episode_range: vec![1],
            ..movie.clone()
        };
        assert_eq!(
            destination_for(&series, &cfg),
            Some(PathBuf::from("/home/user/serie"))
        );
    }

    #[test]
    fn copies_pack_files_flat_without_removing_the_source() {
        let root = std::env::temp_dir().join(format!("rextto-pack-{}", std::process::id()));
        let source = root.join("source").join("Example");
        let destination = root.join("archive");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("Example.S01E01.mkv"), b"episode").unwrap();
        let copied = copy_pack_files(&root.join("source"), &destination).unwrap();
        assert_eq!(copied.len(), 1);
        assert!(source.join("Example.S01E01.mkv").is_file());
        assert!(destination.join("Example.S01E01.mkv").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn matches_numbered_partial_pack_without_season_in_file_names() {
        let root = std::env::temp_dir().join(format!("rextto-numbered-pack-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("01.mkv"), b"one").unwrap();
        fs::write(root.join("Episode 02.mkv"), b"two").unwrap();
        let release = Release { torrent_url: None,
            title: "Example.S03E01-02.1080p.WEB-DL".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "test".into(), quality: Default::default(), kind: "series".into(),
            series: Some("Example".into()), season: Some(3), episode: Some(1),
            is_pack: true, episode_range: vec![1, 2], year: None, discovered_at: chrono::Utc::now(),
        };
        let files = matching_pack_files(&root, &release).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files.iter().map(|file| file.episode).collect::<Vec<_>>(), vec![1, 2]);
        assert!(files.iter().all(|file| file.season == 3));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn does_not_guess_episodes_for_complete_pack_with_opaque_names() {
        let root = std::env::temp_dir().join(format!("rextto-opaque-pack-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("01.mkv"), b"one").unwrap();
        let release = Release { torrent_url: None,
            title: "Example.S03.COMPLETE.1080p.WEB-DL".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "test".into(), quality: Default::default(), kind: "series".into(),
            series: Some("Example".into()), season: Some(3), episode: Some(0),
            is_pack: true, episode_range: vec![0], year: None, discovered_at: chrono::Utc::now(),
        };
        assert!(matching_pack_files(&root, &release).unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_partial_pack_when_any_declared_episode_is_missing() {
        let root = std::env::temp_dir().join(format!("rextto-incomplete-pack-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Example.S03E01.mkv"), b"one").unwrap();
        let release = Release { torrent_url: None,
            title: "Example.S03E01-02.1080p.WEB-DL".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "test".into(), quality: Default::default(), kind: "series".into(),
            series: Some("Example".into()), season: Some(3), episode: Some(1),
            is_pack: true, episode_range: vec![1, 2], year: None, discovered_at: chrono::Utc::now(),
        };
        assert!(matching_pack_files(&root, &release).unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn video_files_include_legacy_webm_and_wmv_extensions() {
        let root = std::env::temp_dir().join(format!("rextto-video-ext-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Example.S01E01.webm"), b"webm").unwrap();
        fs::write(root.join("Example.S01E02.wmv"), b"wmv").unwrap();
        assert_eq!(video_files(&root).unwrap().len(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_destination_inside_source_tree() {
        let root = std::env::temp_dir().join(format!("rextto-paths-{}", std::process::id()));
        let source = root.join("downloads").join("pack");
        let destination = source.join("archive");
        fs::create_dir_all(&source).unwrap();
        assert!(validate_destination_from(&source, &destination).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn renames_episode_without_tmdb_key_using_safe_fallback() {
        let root = std::env::temp_dir().join(format!("rextto-rename-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("source.mkv"), b"episode").unwrap();
        let mut cfg = Config::default();
        cfg.rename_episodes = true;
        cfg.rename_format = "standard".into();
        cfg.series.push(SeriesConfig {
            name: "Example".into(),
            enabled: true,
            ..Default::default()
        });
        let release = Release { torrent_url: None,
            title: "Example S01E01".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "rss".into(),
            quality: Default::default(),
            kind: "series".into(),
            series: Some("Example".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: false,
            episode_range: vec![1],
            year: None,
            discovered_at: chrono::Utc::now(),
        };
        let renamed = rename_episode(&root, &release, &cfg, &TmdbClient::new(None))
            .await
            .unwrap()
            .unwrap();
        assert!(renamed
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("S01E01"));
        assert!(!root.join("source.mkv").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn preview_picks_episode_file_from_series_folder() {
        let root = std::env::temp_dir().join(format!("rextto-rename-dir-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Example - S01E01 - Primo.mkv"), b"a").unwrap();
        fs::write(root.join("Example - S01E02 - Secondo.mkv"), b"b").unwrap();
        let mut cfg = Config::default();
        cfg.rename_episodes = true;
        cfg.rename_format = "standard".into();
        cfg.series.push(SeriesConfig {
            name: "Example".into(),
            seasons: "1+".into(),
            enabled: true,
            ..Default::default()
        });
        let release = Release { torrent_url: None,
            title: "Example S01E02".into(),
            magnet: String::new(),
            source: "archive".into(),
            quality: Default::default(),
            kind: "series".into(),
            series: Some("Example".into()),
            season: Some(1),
            episode: Some(2),
            is_pack: false,
            episode_range: vec![2],
            year: None,
            discovered_at: chrono::Utc::now(),
        };
        let target = preview_episode_rename(&root, &release, &cfg, &TmdbClient::new(None))
            .await
            .unwrap()
            .unwrap();
        let name = target.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.contains("S01E02"), "nome inatteso: {name}");
        assert!(
            !name.contains("S01E01"),
            "ha scelto l'episodio sbagliato: {name}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn renames_movie_without_tmdb_key_using_configured_title() {
        let root = std::env::temp_dir().join(format!("rextto-movie-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Example.Movie.2024.1080p.mkv"), b"movie").unwrap();
        let mut cfg = Config::default();
        cfg.rename_episodes = true;
        cfg.movies.push(crate::config::MovieConfig {
            name: "Example Movie".into(),
            year: "2024".into(),
            enabled: true,
            ..Default::default()
        });
        let release = Release { torrent_url: None,
            title: "Example.Movie.2024.1080p".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "rss".into(),
            quality: Default::default(),
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2024),
            discovered_at: chrono::Utc::now(),
        };
        let renamed = rename_movie(&root, &release, &cfg, &TmdbClient::new(None))
            .await
            .unwrap()
            .unwrap();
        assert!(renamed
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("Example Movie"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_media_info_tags_for_full_rename() {
        let value = serde_json::json!({"media":{"track":[{"@type":"Video","Width":"1920","Height":"1080","Format":"HEVC","HDR_Format":"HDR10"},{"@type":"Audio","Commercial_Name":"Dolby Digital Plus","Channel_s":"6","Language":"it"},{"@type":"Audio","Format":"AAC","Channel_s":"2","Language":"en"}]}});
        let tags = parse_media_tags(&value);
        assert_eq!(tags.resolution.as_deref(), Some("1080p"));
        assert_eq!(tags.video_codec.as_deref(), Some("h265"));
        assert_eq!(tags.hdr.as_deref(), Some("HDR10"));
        assert_eq!(tags.channels.as_deref(), Some("5.1"));
        assert_eq!(tags.languages.as_deref(), Some("IT+EN"));
    }

    #[test]
    fn tag_rules_choose_temp_and_final_directories() {
        let root = std::env::temp_dir().join(format!("rextto-tagdirs-{}", std::process::id()));
        let temp = root.join("temp");
        let final_dir = root.join("final");
        fs::create_dir_all(&temp).unwrap();
        fs::create_dir_all(&final_dir).unwrap();
        let mut cfg = Config::default();
        cfg.settings.insert(
            "tag_dir_rules".into(),
            serde_json::json!([
                {"tag": "Serie TV", "temp_dir": temp.display().to_string(), "final_dir": final_dir.display().to_string()}
            ])
            .to_string(),
        );
        let release = Release { torrent_url: None,
            title: "Example S01E01".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "rss".into(),
            quality: Default::default(),
            kind: "series".into(),
            series: Some("Example".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: false,
            episode_range: vec![1],
            year: None,
            discovered_at: chrono::Utc::now(),
        };
        assert_eq!(
            download_dir_for(&release, &cfg).as_deref(),
            Some(temp.as_path())
        );
        assert_eq!(
            destination_for(&release, &cfg).as_deref(),
            Some(final_dir.as_path())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ignores_missing_temp_directory_in_tag_rules() {
        let mut cfg = Config::default();
        cfg.settings.insert(
            "tag_dir_rules".into(),
            serde_json::json!([
                {"tag": "movie", "temp_dir": "/nonexistent/rextto-temp", "final_dir": "/tmp/final"}
            ])
            .to_string(),
        );
        let release = Release { torrent_url: None,
            title: "Example Movie 2024".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "rss".into(),
            quality: Default::default(),
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2024),
            discovered_at: chrono::Utc::now(),
        };
        assert!(download_dir_for(&release, &cfg).is_none());
    }

    fn event(name: &str) -> TorrentEvent {
        TorrentEvent {
            kind: "torrent_finished".into(),
            hash: "abcdef".into(),
            name: name.into(),
            save_path: "/downloads".into(),
        }
    }

    #[test]
    fn completion_path_never_escapes_save_path() {
        assert_eq!(completion_path(&event("Show.S01E01.mkv")), Path::new("/downloads/Show.S01E01.mkv"));
        // Absolute and traversal names keep only the final component.
        assert_eq!(completion_path(&event("/etc/passwd")), Path::new("/downloads/passwd"));
        assert_eq!(completion_path(&event("../../etc/passwd")), Path::new("/downloads/passwd"));
        assert_eq!(completion_path(&event("a/b/c.bin")), Path::new("/downloads/c.bin"));
        // A name whose final component is `..` or empty must not resolve to the
        // shared root; callers must see a missing path.
        let root_fallback = completion_path(&event(".."));
        assert_ne!(root_fallback, Path::new("/downloads"));
        assert_eq!(root_fallback.parent(), Some(Path::new("/downloads")));
        assert!(root_fallback
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".rextto-missing-"));
        assert_ne!(completion_path(&event("")), Path::new("/downloads"));
    }

    #[test]
    fn completion_path_returns_named_path_even_when_missing() {
        // The old implementation returned the shared save_path when the named
        // entry did not exist, which let callers delete the whole download dir.
        let missing = completion_path(&event("not-here.mkv"));
        assert_eq!(missing, Path::new("/downloads/not-here.mkv"));
        assert!(!missing.exists());
    }

    #[test]
    fn best_episode_file_prefers_higher_resolution_and_ignores_partials() {
        let dir = std::env::temp_dir().join(format!(
            "rextto-best-episode-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Same episode, two qualities, plus a partially copied file that must
        // never be considered (its suffix is not a video extension).
        std::fs::write(dir.join("Show - S01E01 - T - [WEB-DL][1080p][h265].mkv"), b"a").unwrap();
        std::fs::write(dir.join("Show.S01E01.2160p.WEB-DL.H265.mkv"), b"b").unwrap();
        std::fs::write(
            dir.join("Show.S01E01.2160p.WEB-DL.H265.mkv.rextto-part"),
            b"c",
        )
        .unwrap();
        let best = best_episode_file(&dir, 1, 1).expect("episode present");
        assert_eq!(best.file_name().unwrap(), "Show.S01E01.2160p.WEB-DL.H265.mkv");
        assert!(best_episode_file(&dir, 1, 2).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
