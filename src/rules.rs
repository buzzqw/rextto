//! Built-in, always-on release sanity rules.
//!
//! These replace the former user-configurable release rules / size envelopes:
//! the useful checks are hard-coded with values derived from a real archive
//! instead of asking the user to guess numbers. They mirror the safe defaults
//! Sonarr/Radarr apply out of the box.
//!
//! Two checks are implemented:
//!
//! * **Hardcoded subtitles** — Radarr rejects them by default. A release that
//!   carries burned-in subs is refused with a clear reason.
//! * **Absurd size** — a per-resolution sanity floor (MiB) derived from a real
//!   4962-file archive (median and 5th percentile). It drops fake/truncated
//!   releases without touching legitimate short episodes.
//!
//! A small size preference is added to the score so, within the same
//! resolution, a healthier bitrate is preferred; it is capped so it can never
//! outweigh a quality difference (resolution/source/codec).

use crate::models::Release;

/// Size sanity floor in MiB for a resolution. Values come from the 5th
/// percentile of a real archive, rounded down with margin.
///
/// | resolution | p05 archive | floor |
/// |------------|-------------|-------|
/// | 2160p      | ~2668 MiB   | 1200  |
/// | 1080p      | ~354 MiB    | 180   |
/// | 720p       | ~497 MiB    | 120   |
/// | 576p/480p  | ~289 MiB    | 60    |
pub fn size_floor_mb(resolution: &str) -> i64 {
    match resolution {
        "2160p" => 1200,
        "1080p" => 180,
        "720p" => 120,
        "576p" => 80,
        "480p" => 60,
        _ => 50,
    }
}

pub fn size_floor_bytes(resolution: &str) -> u64 {
    (size_floor_mb(resolution).max(0) as u64) * 1_048_576
}

/// Reason a release is refused by a built-in rule, or `None` when it passes.
/// Unknown sizes are never rejected (the value simply is not known).
pub fn denied_reason(release: &Release) -> Option<String> {
    if release.quality.hardcoded_subs {
        return Some("hardcoded subtitles".into());
    }
    if release.size_bytes > 0 {
        let floor = size_floor_bytes(&release.quality.resolution);
        if (release.size_bytes as u64) < floor {
            let resolution = if release.quality.resolution.trim().is_empty() {
                "unknown".to_string()
            } else {
                release.quality.resolution.clone()
            };
            return Some(format!(
                "size {} MiB below the {} sanity floor of {} MiB",
                release.size_bytes / 1_048_576,
                resolution,
                size_floor_mb(&release.quality.resolution)
            ));
        }
    }
    None
}

/// Logs a rejected release. A rejection caused by a built-in rule (size floor
/// or hardcoded subtitles) is logged at `INFO` with the name and basic data so
/// the user can see *why* an otherwise-valid release was dropped; every other
/// filter stays at `DEBUG` to keep cycles quiet.
pub fn log_rejection(release: &Release, reason: &str) {
    if denied_reason(release).is_some() {
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
/// +1 per 100 MiB above the sanity floor, capped. It can never bridge a
/// quality gap (resolution/source/codec deltas are far larger).
pub fn size_score_bonus(release: &Release) -> i64 {
    if release.size_bytes <= 0 {
        return 0;
    }
    let floor = size_floor_bytes(&release.quality.resolution);
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
    fn size_floor_rejects_only_absurd_sizes() {
        assert!(denied_reason(&release("1080p", 1500)).is_none());
        let tiny = denied_reason(&release("1080p", 40)).unwrap();
        assert!(tiny.contains("sanity floor"));
        assert!(denied_reason(&release("2160p", 1200)).is_none());
        assert!(denied_reason(&release("2160p", 300)).is_some());
        // Unknown size is never rejected.
        let mut unknown = release("1080p", 1500);
        unknown.size_bytes = 0;
        assert!(denied_reason(&unknown).is_none());
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
        assert_eq!(size_score_bonus(&release("1080p", 100)), 0); // below floor
        assert_eq!(size_score_bonus(&release("1080p", 180)), 0); // at floor
        assert_eq!(size_score_bonus(&release("1080p", 280)), 1);
        assert_eq!(size_score_bonus(&release("1080p", 100_000)), 100);
        let mut unknown = release("1080p", 5000);
        unknown.size_bytes = 0;
        assert_eq!(size_score_bonus(&unknown), 0);
    }
}
