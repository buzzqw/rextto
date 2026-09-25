//! Built-in, always-on release sanity rules.
//!
//! These replace the former user-configurable release rules / size envelopes:
//! the useful checks are hard-coded with values derived from a real archive
//! instead of asking the user to guess numbers.
//!
//! Two checks are implemented:
//!
//! * **Hardcoded subtitles** — Radarr rejects them by default. A release that
//!   carries burned-in subs is refused with a clear reason.
//! * **Absurd size** — a per-resolution sanity floor (MiB). It has two tiers
//!   so it can never exclude a healthy episode:
//!   - a very low **hard floor** applied globally to drop obvious fakes;
//!   - a **sane floor** applied per title, and only when the series already has
//!     archived episodes: it is *lowered* to half the smallest archived episode
//!     when that is below the sane floor. It is never raised, and when a series
//!     has no history the hard floor is used. So a smaller-but-healthy episode
//!     is never rejected just because the series has larger ones.
//!
//! A small size preference is added to the score so, within the same
//! resolution, a healthier bitrate is preferred; it is capped so it can never
//! outweigh a quality difference (resolution/source/codec).

use crate::models::Release;

/// Very low absolute floor: only obvious fakes/truncated files. Applied
/// globally, before a release is matched to a title.
pub fn hard_floor_mb(resolution: &str) -> i64 {
    match resolution {
        "2160p" => 300,
        "1080p" => 80,
        "720p" => 60,
        "576p" => 50,
        "480p" => 40,
        _ => 40,
    }
}

/// Sane per-resolution floor derived from a real archive (smallest real file
/// with margin, after trimming the large extremes).
///
/// | resolution | smallest real file | sane floor |
/// |------------|--------------------|------------|
/// | 2160p      | ~1539 MiB          | 800        |
/// | 1080p      | ~236 MiB           | 120        |
/// | 720p       | ~437 MiB           | 100        |
/// | 576p       | —                  | 60         |
/// | 480p       | ~236 MiB           | 60         |
pub fn sane_floor_mb(resolution: &str) -> i64 {
    match resolution {
        "2160p" => 800,
        "1080p" => 120,
        "720p" => 100,
        "576p" => 60,
        "480p" => 60,
        _ => 50,
    }
}

pub fn hard_floor_bytes(resolution: &str) -> u64 {
    (hard_floor_mb(resolution).max(0) as u64) * 1_048_576
}

pub fn sane_floor_bytes(resolution: &str) -> u64 {
    (sane_floor_mb(resolution).max(0) as u64) * 1_048_576
}

/// Effective sane floor for a title. Lower-only: it can never exceed
/// [`sane_floor_mb`], and a series with no archived history falls back to the
/// permissive hard floor so a healthy episode is not excluded just because the
/// series has no large file yet.
///
/// `series_min_mb` is the smallest archived episode of the series, when known.
pub fn effective_floor_mb(resolution: &str, series_min_mb: Option<i64>) -> i64 {
    match series_min_mb {
        Some(min) if min > 0 => sane_floor_mb(resolution).min(min / 2),
        _ => hard_floor_mb(resolution),
    }
}

/// Global reason a release is refused by a built-in rule (hardcoded subtitles
/// or the hard size floor), or `None` when it passes.
pub fn denied_reason(release: &Release) -> Option<String> {
    if release.quality.hardcoded_subs {
        return Some("hardcoded subtitles".into());
    }
    if release.size_bytes > 0 {
        let floor = hard_floor_bytes(&release.quality.resolution);
        if (release.size_bytes as u64) < floor {
            return Some(format!(
                "size {} MiB below the {} hard floor of {} MiB",
                release.size_bytes / 1_048_576,
                display_resolution(&release.quality.resolution),
                hard_floor_mb(&release.quality.resolution)
            ));
        }
    }
    None
}

/// Per-title sane size reason, using the adaptive floor. Returns `None` when
/// the size is unknown or acceptable.
pub fn sane_size_denied_reason(
    release: &Release,
    series_min_mb: Option<i64>,
) -> Option<String> {
    if release.quality.hardcoded_subs || release.size_bytes <= 0 {
        return None;
    }
    let floor = effective_floor_mb(&release.quality.resolution, series_min_mb);
    if (release.size_bytes as u64) < (floor.max(0) as u64) * 1_048_576 {
        return Some(format!(
            "size {} MiB below the {} sanity floor of {} MiB",
            release.size_bytes / 1_048_576,
            display_resolution(&release.quality.resolution),
            floor
        ));
    }
    None
}

