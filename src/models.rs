use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Quality {
    pub resolution: String,
    pub source: String,
    pub codec: String,
    pub audio: String,
    pub hdr: String,
    pub group: String,
    pub is_ita: bool,
    pub is_dv: bool,
    #[serde(default)]
    pub is_repack: bool,
    #[serde(default)]
    pub is_proper: bool,
    #[serde(default)]
    pub is_real: bool,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub has_subtitle: bool,
    #[serde(default)]
    pub subtitle_languages: Vec<String>,
    /// Burned-in subtitles (`HC`/`hardcoded`): refused by a built-in rule.
    #[serde(default)]
    pub hardcoded_subs: bool,
}

impl Quality {
    /// Scomposizione del punteggio base per categoria (usata anche dal
    /// simulatore punteggi della UI).
    pub fn score_breakdown(&self) -> Vec<(&'static str, i64)> {
        let resolution = [
            ("2160p", 2000),
            ("1080p", 1000),
            ("720p", 400),
            ("576p", 80),
        ]
        .into_iter()
        .find(|(key, _)| self.resolution == *key)
        .map(|(_, value)| value)
        .unwrap_or(0);
        let source = [
            ("bluray", 300),
            ("remux", 280),
            ("webdl", 200),
            ("webrip", 150),
            ("hdtv", 50),
            ("dvdrip", 20),
        ]
        .into_iter()
        .find(|(key, _)| self.source == *key)
        .map(|(_, value)| value)
        .unwrap_or(0);
        let codec = if matches!(self.codec.as_str(), "h265" | "x265" | "hevc") {
            200
        } else if matches!(self.codec.as_str(), "h264" | "x264" | "avc") {
            50
        } else {
            0
        };
        let audio = if self.audio.contains("truehd") {
            150
        } else if self.audio.contains("dts-hd") {
            120
        } else if self.audio.contains("dts") {
            100
        } else if self.audio.contains("ddp") || self.audio.contains("eac3") {
            80
        } else if self.audio.contains("ac3") || self.audio.contains("5.1") {
            50
        } else if self.audio.contains("aac") {
            30
        } else if self.audio.contains("mp3") {
            10
        } else {
            0
        };
        vec![
            ("Risoluzione", resolution),
            ("Sorgente", source),
            ("Codec", codec),
            ("Audio", audio),
            ("HDR", if !self.hdr.is_empty() { 100 } else { 0 }),
            ("Dolby Vision", if self.is_dv { 300 } else { 0 }),
            ("Proper", if self.is_proper { 75 } else { 0 }),
            ("Repack", if self.is_repack { 50 } else { 0 }),
            ("Real", if self.is_real { 100 } else { 0 }),
        ]
    }

    pub fn score(&self) -> i64 {
        self.score_breakdown().iter().map(|(_, value)| *value).sum()
    }

    /// Any recognised HDR flavour (Dolby Vision included).
    pub fn has_hdr(&self) -> bool {
        self.is_dv || !self.hdr.is_empty()
    }

    /// Un REMUX conserva la qualità piena della sorgente: usato come criterio di
    /// preferenza a parità di punteggio.
    pub fn is_remux(&self) -> bool {
        self.source == "remux"
    }

    pub fn resolution_rank(&self) -> i32 {
        match self.resolution.as_str() {
            "2160p" => 6,
            "1080p" => 5,
            "720p" => 4,
            "576p" => 3,
            "480p" => 2,
            "360p" => 1,
            _ => 0,
        }
    }

    pub fn source_rank(&self) -> i32 {
        match self.source.as_str() {
            // Il REMUX è la copia a risoluzione piena di un BluRay: stesso rango
            // del BluRay, così le regole di upgrade lo trattano correttamente.
            "bluray" | "remux" => 5,
            "webdl" => 4,
            "webrip" => 3,
            "dvdrip" => 2,
            "hdtv" => 1,
            _ => 0,
        }
    }

