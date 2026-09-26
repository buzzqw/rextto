//! Spiegazione read-only delle decisioni su una release.
//!
//! Questo modulo non sostituisce ancora il percorso di approvazione usato dal
//! ciclo automatico. Ricostruisce i controlli senza scrivere nel database, in
//! modo che la UI possa spiegare una scelta senza creare placeholder, backup o
//! altri effetti collaterali.

use crate::{
    config::{Config, MovieConfig, SeriesConfig},
    database::Database,
    models::{Quality, Release, TorrentMeta},
    parser::parse_quality,
    rules,
    utils::magnet_hash,
};
use anyhow::Result;
use rusqlite::OptionalExtension;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DecisionStep {
    pub rule: String,
    pub result: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionTrace {
    pub decision: String,
    pub reason: String,
    pub candidate: String,
    pub target: String,
    pub score: i64,
    pub score_components: Vec<serde_json::Value>,
    pub steps: Vec<DecisionStep>,
    pub comparison: Option<serde_json::Value>,
}

fn step(rule: impl Into<String>, result: &str, detail: impl Into<String>) -> DecisionStep {
    DecisionStep {
        rule: rule.into(),
        result: result.into(),
        detail: detail.into(),
    }
}

fn current_quality(title: &str) -> Quality {
    parse_quality(title)
}

/// Scans the configured series archive. This is deliberately exposed to the
/// HTTP layer so it can run in `spawn_blocking`; walking a NAS must not occupy
/// a Tokio worker.
pub fn archive_quality(cfg: &Config, release: &Release) -> Option<(Quality, i64)> {
    let series_name = release.series.as_deref()?;
    let series = cfg.find_series_match(series_name, release.season)?;
    let archive_path = cfg.resolve_archive_path(series)?;
    let index = crate::cleaner::index_archive(&series.name, &archive_path, &cfg.settings);
    index.best_for(release.season?, release.episode?).cloned()
}

fn target_for(release: &Release) -> String {
    if release.kind == "series" {
        let series = release.series.as_deref().unwrap_or(&release.title);
        if release.is_pack {
            format!("{series} S{:02} pack", release.season.unwrap_or_default())
        } else {
            format!(
                "{series} S{:02}E{:02}",
                release.season.unwrap_or_default(),
                release.episode.unwrap_or_default()
            )
        }
    } else {
        match release.year {
            Some(year) => format!("{} ({year})", release.title),
            None => release.title.clone(),
        }
    }
}

fn release_match<'a>(
    cfg: &'a Config,
    release: &Release,
) -> (Option<&'a SeriesConfig>, Option<&'a MovieConfig>) {
    if release.kind == "series" {
        let name = release.series.as_deref().unwrap_or(&release.title);
        (cfg.find_series_match(name, release.season), None)
    } else if release.kind == "movie" {
        (
            None,
            cfg.find_movie_match_manual(&release.title, release.year),
        )
    } else {
        (None, None)
    }
}

fn active_reason(db: &Database, release: &Release, hash: Option<&str>) -> Result<Option<String>> {
    if let Some(hash) = hash {
        let active: bool = db.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM torrent_meta WHERE lower(hash)=lower(?1) AND status NOT IN ('completed','error','removed'))",
            [hash],
            |row| row.get(0),
        )?;
        if active {
            return Ok(Some("lo stesso hash è già attivo".into()));
        }
    }
    if release.kind == "series" {
        if let (Some(series), Some(season), Some(episode)) =
            (release.series.as_deref(), release.season, release.episode)
        {
            let active: bool = db.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM torrent_meta WHERE lower(series_name)=lower(?1) AND season=?2 AND episode=?3 AND status NOT IN ('completed','error','removed'))",
                rusqlite::params![series, season, episode],
                |row| row.get(0),
            )?;
            if active {
                return Ok(Some("l'episodio ha già un download attivo".into()));
            }
        }
    }
    Ok(None)
}