fn display_resolution(resolution: &str) -> String {
    if resolution.trim().is_empty() {
        "unknown".into()
    } else {
        resolution.to_string()
    }
}

/// Logs a rejected release. A rejection caused by a built-in rule (size floor
/// or hardcoded subtitles) is logged at `INFO` with the name and basic data so
/// the user can see *why* an otherwise-valid release was dropped; every other
/// filter stays at `DEBUG`.
pub fn log_rejection(release: &Release, reason: &str) {
    if denied_reason(release).is_some() || reason.contains("sanity floor") {
        tracing::info!(
            title = %release.title,
            source = %release.source,
            resolution = %release.quality.resolution,
            size_mb = release.size_bytes / 1_048_576,
            season = ?release.season,
            episode = ?release.episode,
            reason = %reason,
            "🚫 release rejected"
        );
    } else {
        tracing::debug!(
            title = %release.title,
            source = %release.source,
            reason = %reason,
            "🚫 FILTER rejected"
        );
    }
}

/// Small preference for a healthier bitrate within the same resolution:
/// +1 per 100 MiB above the sane floor, capped. It can never bridge a quality
/// gap (resolution/source/codec deltas are far larger).
pub fn size_score_bonus(release: &Release) -> i64 {
    if release.size_bytes <= 0 {
        return 0;
    }
    let floor = sane_floor_bytes(&release.quality.resolution);
    let size = release.size_bytes as u64;
    if size <= floor {
        return 0;
    }
    (((size - floor) / (100 * 1_048_576)) as i64).min(100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Quality, Release};
    use chrono::Utc;

    fn release(resolution: &str, size_mb: i64) -> Release {
        Release {
            title: format!("Show.S01E01.{resolution}.WEB-DL"),
            magnet: format!("magnet:?xt=urn:btih:{}", "a".repeat(40)),
            torrent_url: None,
            source: "test".into(),
            quality: Quality {
                resolution: resolution.into(),
                ..Default::default()
            },
            kind: "series".into(),
            series: Some("Show".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: false,
            episode_range: vec![1],
            year: None,
            discovered_at: Utc::now(),
            size_bytes: size_mb * 1_048_576,
            seeders: -1,
            peers: -1,
        }
    }

    #[test]
    fn hard_floor_rejects_only_obvious_fakes() {
        assert!(denied_reason(&release("1080p", 1500)).is_none());
        assert!(denied_reason(&release("1080p", 40)).is_some());
        assert!(denied_reason(&release("2160p", 1200)).is_none());
        assert!(denied_reason(&release("2160p", 200)).is_some());
        // Unknown size is never rejected.
        let mut unknown = release("1080p", 1500);
        unknown.size_bytes = 0;
        assert!(denied_reason(&unknown).is_none());
    }

    #[test]
    fn adaptive_floor_never_excludes_healthy_episodes() {
        // No history: the permissive hard floor is used, never the sane one.
        assert_eq!(effective_floor_mb("2160p", None), hard_floor_mb("2160p"));
        // History with large episodes: stays at the sane floor (not raised).
        assert_eq!(effective_floor_mb("2160p", Some(6440)), sane_floor_mb("2160p"));
        // History with small episodes: lowered below the sane floor.
        assert_eq!(effective_floor_mb("2160p", Some(1000)), 500);
        assert_eq!(effective_floor_mb("1080p", Some(200)), 100);
        // A healthy small episode of a small series passes.
        let small = release("2160p", 600);
        assert!(sane_size_denied_reason(&small, Some(1000)).is_none());
        // An episode far below the series scale is refused.
        assert!(sane_size_denied_reason(&small, Some(6000)).is_some());
    }

    #[test]
    fn hardcoded_subtitles_are_rejected() {
        let mut r = release("1080p", 1500);
        assert!(denied_reason(&r).is_none());
        r.quality.hardcoded_subs = true;
        assert_eq!(denied_reason(&r).as_deref(), Some("hardcoded subtitles"));
    }

    #[test]
    fn size_bonus_is_capped_and_never_negative() {
        assert_eq!(size_score_bonus(&release("1080p", 100)), 0); // below sane floor
        assert_eq!(size_score_bonus(&release("1080p", 120)), 0); // at sane floor
        assert_eq!(size_score_bonus(&release("1080p", 220)), 1);
        assert_eq!(size_score_bonus(&release("1080p", 100_000)), 100);
        let mut unknown = release("1080p", 5000);
        unknown.size_bytes = 0;
        assert_eq!(size_score_bonus(&unknown), 0);
    }
}