    /// Ports legacy's `Quality.upgrade_reason`: explains why `self` may replace
    /// `old`, or `None` when the change is not worth a re-download. A resolution
    /// jump always wins; HDTV→WEB-DL, gaining HDR and a first REPACK are
    /// meaningful even below the additive score threshold; otherwise the score
    /// must improve by at least `min_score_diff`.
    pub fn upgrade_reason(
        &self,
        old: &Quality,
        new_score: i64,
        old_score: i64,
        min_score_diff: i64,
    ) -> Option<&'static str> {
        let new_res = self.resolution_rank();
        let old_res = old.resolution_rank();
        if new_res > old_res {
            return Some("resolution");
        }
        if old.source == "hdtv" && self.source == "webdl" && new_res >= old_res {
            return Some("source");
        }
        // A parità di punteggio un REMUX vince sulla copia esistente, in
        // particolare quando è il remux dell'episodio già scaricato: è la
        // versione a risoluzione piena, senza ricodifica.
        if self.is_remux() && !old.is_remux() && new_res >= old_res && new_score >= old_score {
            return Some("remux");
        }
        if self.has_hdr() && !old.has_hdr() && new_res >= old_res {
            return Some("hdr");
        }
        if self.is_repack
            && !old.is_repack
            && new_res >= old_res
            && self.source_rank() >= old.source_rank()
        {
            return Some("repack");
        }
        // I nomi normalizzati dell'archivio possono non conservare la sorgente
        // (`WEB-DL`/`WEBRip`). In quel caso non trattare il solo riempimento di
        // `unknown` come un upgrade: altrimenti ogni file già presente sembra
        // inferiore alla stessa release con il tag della sorgente nel titolo.
        if old.source == "unknown"
            && self.source != "unknown"
            && self.same_non_source_quality(old)
        {
            return None;
        }
        if new_score > old_score && new_score - old_score >= min_score_diff {
            return Some("score");
        }
        None
    }

    /// Confronta i campi che contribuiscono alla qualità ma non la sorgente.
    /// La sorgente viene esclusa intenzionalmente perché può essere assente
    /// soltanto dal nome file archiviato, non dal file video reale.
    fn same_non_source_quality(&self, other: &Quality) -> bool {
        self.resolution == other.resolution
            && self.codec == other.codec
            && self.audio == other.audio
            && self.hdr == other.hdr
            && self.is_dv == other.is_dv
            && self.is_repack == other.is_repack
            && self.is_proper == other.is_proper
            && self.is_real == other.is_real
    }

    pub fn score_with_settings(
        &self,
        settings: &std::collections::BTreeMap<String, String>,
    ) -> i64 {
        let mut score = self.score();
        let adjust = |score: &mut i64, key: &str, default: i64, active: bool| {
            if active {
                if let Some(value) = settings
                    .get(key)
                    .and_then(|value| value.parse::<i64>().ok())
                {
                    *score += value - default;
                }
            }
        };
        let resolution_defaults = [
            ("2160p", 2000),
            ("1080p", 1000),
            ("720p", 400),
            ("576p", 80),
        ];
        for (resolution, default) in resolution_defaults {
            adjust(
                &mut score,
                &format!("score_res_{resolution}"),
                default,
                self.resolution == resolution,
            );
        }
        let source_defaults = [
            ("bluray", 300),
            ("remux", 280),
            ("webdl", 200),
            ("webrip", 150),
            ("hdtv", 50),
            ("dvdrip", 20),
        ];
        for (source, default) in source_defaults {
            adjust(
                &mut score,
                &format!("score_source_{source}"),
                default,
                self.source == source,
            );
        }
        let codec_defaults = [
            ("h265", 200),
            ("h264", 50),
            ("x265", 200),
            ("x264", 50),
            ("hevc", 200),
            ("avc", 50),
        ];
        for (codec, default) in codec_defaults {
            adjust(
                &mut score,
                &format!("score_codec_{codec}"),
                default,
                self.codec == codec,
            );
        }
        let audio_defaults = [
            ("truehd", 150),
            ("dts-hd", 120),
            ("dts", 100),
            ("ddp", 80),
            ("eac3", 80),
            ("ac3", 50),
            ("5.1", 50),
            ("aac", 30),
            ("mp3", 10),
        ];
        for (audio, default) in audio_defaults {
            adjust(
                &mut score,
                &format!("score_audio_{audio}"),
                default,
                self.audio.contains(audio),
            );
        }
        adjust(&mut score, "score_bonus_dv", 300, self.is_dv);
        adjust(&mut score, "score_bonus_hdr", 100, !self.hdr.is_empty());
        adjust(&mut score, "score_bonus_proper", 75, self.is_proper);
        adjust(&mut score, "score_bonus_repack", 50, self.is_repack);
        adjust(&mut score, "score_bonus_real", 100, self.is_real);
        let group_key = format!("score_group_{}", self.group.to_ascii_lowercase());
        if let Some(value) = settings
            .get(&group_key)
            .and_then(|value| value.parse::<i64>().ok())
        {
            score += value;
        }
        score
    }
}

