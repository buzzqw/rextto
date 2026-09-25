//! Ground-truth media inspection via `ffprobe` (Sonarr/Radarr use the same
//! idea). The release name is only a hint: reading the actual file gives the
//! real bit depth, HDR flavour, audio tracks and subtitles, which makes
//! upgrades and rescoring trustworthy.
//!
//! Parsing is a pure function over the `ffprobe` JSON so it can be unit tested
//! without the binary; [`probe`] is the thin process wrapper. When `ffprobe` is
//! not installed the probe returns `None` and callers keep the filename-based
//! quality, so nothing breaks.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MediaInfo {
    pub container: String,
    pub width: i64,
    pub height: i64,
    pub video_codec: String,
    pub bit_depth: i64,
    /// `""`, `HDR10`, `HDR10+`, `HLG`, `DV`, `DV HDR10`, ...
    pub hdr: String,
    pub audio_codec: String,
    pub audio_channels: i64,
    pub audio_languages: Vec<String>,
    pub subtitle_languages: Vec<String>,
    /// Duration in seconds, rounded.
    pub runtime_seconds: i64,
}

impl MediaInfo {
    /// Resolution bucket derived from the real frame size, used to sanity-check
    /// (or replace) the filename-derived resolution.
    pub fn resolution(&self) -> &'static str {
        if self.width >= 3800 || self.height >= 2000 {
            "2160p"
        } else if self.width >= 1900 || self.height >= 1000 {
            "1080p"
        } else if self.width >= 1200 || self.height >= 700 {
            "720p"
        } else if self.height >= 540 {
            "576p"
        } else if self.height > 0 || self.width > 0 {
            "480p"
        } else {
            ""
        }
    }

    pub fn has_hdr(&self) -> bool {
        !self.hdr.is_empty()
    }

    /// Enriches a filename-derived quality with ground-truth attributes. It is
    /// additive: it fills HDR/codec/audio/resolution only when the probe found
    /// them and the filename value is unknown, so it can never clear a value or
    /// regress an existing file. Used to make upgrade comparisons read the real
    /// archived file, not just its name.
    pub fn apply_to_quality(&self, quality: &mut crate::models::Quality) {
        if !self.hdr.trim().is_empty() {
            if quality.hdr.trim().is_empty() {
                quality.hdr = self.hdr.clone();
            }
            if self.hdr.to_ascii_lowercase().contains("dv") {
                quality.is_dv = true;
            }
        }
        if matches!(quality.codec.trim(), "" | "unknown") {
            if let Some(codec) = codec_label(&self.video_codec) {
                quality.codec = codec.to_string();
            }
        }
        if matches!(quality.audio.trim(), "" | "unknown") {
            if let Some(audio) = audio_label(&self.audio_codec) {
                quality.audio = audio.to_string();
            }
        }
        if matches!(quality.resolution.trim(), "" | "unknown") {
            let resolution = self.resolution();
            if !resolution.is_empty() {
                quality.resolution = resolution.to_string();
            }
        }
        if quality.languages.is_empty() && !self.audio_languages.is_empty() {
            quality.languages = self.audio_languages.clone();
        }
    }
}

/// ffprobe codec name → Rextto quality codec label.
pub fn codec_label(codec: &str) -> Option<&'static str> {
    match codec.trim().to_ascii_lowercase().as_str() {
        "hevc" | "h265" => Some("h265"),
        "h264" | "avc" => Some("h264"),
        "av1" => Some("av1"),
        "vp9" => Some("vp9"),
        "mpeg2video" => Some("mpeg2"),
        "vc1" => Some("vc1"),
        _ => None,
    }
}