fn archive_comparison(
    db: &Database,
    cfg: &Config,
    release: &Release,
    score: i64,
    forbid_upgrade: bool,
    disk: Option<(Quality, i64)>,
) -> Result<Option<serde_json::Value>> {
    if release.kind == "movie" {
        let row: Option<(String, i64, Option<String>, String)> = db
            .conn
            .query_row(
                "SELECT COALESCE(m.title,''), m.quality_score, m.downloaded_at, COALESCE(t.metadata_json,'') FROM movies m LEFT JOIN torrent_meta t ON lower(t.hash)=lower(m.magnet_hash) WHERE m.name=?1 AND m.year IS ?2 AND m.removed_at IS NULL",
                rusqlite::params![release.title, release.year],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((title, old_score, downloaded_at, metadata_json)) = row else {
            return Ok(None);
        };
        let old_quality = serde_json::from_str::<TorrentMeta>(&metadata_json)
            .map(|metadata| metadata.release.quality)
            .unwrap_or_else(|_| current_quality(&title));
        let upgrade_reason = release
            .quality
            .upgrade_reason(&old_quality, score, old_score, cfg.upgrade_min_score_diff)
            .map(str::to_string);
        return Ok(Some(serde_json::json!({
            "kind": "movie",
            "title": title,
            "quality": old_quality,
            "score": old_score,
            "downloaded": downloaded_at.is_some(),
            "upgrade_allowed": !forbid_upgrade,
            "upgrade_reason": upgrade_reason,
        })));
    }

    let (Some(series), Some(season), Some(episode)) =
        (release.series.as_deref(), release.season, release.episode)
    else {
        return Ok(None);
    };
    let row: Option<(String, i64, String, Option<String>)> = db
        .conn
        .query_row(
            "SELECT COALESCE(e.title,''), e.quality_score, COALESCE(e.archive_path,''), e.downloaded_at FROM episodes e JOIN series s ON s.id=e.series_id WHERE lower(s.name)=lower(?1) AND e.season=?2 AND e.episode=?3",
            rusqlite::params![series, season, episode],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((title, old_score, archive_path, downloaded_at)) = row else {
        let Some((quality, disk_score)) = disk else {
            return Ok(None);
        };
        let upgrade_reason = release
            .quality
            .upgrade_reason(&quality, score, disk_score, cfg.upgrade_min_score_diff)
            .map(str::to_string);
        return Ok(Some(serde_json::json!({
            "kind": "episode",
            "title": "file presente su disco",
            "quality": quality,
            "score": disk_score,
            "archive_path": "",
            "downloaded": true,
            "upgrade_allowed": !forbid_upgrade,
            "upgrade_reason": upgrade_reason,
            "source": "disk",
        })));
    };
    let disk_present = disk.is_some();
    let (old_quality, old_score) = match disk {
        Some((disk_quality, disk_score)) => (
            crate::parser::merge_quality(disk_quality, current_quality(&title)),
            disk_score.max(old_score),
        ),
        None => (current_quality(&title), old_score),
    };
    let upgrade_reason = release
        .quality
        .upgrade_reason(&old_quality, score, old_score, cfg.upgrade_min_score_diff)
        .map(str::to_string);
    Ok(Some(serde_json::json!({
        "kind": "episode",
        "title": title,
        "quality": old_quality,
        "score": old_score,
        "archive_path": archive_path,
        "downloaded": downloaded_at.is_some(),
        "upgrade_allowed": !forbid_upgrade,
        "upgrade_reason": upgrade_reason,
        "source": if disk_present { "disk+database" } else { "database" },
    })))
}

/// Produce a detailed, side-effect-free explanation of the automatic decision.
pub fn explain(cfg: &Config, db: &Database, release: &Release) -> Result<DecisionTrace> {
    let disk = archive_quality(cfg, release);
    explain_with_archive(cfg, db, release, disk)
}

/// Same explanation using a previously computed archive index. The web layer
/// uses this variant after moving the filesystem scan to a blocking task.
pub fn explain_with_archive(
    cfg: &Config,
    db: &Database,
    release: &Release,
    disk: Option<(Quality, i64)>,
) -> Result<DecisionTrace> {
    let mut steps = Vec::new();
    let mut first_failure = None::<String>;
    let (series, movie) = release_match(cfg, release);
    let target = target_for(release);
    // L'orchestrator canonizza il titolo prima di calcolare lo score e di
    // confrontarlo con il DB. La spiegazione deve fare lo stesso, altrimenti
    // una ricerca manuale con il titolo originale non vedrebbe il film già
    // archiviato con il nome configurato.
    let mut evaluated = release.clone();
    if let Some(series) = series {
        evaluated.series = Some(series.name.clone());
    }
    if let Some(movie) = movie {
        evaluated.title = movie.name.clone();
        evaluated.year = movie.year.parse::<i64>().ok().or(evaluated.year);
    }
    let score = cfg.release_score(&evaluated);
    let mut score_components = vec![serde_json::json!({
        "label": "qualità",
        "value": cfg.quality_score(&evaluated.quality)
    })];
    if let Some(movie) = movie {
        let bonus = Config::movie_subtitle_bonus(movie, &evaluated.quality);
        if bonus != 0 {
            score_components.push(serde_json::json!({
                "label": "preferenza sottotitoli",
                "value": bonus
            }));
        }
    }
    let size_bonus = rules::size_score_bonus(&evaluated);
    if size_bonus != 0 {
        score_components.push(serde_json::json!({
            "label": "dimensione",
            "value": size_bonus
        }));
    }

    if series.is_some() || movie.is_some() {
        steps.push(step(
            "titolo monitorato",
            "pass",
            "La release corrisponde a un elemento monitorato.",
        ));
    } else {
        let detail = "Nessun film o serie attiva corrisponde al titolo, stagione e anno.";
        steps.push(step("titolo monitorato", "fail", detail));
        first_failure = Some(detail.into());
    }

    let hash = magnet_hash(&release.magnet);
    if let Some(hash) = hash.as_deref() {
        let blocked = db.is_blocklisted(hash)?;
        if blocked {
            let detail = "L'hash è presente nella blocklist.";
            steps.push(step("blocklist", "fail", detail));
            first_failure.get_or_insert_with(|| detail.into());
        } else {
            steps.push(step(
                "blocklist",
                "pass",
                "Hash non presente nella blocklist.",
            ));
        }
    } else {
        let detail = "La release non contiene un infohash verificabile.";
        steps.push(step("infohash", "fail", detail));
        first_failure.get_or_insert_with(|| detail.into());
    }

    if let Some(reason) = cfg.release_denied_reason(release) {
        steps.push(step("filtri globali", "fail", reason));
        first_failure.get_or_insert_with(|| reason.into());
    } else {
        steps.push(step(
            "filtri globali",
            "pass",
            "Blacklist, filtri contenuto ed età massima superati.",
        ));
    }
    if let Some(reason) = cfg.source_filter_denied_reason(release) {
        steps.push(step("filtro sorgente", "fail", reason.clone()));
        first_failure.get_or_insert(reason);
    } else {
        steps.push(step(
            "filtro sorgente",
            "pass",
            "Nessun filtro per sorgente rifiuta questa release.",
        ));
    }
    if let Some(reason) = rules::denied_reason(release) {
        steps.push(step("sanità della release", "fail", reason.clone()));
        first_failure.get_or_insert(reason);
    } else {
        steps.push(step(
            "sanità della release",
            "pass",
            "Dimensione minima e sottotitoli hardcoded superati.",
        ));
    }

    let policy_allowed = match (series, movie) {
        (Some(series), _) => cfg.series_release_allowed(series, &release.quality, &release.title),
        (_, Some(movie)) => cfg.movie_release_allowed(movie, &release.quality),
        _ => false,
    };
    if policy_allowed {
        steps.push(step(
            "qualità, lingua e sottotitoli",
            "pass",
            "La release rispetta i requisiti del titolo.",
        ));
    } else {
        let detail = match (series, movie) {
            (Some(series), _) => format!(
                "Configurazione: qualità '{}', lingua '{}', sottotitoli '{}', esclusioni '{}'.",
                series.quality, series.language, series.subtitle, series.exclude
            ),
            (_, Some(movie)) => format!(
                "Configurazione: qualità '{}', lingua '{}', requisiti lingua '{}', sottotitoli '{}', requisiti sottotitoli '{}'.",
                movie.quality,
                movie.language,
                movie.language_requirements,
                movie.subtitle,
                movie.subtitle_requirements
            ),
            _ => "Nessun profilo di titolo disponibile.".into(),
        };
        steps.push(step("qualità, lingua e sottotitoli", "fail", detail.clone()));
        first_failure.get_or_insert(detail);
    }

    if let Some(series) = series {
        let min_size = db.series_archived_min_size(&series.name)?;
        if let Some(reason) = rules::sane_size_denied_reason(release, min_size) {
            steps.push(step(
                "dimensione rispetto all'archivio",
                "fail",
                reason.clone(),
            ));
            first_failure.get_or_insert(reason);
        } else {
            steps.push(step(
                "dimensione rispetto all'archivio",
                "pass",
                "La dimensione è compatibile con la storia dell'archivio.",
            ));
        }
    } else if let Some(movie) = movie {
        let min_size = db.movie_archived_size(&movie.name, release.year)?;
        if let Some(reason) = rules::sane_size_denied_reason(release, min_size) {
            steps.push(step(
                "dimensione rispetto all'archivio",
                "fail",
                reason.clone(),
            ));
            first_failure.get_or_insert(reason);
        } else {
            steps.push(step(
                "dimensione rispetto all'archivio",
                "pass",
                "La dimensione è compatibile con l'archivio.",
            ));
        }
    }

    if let Some(reason) = active_reason(db, &evaluated, hash.as_deref())? {
        steps.push(step("download attivo", "fail", reason.clone()));
        first_failure.get_or_insert(reason);
    } else {
        steps.push(step(
            "download attivo",
            "pass",
            "Non risultano download concorrenti per questo target.",
        ));
    }

    // These checks are deliberately informative: the endpoint receives one
    // release and not the complete cycle context (gap set, other candidates,
    // free space and pending-delay state). They must not be presented as a
    // definitive approval/rejection.
    steps.push(step(
        "selezione del ciclo",
        "info",
        "La scelta finale può inoltre dipendere da gap filling, smart episode, ritardi, spazio libero e confronto con gli altri candidati del ciclo.",
    ));

    let forbid_upgrade = series.is_some_and(|item| item.disable_upgrades)
        || movie.is_some_and(|item| item.disable_upgrades);
    let comparison = archive_comparison(db, cfg, &evaluated, score, forbid_upgrade, disk)?;
    match comparison.as_ref() {
        None => steps.push(step(
            "confronto archivio",
            "pass",
            "Nessun file esistente da sostituire: è un primo download.",
        )),
        Some(current) => {
            let upgrade_reason = current
                .get("upgrade_reason")
                .and_then(|value| value.as_str());
            if forbid_upgrade {
                let detail = "Gli upgrade sono disabilitati per questo titolo.";
                steps.push(step("confronto archivio", "fail", detail));
                first_failure.get_or_insert_with(|| detail.into());
            } else if let Some(reason) = upgrade_reason {
                steps.push(step(
                    "confronto archivio",
                    "pass",
                    format!("Upgrade riconosciuto: {reason}."),
                ));
            } else {
                let detail = "Il file presente è uguale o migliore, oppure il miglioramento non supera la soglia configurata.";
                steps.push(step("confronto archivio", "fail", detail));
                first_failure.get_or_insert_with(|| detail.into());
            }
        }
    }

    let decision = if first_failure.is_some() {
        "rejected"
    } else {
        "eligible"
    };
    Ok(DecisionTrace {
        decision: decision.into(),
        reason: first_failure.unwrap_or_else(|| "La release supera i controlli read-only.".into()),
        candidate: release.title.clone(),
        target,
        score,
        score_components,
        steps,
        comparison,
    })
}

#[cfg(test)]
mod tests {
    use super::explain;
    use crate::{
        config::Config,
        database::Database,
        models::{Quality, Release},
    };
    use chrono::Utc;

    fn release() -> Release {
        Release {
            title: "Example Show S01E01 1080p WEB-DL".into(),
            magnet: format!("magnet:?xt=urn:btih:{}", "a".repeat(40)),
            torrent_url: None,
            source: "test".into(),
            quality: Quality {
                resolution: "1080p".into(),
                source: "webdl".into(),
                ..Default::default()
            },
            kind: "series".into(),
            series: Some("Example Show".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: false,
            episode_range: vec![1],
            year: None,
            discovered_at: Utc::now(),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
        }
    }

    #[test]
    fn explanation_is_read_only_and_structured() {
        let path =
            std::env::temp_dir().join(format!("rextto-decision-{}.db", uuid::Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let trace = explain(&Config::default(), &db, &release()).unwrap();
        assert!(!trace.steps.is_empty());
        assert!(trace.steps.iter().any(|item| item.rule == "blocklist"));
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