/// Miglior qualità trovata **su disco** per `(stagione, episodio)` nella
/// cartella di una serie. Serve a decidere gli upgrade guardando i file reali
/// (come il legacy `_best_quality_in_path`), non solo le righe del DB: un file
/// non collegato non deve far riscaricare una release uguale o peggiore.
#[derive(Debug, Clone, Default)]
pub struct ArchiveQualityIndex {
    pub best: std::collections::HashMap<(i64, i64), (Quality, i64)>,
}

impl ArchiveQualityIndex {
    pub fn best_for(&self, season: i64, episode: i64) -> Option<&(Quality, i64)> {
        self.best.get(&(season, episode))
    }

    pub fn is_empty(&self) -> bool {
        self.best.is_empty()
    }
}

/// Torrent attualmente nella sessione libtorrent (non solo nel DB): serve a
/// bloccare una release già in corso, come `LibtorrentClient.list_torrents`
/// del legacy.
#[derive(Debug, Clone, Default)]
pub struct LiveDownloads {
    pub hashes: std::collections::HashSet<String>,
    pub episodes: std::collections::HashSet<(String, i64, i64)>,
}

impl LiveDownloads {
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty() && self.episodes.is_empty()
    }
}

/// Dati extra per la decisione di approvazione: cosa c'è su disco (archivio) e
/// cosa c'è già in download (sessione live).
#[derive(Debug, Clone, Default)]
pub struct ApprovalContext {
    pub archive: Arc<ArchiveQualityIndex>,
    pub live: Arc<LiveDownloads>,
    /// When true, an existing archived file is never replaced. Set from a
    /// quality profile with `upgrade_allowed = false` or when the cutoff
    /// resolution is already reached.
    pub forbid_upgrade: bool,
    /// True when this candidate fills a known archive gap. Gap-fill always
    /// wins over the "no older episode" best-practice guard.
    pub gap_episode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub title: String,
    pub magnet: String,
    /// Link `.torrent` diretto di un feed RSS (es. TorrentLeech) quando la
    /// release non espone un magnet: viene risolto in infohash prima dell'uso.
    #[serde(default)]
    pub torrent_url: Option<String>,
    pub source: String,
    pub quality: Quality,
    pub kind: String,
    pub series: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    #[serde(default)]
    pub is_pack: bool,
    pub episode_range: Vec<i64>,
    pub year: Option<i64>,
    pub discovered_at: DateTime<Utc>,
    /// Size advertised by the indexer/feed, in bytes. `0` means unknown, in
    /// which case size-based policy rules are skipped instead of guessing.
    #[serde(default)]
    pub size_bytes: i64,
    /// Seeders advertised by the indexer/feed, or `-1` when unknown.
    #[serde(default = "unknown_peer_count")]
    pub seeders: i64,
    /// Leechers advertised by the indexer/feed, or `-1` when unknown.
    #[serde(default = "unknown_peer_count")]
    pub peers: i64,
}