/// ffprobe codec name → Rextto quality audio label.
pub fn audio_label(codec: &str) -> Option<&'static str> {
    match codec.trim().to_ascii_lowercase().as_str() {
        "eac3" => Some("ddp"),
        "ac3" => Some("ac3"),
        "aac" => Some("aac"),
        "dts" => Some("dts"),
        "truehd" => Some("truehd"),
        "flac" => Some("flac"),
        "opus" => Some("opus"),
        "mp3" => Some("mp3"),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn stream_text(stream: &Value, key: &str) -> Option<String> {
    text(stream.get(key))
}

/// Best-effort bit depth from the pixel format: `yuv420p10le` → 10, `yuv420p`
/// → 8. Falls back to 8 when unknown.
pub fn bit_depth_from_pix_fmt(pix_fmt: &str) -> i64 {
    let digits: String = pix_fmt
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    // `yuv420p10le` ends in `le`, so scan the whole string for `p<depth>` and
    // for a 9/10/12/16 token before the endianness suffix.
    if let Some(depth) = digits.parse::<i64>().ok().filter(|depth| {
        matches!(depth, 9 | 10 | 12 | 14 | 16)
    }) {
        return depth;
    }
    let lowered = pix_fmt.to_ascii_lowercase();
    for depth in [16, 14, 12, 10, 9] {
        if lowered.contains(&format!("p{depth}")) {
            return depth;
        }
    }
    8
}

/// Maps `ffprobe` colour metadata to a display HDR label. Dolby Vision is
/// detected from the DOVI side data first, then HLG/PQ from the transfer.
pub fn hdr_from_stream(stream: &Value) -> String {
    let side_data = stream
        .get("side_data_list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let dovi = side_data.iter().any(|entry| {
        entry
            .get("side_data_type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.to_ascii_lowercase().contains("dovi"))
    });
    let transfer = stream_text(stream, "color_transfer")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let primaries = stream_text(stream, "color_primaries")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let hlg = transfer.contains("arib-std-b67");
    let pq = transfer.contains("smpte2084") || transfer.contains("pq");
    let is_hdr = hlg || pq || primaries.contains("bt2020");
    if !is_hdr && !dovi {
        return String::new();
    }
    let dynamic = stream.get("tags").is_some_and(|tags| {
        tags.get("NUMBER_OF_DYNAMIC_HDR_LAYERS")
            .or_else(|| tags.get("dynamic_hdr_plus"))
            .is_some()
    });
    match (dovi, hlg, pq, dynamic) {
        (true, true, _, _) => "DV HLG".into(),
        (true, false, true, true) => "DV HDR10+".into(),
        (true, false, true, false) => "DV".into(),
        (false, true, _, _) => "HLG".into(),
        (false, false, true, true) => "HDR10+".into(),
        (false, false, true, false) => "HDR10".into(),
        _ => "HDR".into(),
    }
}

/// Parses the JSON produced by
/// `ffprobe -v error -print_format json -show_format -show_streams`.
pub fn parse_ffprobe(root: &Value) -> MediaInfo {
    let streams = root
        .get("streams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let is_motion_image = |codec: &str| matches!(codec, "mjpeg" | "png" | "bmp" | "gif");
    let video = streams.iter().find(|stream| {
        stream.get("codec_type").and_then(Value::as_str) == Some("video")
            && !is_motion_image(
                stream
                    .get("codec_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });

    let mut info = MediaInfo::default();
    if let Some(video) = video {
        info.width = video.get("width").and_then(Value::as_i64).unwrap_or(0);
        info.height = video.get("height").and_then(Value::as_i64).unwrap_or(0);
        info.video_codec = stream_text(video, "codec_name").unwrap_or_default();
        info.bit_depth = stream_text(video, "pix_fmt")
            .map(|pix_fmt| bit_depth_from_pix_fmt(&pix_fmt))
            .unwrap_or(8);
        info.hdr = hdr_from_stream(video);
    }

    let mut audio = streams
        .iter()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"));
    if let Some(first) = audio.next() {
        info.audio_codec = stream_text(first, "codec_name").unwrap_or_default();
        info.audio_channels = first.get("channels").and_then(Value::as_i64).unwrap_or(0);
    }
    for stream in streams
        .iter()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"))
    {
        if let Some(language) = stream
            .get("tags")
            .and_then(|tags| tags.get("language"))
            .and_then(Value::as_str)
        {
            let language = language.trim().to_ascii_lowercase();
            if !language.is_empty() && !info.audio_languages.contains(&language) {
                info.audio_languages.push(language);
            }
        }
    }
    for stream in streams
        .iter()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("subtitle"))
    {
        if let Some(language) = stream
            .get("tags")
            .and_then(|tags| tags.get("language"))
            .and_then(Value::as_str)
        {
            let language = language.trim().to_ascii_lowercase();
            if !language.is_empty() && !info.subtitle_languages.contains(&language) {
                info.subtitle_languages.push(language);
            }
        }
    }

    if let Some(format) = root.get("format") {
        info.container = stream_text(format, "format_name").unwrap_or_default();
        info.runtime_seconds = stream_text(format, "duration")
            .and_then(|value| value.parse::<f64>().ok())
            .map(|seconds| seconds.round() as i64)
            .unwrap_or(0);
    }
    info
}

const VIDEO_EXTENSIONS: [&str; 8] = ["mkv", "mp4", "avi", "m4v", "ts", "mov", "wmv", "webm"];

/// Probes a video file, or the largest video file directly inside a directory
/// (archive entries are sometimes stored as a folder).
pub fn probe_best(path: &Path) -> Option<MediaInfo> {
    if path.is_file() {
        return probe(path);
    }
    if !path.is_dir() {
        return None;
    }
    let mut best: Option<(u64, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(path).ok()?.flatten() {
        let candidate = entry.path();
        if !candidate.is_file() {
            continue;
        }
        let extension = candidate
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !VIDEO_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        if best.as_ref().is_none_or(|(current, _)| size > *current) {
            best = Some((size, candidate));
        }
    }
    best.and_then(|(_, path)| probe(&path))
}

/// True when `ffprobe` can be executed. Lets the scheduler skip runs instead
/// of retrying every entry when the binary is missing.
pub fn available() -> bool {
    std::process::Command::new("ffprobe")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Runs `ffprobe` on `path`. Returns `None` when the binary is missing, the
/// file is unreadable, or the output is not valid JSON.
pub fn probe(path: &Path) -> Option<MediaInfo> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    Some(parse_ffprobe(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_hdr10_10bit_with_audio_and_subtitles() {
        let root = json!({
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "hevc",
                    "width": 1920,
                    "height": 1080,
                    "pix_fmt": "yuv420p10le",
                    "color_primaries": "bt2020",
                    "color_transfer": "smpte2084"
                },
                {
                    "codec_type": "audio",
                    "codec_name": "eac3",
                    "channels": 6,
                    "tags": {"language": "ita"}
                },
                {
                    "codec_type": "audio",
                    "codec_name": "aac",
                    "channels": 2,
                    "tags": {"language": "eng"}
                },
                {
                    "codec_type": "subtitle",
                    "codec_name": "subrip",
                    "tags": {"language": "ita"}
                }
            ],
            "format": {"format_name": "matroska,webm", "duration": "5400.42"}
        });
        let info = parse_ffprobe(&root);
        assert_eq!(info.resolution(), "1080p");
        assert_eq!(info.video_codec, "hevc");
        assert_eq!(info.bit_depth, 10);
        assert_eq!(info.hdr, "HDR10");
        assert_eq!(info.audio_codec, "eac3");
        assert_eq!(info.audio_channels, 6);
        assert_eq!(info.audio_languages, vec!["ita", "eng"]);
        assert_eq!(info.subtitle_languages, vec!["ita"]);
        assert_eq!(info.runtime_seconds, 5400);
        assert!(info.has_hdr());
    }

    #[test]
    fn parses_dolby_vision_from_side_data() {
        let root = json!({
            "streams": [{
                "codec_type": "video",
                "codec_name": "hevc",
                "width": 3840,
                "height": 2160,
                "pix_fmt": "yuv420p10le",
                "color_primaries": "bt2020",
                "color_transfer": "smpte2084",
                "side_data_list": [{"side_data_type": "DOVI configuration record"}]
            }],
            "format": {"format_name": "matroska", "duration": "60"}
        });
        let info = parse_ffprobe(&root);
        assert!(info.hdr.starts_with("DV"), "got {}", info.hdr);
        assert_eq!(info.resolution(), "2160p");
    }

    #[test]
    fn skips_motion_image_video_streams() {
        let root = json!({
            "streams": [
                {"codec_type": "video", "codec_name": "mjpeg", "width": 600, "height": 600},
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": 1280,
                    "height": 720,
                    "pix_fmt": "yuv420p"
                }
            ],
            "format": {"format_name": "mp4", "duration": "120"}
        });
        let info = parse_ffprobe(&root);
        assert_eq!(info.video_codec, "h264");
        assert_eq!(info.resolution(), "720p");
        assert_eq!(info.bit_depth, 8);
        assert_eq!(info.hdr, "");
    }

    #[test]
    fn apply_to_quality_is_additive_only() {
        let info = MediaInfo {
            hdr: "HDR10".into(),
            video_codec: "hevc".into(),
            audio_codec: "eac3".into(),
            width: 1920,
            height: 1080,
            audio_languages: vec!["ita".into()],
            ..Default::default()
        };
        let mut unknown = crate::models::Quality {
            resolution: "1080p".into(),
            codec: "unknown".into(),
            audio: "unknown".into(),
            ..Default::default()
        };
        info.apply_to_quality(&mut unknown);
        assert_eq!(unknown.hdr, "HDR10");
        assert_eq!(unknown.codec, "h265");
        assert_eq!(unknown.audio, "ddp");
        assert_eq!(unknown.languages, vec!["ita"]);
        // Known values are never overwritten.
        let mut known = crate::models::Quality {
            resolution: "1080p".into(),
            hdr: "HLG".into(),
            codec: "h264".into(),
            audio: "dts".into(),
            ..Default::default()
        };
        info.apply_to_quality(&mut known);
        assert_eq!(known.hdr, "HLG");
        assert_eq!(known.codec, "h264");
        assert_eq!(known.audio, "dts");
    }

    #[test]
    fn bit_depth_handles_endianness_suffixes() {
        assert_eq!(bit_depth_from_pix_fmt("yuv420p10le"), 10);
        assert_eq!(bit_depth_from_pix_fmt("yuv420p"), 8);
        assert_eq!(bit_depth_from_pix_fmt("yuv444p12be"), 12);
        assert_eq!(bit_depth_from_pix_fmt("gbrp16le"), 16);
    }
}