fn unknown_peer_count() -> i64 {
    -1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentMeta {
    pub release: Release,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CycleStats {
    pub scraped: usize,
    pub candidates: usize,
    pub downloads_started: usize,
    pub gaps_filled: usize,
    pub errors: usize,
    pub error_details: HashMap<String, usize>,
    pub last_started_at: Option<DateTime<Utc>>,
}

impl CycleStats {
    pub fn error(&mut self, category: &str) {
        self.errors += 1;
        *self.error_details.entry(category.into()).or_default() += 1;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentView {
    pub hash: String,
    pub name: String,
    pub progress: f64,
    pub state: String,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub save_path: String,
    pub download_limit: i64,
    pub upload_limit: i64,
    pub all_time_upload: i64,
    pub all_time_download: i64,
    pub seeding_seconds: i64,
    pub queue_position: i32,
    pub num_peers: i32,
    pub num_seeds: i32,
    /// -1 uses the global policy; 0 means infinite seeding.
    pub seed_ratio: f64,
    pub seed_days: i64,
    pub has_metadata: bool,
    pub auto_managed: bool,
    /// Torrent metadata version: "v1", "v2", "hybrid" or "" when metadata is missing.
    #[serde(default)]
    pub torrent_version: String,
    /// Total bytes to download (0 while metadata is missing).
    #[serde(default)]
    pub total_size: i64,
    /// Bytes already downloaded (0 while metadata is missing).
    #[serde(default)]
    pub total_done: i64,
    /// True when Rextto has parked the torrent as stalled while retaining it
    /// for periodic peer/seed discovery retries.
    #[serde(default)]
    pub stalled: bool,
}

/// Escalating backoff state of one source (feed, indexer or web engine).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderStatus {
    pub provider: String,
    pub kind: String,
    pub level: i64,
    pub disabled_till: String,
    pub most_recent_failure: String,
    pub last_error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentEvent {
    pub kind: String,
    pub hash: String,
    pub name: String,
    pub save_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerView {
    pub address: String,
    pub client: String,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub pieces: i32,
    pub seed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerView {
    pub url: String,
    pub tier: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileView {
    pub path: String,
    pub size: i64,
    pub downloaded: i64,
    /// libtorrent priority: 0 skipped, 1 normal, 6 high, 7 maximum.
    #[serde(default = "default_priority")]
    pub priority: i32,
}

fn default_priority() -> i32 {
    4
}

#[cfg(test)]
mod tests {
    use super::Quality;
    use std::collections::BTreeMap;

    #[test]
    fn upgrade_reason_ports_extto_rules() {
        let base = Quality {
            resolution: "1080p".into(),
            source: "webdl".into(),
            codec: "h264".into(),
            audio: "aac".into(),
            ..Default::default()
        };
        // Resolution jump wins regardless of score.
        let mut higher = base.clone();
        higher.resolution = "2160p".into();
        assert_eq!(
            higher.upgrade_reason(&base, higher.score(), base.score(), 200),
            Some("resolution")
        );
        // HDTV -> WEB-DL is meaningful even below the additive threshold.
        let hdtv = Quality {
            resolution: "1080p".into(),
            source: "hdtv".into(),
            ..Default::default()
        };
        assert_eq!(
            base.upgrade_reason(&hdtv, base.score(), base.score(), 200),
            Some("source")
        );
        // Gaining HDR at the same resolution.
        let mut hdr = base.clone();
        hdr.hdr = "HDR10".into();
        assert_eq!(
            hdr.upgrade_reason(&base, hdr.score(), base.score(), 200),
            Some("hdr")
        );
        // First REPACK at the same resolution/source.
        let mut repack = base.clone();
        repack.is_repack = true;
        assert_eq!(
            repack.upgrade_reason(&base, repack.score(), base.score(), 200),
            Some("repack")
        );
        // Same quality, small score deltas are not upgrades.
        assert_eq!(
            base.upgrade_reason(&base, base.score() + 50, base.score(), 200),
            None
        );
        // A large score gain is a score upgrade.
        assert_eq!(
            base.upgrade_reason(&base, base.score() + 500, base.score(), 200),
            Some("score")
        );
    }

    #[test]
    fn missing_archived_source_does_not_create_upgrade() {
        let archived = Quality {
            resolution: "1080p".into(),
            source: "unknown".into(),
            codec: "h264".into(),
            audio: "ddp".into(),
            ..Default::default()
        };
        let candidate = Quality {
            source: "webrip".into(),
            ..archived.clone()
        };
        assert_eq!(
            candidate.upgrade_reason(&archived, candidate.score(), archived.score(), 200),
            None
        );
    }

    #[test]
    fn remux_wins_at_equal_or_better_score() {
        let webdl = Quality {
            resolution: "1080p".into(),
            source: "webdl".into(),
            codec: "h264".into(),
            audio: "aac".into(),
            ..Default::default()
        };
        let remux = Quality {
            resolution: "1080p".into(),
            source: "remux".into(),
            codec: "h264".into(),
            audio: "aac".into(),
            ..Default::default()
        };
        // Il REMUX ha uno score maggiore (280 vs 200) ma sotto la soglia minima:
        // vince comunque grazie alla regola dedicata.
        assert!(remux.score() > webdl.score());
        assert_eq!(
            remux.upgrade_reason(&webdl, remux.score(), webdl.score(), 200),
            Some("remux")
        );
        // Un REMUX non sostituisce un altro REMUX a parità di punteggio.
        assert_eq!(
            remux.upgrade_reason(&remux, remux.score(), remux.score(), 200),
            None
        );
        // Un REMUX con punteggio inferiore a un BluRay non vince.
        let bluray = Quality {
            resolution: "1080p".into(),
            source: "bluray".into(),
            codec: "h264".into(),
            audio: "aac".into(),
            ..Default::default()
        };
        assert!(bluray.score() > remux.score());
        assert_eq!(
            remux.upgrade_reason(&bluray, remux.score(), bluray.score(), 200),
            None
        );
    }

    #[test]
    fn score_breakdown_sums_to_the_total_score() {
        let quality = Quality {
            resolution: "1080p".into(),
            source: "webdl".into(),
            codec: "h264".into(),
            audio: "aac".into(),
            hdr: "HDR10".into(),
            ..Default::default()
        };
        let total: i64 = quality
            .score_breakdown()
            .iter()
            .map(|(_, value)| *value)
            .sum();
        assert_eq!(total, quality.score());
        // 1080p + webdl + h264 + aac + HDR = 1000 + 200 + 50 + 30 + 100
        assert_eq!(quality.score(), 1380);
    }

    #[test]
    fn language_does_not_change_quality_score() {
        let base = Quality {
            resolution: "1080p".into(),
            source: "webdl".into(),
            codec: "h264".into(),
            audio: "aac".into(),
            language: "ita".into(),
            is_ita: true,
            ..Default::default()
        };
        let mut english = base.clone();
        english.language = "eng".into();
        english.is_ita = false;
        assert_eq!(base.score(), english.score());
    }

    #[test]
    fn codec_and_audio_score_overrides_are_applied() {
        let quality = Quality {
            codec: "h265".into(),
            audio: "aac".into(),
            ..Default::default()
        };
        let mut settings = BTreeMap::new();
        settings.insert("score_codec_h265".into(), "500".into());
        settings.insert("score_audio_aac".into(), "100".into());
        assert_eq!(
            quality.score_with_settings(&settings),
            quality.score() + 300 + 70
        );
    }

    #[test]
    fn standard_bonus_overrides_preserve_the_base_score() {
        let quality = Quality {
            is_dv: true,
            is_proper: true,
            is_repack: true,
            is_real: true,
            hdr: "HDR10".into(),
            ..Default::default()
        };
        let settings = BTreeMap::from([
            ("score_bonus_dv".into(), "300".into()),
            ("score_bonus_hdr".into(), "100".into()),
            ("score_bonus_proper".into(), "75".into()),
            ("score_bonus_repack".into(), "50".into()),
            ("score_bonus_real".into(), "100".into()),
        ]);
        assert_eq!(quality.score_with_settings(&settings), quality.score());
    }
}
