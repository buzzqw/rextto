//! Release policy engine.
//!
//! Rextto historically decided whether a release was acceptable with a fixed
//! additive quality score plus a handful of hard-coded filters (blacklist,
//! content filters, per-source keywords, per-title exclude). This module adds
//! the configurable, data-driven layer that Sonarr/Radarr (custom formats,
//! release profiles, per-quality size envelopes), qBittorrent (RSS rules),
//! autobrr (filter engine with rejection reasons) and BiglyBT (subscription
//! filters) all converge on:
//!
//! * [`ReleaseRule`] — an ordered, named predicate set with a reject/score
//!   action. Rejections carry a human-readable reason; score actions feed the
//!   existing upgrade threshold.
//! * [`CustomFormat`] — named predicate sets with a score. Conditions are
//!   grouped by attribute and combined with AND across attributes, OR within
//!   one attribute (a `required` condition makes its group mandatory). This
//!   mirrors Sonarr's `CustomFormat` / `SpecificationMatchesGroup` semantics.
//! * [`SizeRule`] — per-resolution minimum/maximum size in MiB, rejecting
//!   absurdly small (fake/sample) or oversized releases.
//!
//! Everything here is a pure function over [`Release`] so the engine can be
//! unit-tested without a database, network or libtorrent session.

use crate::models::Release;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Which media kind a rule applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MediaScope {
    #[default]
    Any,
    Series,
    Movie,
    Comic,
}

impl MediaScope {
    pub fn matches(self, kind: &str) -> bool {
        match self {
            MediaScope::Any => true,
            MediaScope::Series => kind == "series",
            MediaScope::Movie => kind == "movie",
            MediaScope::Comic => kind == "comic",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            MediaScope::Any => "any",
            MediaScope::Series => "series",
            MediaScope::Movie => "movie",
            MediaScope::Comic => "comic",
        }
    }
}

/// What a matching rule does.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleAction {
    /// Refuse the release. The `reason` is shown in the log and the API.
    Reject {
        #[serde(default)]
        reason: String,
    },
    /// Add `score` to the release score (can be negative). A release whose
    /// total falls below `min_custom_format_score` is rejected by the engine
    /// gate, so negative scores act as penalties.
    Score { score: i64 },
}

impl Default for RuleAction {
    fn default() -> Self {
        RuleAction::Reject {
            reason: String::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// An ordered release predicate with a reject/score action.
///
/// All configured predicates must match for the rule to fire. Empty predicate
/// lists are ignored, so a rule with no predicate matches every release of its
/// scope (useful as a catch-all). Term semantics: plain text is a
/// case-insensitive substring, `*`/`?` become wildcards, `/pattern/flags`
/// (flags `i`, `m`, `s`) becomes a regular expression.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReleaseRule {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Evaluation order, ascending. Rules with `score` actions never stop the
    /// pipeline; reject rules all contribute a reason.
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub media: MediaScope,
    /// Optional source scope: rule only applies when the release `source`
    /// (feed/indexer/engine name) contains this text.
    #[serde(default)]
    pub source: String,
    /// All terms must be present in the release title.
    #[serde(default)]
    pub match_terms: Vec<String>,
    /// At least one term must be present in the release title. When a reject
    /// rule has required terms, it fires because the requirement is *missing*;
    /// for a score rule it adds points only when satisfied.
    #[serde(default)]
    pub required_terms: Vec<String>,
    /// Terms that, when present, make a reject rule fire (the classic "ignore
    /// releases containing CAM/TS" filter). For a score rule they must be
    /// absent for the rule to add points.
    #[serde(default)]
    pub except_terms: Vec<String>,
    #[serde(default)]
    pub resolutions: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub codecs: Vec<String>,
    #[serde(default)]
    pub audio: Vec<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    /// Language codes the release must contain (ITA, ENG, MULTI, ...).
    #[serde(default)]
    pub languages: Vec<String>,
    /// Minimum size in bytes (0 = unlimited). Ignored when the size is unknown.
    #[serde(default)]
    pub min_size_bytes: u64,
    /// Maximum size in bytes (0 = unlimited). Ignored when the size is unknown.
    #[serde(default)]
    pub max_size_bytes: u64,
    #[serde(default)]
    pub min_seeders: Option<i64>,
    #[serde(default)]
    pub max_seeders: Option<i64>,
    #[serde(default)]
    pub min_peers: Option<i64>,
    /// Maximum age in days (0 = unlimited).
    #[serde(default)]
    pub max_age_days: i64,
    #[serde(default)]
    pub action: RuleAction,
}

/// Attribute a custom-format condition inspects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConditionKind {
    #[default]
    Title,
    Group,
    Resolution,
    Source,
    Codec,
    Audio,
    Language,
    Hdr,
    Size,
    Indexer,
    ReleaseType,
}

/// A single custom-format condition. `value` semantics depend on `kind`:
/// size uses `min:max` in MiB (`500:2000`, `:2000`, `500:`), HDR accepts
/// `any`/`none`/`dv`/`hdr10`/`hlg` or a raw token, release type accepts
/// `pack`/`episode`/`movie`/`series`, and every other kind is a term (same
/// semantics as [`ReleaseRule`] terms).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FormatCondition {
    pub kind: ConditionKind,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub negate: bool,
    /// When set, the condition must match for its attribute group to match.
    #[serde(default)]
    pub required: bool,
}

/// A named predicate set worth `score` points. Conditions are grouped by
/// [`ConditionKind`]: every group must match, and inside a group either all
/// `required` conditions match or (when none is required) at least one
/// condition matches.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomFormat {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub conditions: Vec<FormatCondition>,
}

/// Per-resolution size envelope in MiB. `resolution` is `any` or a bucket such
/// as `1080p`; a specific bucket wins over `any`. `0` disables a bound.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SizeRule {
    pub resolution: String,
    #[serde(default)]
    pub min_mb: i64,
    #[serde(default)]
    pub max_mb: i64,
}

/// Result of evaluating the policy for one release.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub score_delta: i64,
    pub rejections: Vec<String>,
    pub matched_formats: Vec<String>,
    pub violated_rules: Vec<String>,
}

impl PolicyDecision {
    pub fn reason(&self) -> Option<String> {
        (!self.rejections.is_empty()).then(|| self.rejections.join("; "))
    }
}

/// The full policy, as stored in the configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub release_rules: Vec<ReleaseRule>,
    #[serde(default)]
    pub custom_formats: Vec<CustomFormat>,
    #[serde(default)]
    pub size_rules: Vec<SizeRule>,
    /// Reject when the summed custom-format score is below this. `i64::MIN`
    /// disables the gate.
    #[serde(default = "default_min_format_score")]
    pub min_custom_format_score: i64,
}

pub fn default_min_format_score() -> i64 {
    i64::MIN
}

impl Policy {
    pub fn evaluate(&self, release: &Release) -> PolicyDecision {
        Self::evaluate_parts(
            release,
            &self.release_rules,
            &self.custom_formats,
            &self.size_rules,
            self.min_custom_format_score,
        )
    }

    /// Evaluate without cloning the policy: lets callers pass borrowed slices
    /// straight from the runtime configuration.
    pub fn evaluate_parts(
        release: &Release,
        release_rules: &[ReleaseRule],
        custom_formats: &[CustomFormat],
        size_rules: &[SizeRule],
        min_custom_format_score: i64,
    ) -> PolicyDecision {
        let mut decision = PolicyDecision {
            allowed: true,
            ..Default::default()
        };

        // Rules, ascending by `order` (stable so equal orders keep insertion
        // order). Reject rules accumulate; score rules add to the delta.
        let mut indices: Vec<usize> = (0..release_rules.len()).collect();
        indices.sort_by_key(|index| release_rules[*index].order);
        for index in indices {
            let rule = &release_rules[index];
            if !rule.enabled {
                continue;
            }
            match &rule.action {
                RuleAction::Reject { reason } => {
                    if !rule_violation(rule, release) {
                        continue;
                    }
                    let label = if reason.trim().is_empty() {
                        rule.name.trim()
                    } else {
                        reason.trim()
                    };
                    let label = if label.is_empty() {
                        "release rule"
                    } else {
                        label
                    };
                    decision
                        .rejections
                        .push(format!("rejected by rule '{label}'"));
                    decision.violated_rules.push(rule.name.clone());
                    decision.allowed = false;
                }
                RuleAction::Score { score } => {
                    if rule_score_match(rule, release) {
                        decision.score_delta += score;
                    }
                }
            }
        }

        // Per-resolution size envelope.
        if release.size_bytes > 0 {
            if let Some(rule) = size_rule_for(&release.quality.resolution, size_rules) {
                let size_mb = release.size_bytes as f64 / 1_048_576.0;
                if rule.min_mb > 0 && size_mb < rule.min_mb as f64 {
                    decision.rejections.push(format!(
                        "size {size_mb:.0} MiB below the {} minimum of {} MiB",
                        display_resolution(&rule.resolution),
                        rule.min_mb
                    ));
                    decision.allowed = false;
                }
                if rule.max_mb > 0 && size_mb > rule.max_mb as f64 {
                    decision.rejections.push(format!(
                        "size {size_mb:.0} MiB above the {} maximum of {} MiB",
                        display_resolution(&rule.resolution),
                        rule.max_mb
                    ));
                    decision.allowed = false;
                }
            }
        }

        // Custom formats: matched scores feed the delta and the minimum gate.
        let mut total_format_score = 0i64;
        for format in custom_formats {
            if !format.enabled || !format_matches(format, release) {
                continue;
            }
            total_format_score += format.score;
            decision.matched_formats.push(format.name.clone());
        }
        decision.score_delta += total_format_score;
        if min_custom_format_score != i64::MIN && total_format_score < min_custom_format_score {
            decision.rejections.push(format!(
                "custom format score {total_format_score} below the minimum of {min_custom_format_score}"
            ));
            decision.allowed = false;
        }

        decision
    }
}

fn display_resolution(resolution: &str) -> String {
    if resolution.eq_ignore_ascii_case("any") {
        "any".into()
    } else {
        resolution.to_string()
    }
}

/// Compiles a term into a case-insensitive regex, or `None` for plain text.
fn regex_term(term: &str) -> Option<String> {
    let term = term.trim();
    if term.len() > 2 && term.starts_with('/') {
        if let Some(end) = term.rfind('/') {
            if end > 0 {
                let pattern = &term[1..end];
                let flags = &term[end + 1..];
                let mut prefix = String::new();
                if flags.contains('i') {
                    prefix.push_str("(?i)");
                }
                if flags.contains('m') {
                    prefix.push_str("(?m)");
                }
                if flags.contains('s') {
                    prefix.push_str("(?s)");
                }
                return Some(format!("{prefix}{pattern}"));
            }
        }
        // A lone leading slash is not a regex: treat it as literal text.
    }
    if term.contains('*') || term.contains('?') {
        let mut out = String::from("(?i)");
        for character in term.chars() {
            match character {
                '*' => out.push_str(".*"),
                '?' => out.push('.'),
                other => out.push_str(&regex::escape(&other.to_string())),
            }
        }
        return Some(out);
    }
    None
}

/// Compile-check a term so the API can reject a broken pattern at save time
/// instead of silently treating it as literal text later.
pub fn validate_term(term: &str) -> Result<(), String> {
    match regex_term(term) {
        Some(pattern) => crate::utils::cached_regex(&pattern)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// True when `term` is found in `haystack` (substring / wildcard / regex).
pub fn term_matches(term: &str, haystack: &str) -> bool {
    let term = term.trim();
    if term.is_empty() {
        return false;
    }
    if let Some(pattern) = regex_term(term) {
        if let Ok(regex) = crate::utils::cached_regex(&pattern) {
            return regex.is_match(haystack);
        }
    }
    haystack
        .to_ascii_lowercase()
        .contains(&term.to_ascii_lowercase())
}

/// True when a list predicate matches `value`. An empty list matches
/// everything; otherwise the value must equal or contain one entry.
fn list_matches(list: &[String], value: &str) -> bool {
    if list.is_empty() {
        return true;
    }
    let value = value.to_ascii_lowercase();
    list.iter().any(|item| {
        let item = item.trim().to_ascii_lowercase();
        !item.is_empty() && (value == item || value.contains(&item))
    })
}

fn languages_match(requested: &[String], release: &Release) -> bool {
    if requested.is_empty() {
        return true;
    }
    let mut detected: Vec<String> = release
        .quality
        .languages
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    if !release.quality.language.trim().is_empty() {
        detected.push(release.quality.language.to_ascii_lowercase());
    }
    if release.quality.is_ita {
        detected.push("ita".into());
    }
    requested.iter().any(|item| {
        let item = item.trim().to_ascii_lowercase();
        !item.is_empty() && detected.iter().any(|value| value.contains(&item) || item.contains(value))
    })
}

/// Gating predicates shared by both actions: media kind, source scope, title
/// match terms and the attribute lists. These decide whether a rule applies at
/// all; they never decide the outcome on their own.
fn rule_selectors_pass(rule: &ReleaseRule, release: &Release) -> bool {
    if !rule.media.matches(&release.kind) {
        return false;
    }
    let source_scope = rule.source.trim().to_ascii_lowercase();
    if !source_scope.is_empty() && !release.source.to_ascii_lowercase().contains(&source_scope) {
        return false;
    }
    if !rule
        .match_terms
        .iter()
        .all(|term| term_matches(term, &release.title))
    {
        return false;
    }
    if !list_matches(&rule.resolutions, &release.quality.resolution) {
        return false;
    }
    if !list_matches(&rule.sources, &release.quality.source) {
        return false;
    }
    if !list_matches(&rule.codecs, &release.quality.codec) {
        return false;
    }
    if !list_matches(&rule.audio, &release.quality.audio) {
        return false;
    }
    if !list_matches(&rule.groups, &release.quality.group) {
        return false;
    }
    if !languages_match(&rule.languages, release) {
        return false;
    }
    true
}

/// Whether a reject rule fires. A selected rule with no explicit violation
/// condition (required/ignored terms, age, size or peer bounds) rejects
/// outright; when violation conditions exist the rule fires only if one of
/// them is breached. This mirrors Sonarr/Radarr release profiles: "must
/// contain" rejects when absent, "must not contain" rejects when present.
fn rule_violation(rule: &ReleaseRule, release: &Release) -> bool {
    if !rule_selectors_pass(rule, release) {
        return false;
    }
    let mut has_condition = false;
    let mut violated = false;
    if !rule.required_terms.is_empty() {
        has_condition = true;
        if !rule
            .required_terms
            .iter()
            .any(|term| term_matches(term, &release.title))
        {
            violated = true;
        }
    }
    if !rule.except_terms.is_empty() {
        has_condition = true;
        if rule
            .except_terms
            .iter()
            .any(|term| term_matches(term, &release.title))
        {
            violated = true;
        }
    }
    if rule.max_age_days > 0 {
        has_condition = true;
        if Utc::now()
            .signed_duration_since(release.discovered_at)
            .num_days()
            > rule.max_age_days
        {
            violated = true;
        }
    }
    if rule.min_size_bytes > 0 || rule.max_size_bytes > 0 {
        // A configured size bound is a violation condition even when the
        // release size is unknown; unknown simply cannot be violated.
        has_condition = true;
        if release.size_bytes > 0 {
            let size = release.size_bytes as u64;
            if (rule.min_size_bytes > 0 && size < rule.min_size_bytes)
                || (rule.max_size_bytes > 0 && size > rule.max_size_bytes)
            {
                violated = true;
            }
        }
    }
    if rule.min_seeders.is_some() || rule.max_seeders.is_some() || rule.min_peers.is_some() {
        has_condition = true;
        if let Some(min) = rule.min_seeders {
            if release.seeders >= 0 && release.seeders < min {
                violated = true;
            }
        }
        if let Some(max) = rule.max_seeders {
            if release.seeders >= 0 && release.seeders > max {
                violated = true;
            }
        }
        if let Some(min) = rule.min_peers {
            if release.peers >= 0 && release.peers < min {
                violated = true;
            }
        }
    }
    // A plain gated reject rule (no bounds at all) fires whenever selected.
    !has_condition || violated
}

/// Whether a score rule adds its points: gated selectors plus the full
/// predicate set (required terms satisfied, ignored terms absent, bounds
/// respected, unknown values never block).
fn rule_score_match(rule: &ReleaseRule, release: &Release) -> bool {
    if !rule_selectors_pass(rule, release) {
        return false;
    }
    if !rule.required_terms.is_empty()
        && !rule
            .required_terms
            .iter()
            .any(|term| term_matches(term, &release.title))
    {
        return false;
    }
    if rule
        .except_terms
        .iter()
        .any(|term| term_matches(term, &release.title))
    {
        return false;
    }
    if rule.max_age_days > 0
        && Utc::now()
            .signed_duration_since(release.discovered_at)
            .num_days()
            > rule.max_age_days
    {
        return false;
    }
    if release.size_bytes > 0 {
        let size = release.size_bytes as u64;
        if rule.min_size_bytes > 0 && size < rule.min_size_bytes {
            return false;
        }
        if rule.max_size_bytes > 0 && size > rule.max_size_bytes {
            return false;
        }
    }
    if let Some(min) = rule.min_seeders {
        if release.seeders >= 0 && release.seeders < min {
            return false;
        }
    }
    if let Some(max) = rule.max_seeders {
        if release.seeders >= 0 && release.seeders > max {
            return false;
        }
    }
    if let Some(min) = rule.min_peers {
        if release.peers >= 0 && release.peers < min {
            return false;
        }
    }
    true
}

fn size_rule_for<'a>(resolution: &str, size_rules: &'a [SizeRule]) -> Option<&'a SizeRule> {
    size_rules
        .iter()
        .find(|rule| rule.resolution.eq_ignore_ascii_case(resolution))
        .or_else(|| {
            size_rules
                .iter()
                .find(|rule| rule.resolution.eq_ignore_ascii_case("any"))
        })
}

fn size_condition_matches(value: &str, release: &Release) -> bool {
    if release.size_bytes <= 0 {
        return false;
    }
    let size_mb = release.size_bytes as f64 / 1_048_576.0;
    let value = value.trim();
    let (min, max) = match value.split_once(':') {
        Some((min, max)) => (min.trim(), max.trim()),
        None => (value, value),
    };
    let min = min.parse::<f64>().ok().unwrap_or(0.0);
    let max = max.parse::<f64>().ok().unwrap_or(0.0);
    (min <= 0.0 || size_mb >= min) && (max <= 0.0 || size_mb <= max)
}

fn condition_matches(condition: &FormatCondition, release: &Release) -> bool {
    let quality = &release.quality;
    let raw = match condition.kind {
        ConditionKind::Title => {
            let matched = term_matches(&condition.value, &release.title);
            return apply_negate(matched, condition.negate);
        }
        // Attribute kinds match the normalised value first and fall back to the
        // raw title, so a condition written as `x265` or `WEB-DL` still matches
        // even though the parsed fields are canonicalised (`h265`, `webdl`).
        ConditionKind::Group => quality.group.clone(),
        ConditionKind::Resolution => format!("{} {}", quality.resolution, release.title),
        ConditionKind::Source => format!("{} {}", quality.source, release.title),
        ConditionKind::Codec => format!("{} {}", quality.codec, release.title),
        ConditionKind::Audio => format!("{} {}", quality.audio, release.title),
        ConditionKind::Language => {
            let mut values = quality.languages.clone();
            values.push(quality.language.clone());
            if quality.is_ita {
                values.push("ita".into());
            }
            values.join(" ")
        }
        ConditionKind::Indexer => release.source.clone(),
        ConditionKind::ReleaseType => {
            if release.is_pack {
                "pack".into()
            } else if release.kind == "movie" {
                "movie".into()
            } else if release.episode.is_some() {
                "episode".into()
            } else {
                release.kind.clone()
            }
        }
        ConditionKind::Hdr => {
            let value = condition.value.trim().to_ascii_lowercase();
            let matched = match value.as_str() {
                "" | "any" => quality.has_hdr(),
                "none" | "sdr" => !quality.has_hdr(),
                "dv" | "dolby vision" => quality.is_dv,
                "hdr" => quality.has_hdr(),
                _ => {
                    term_matches(&condition.value, &quality.hdr)
                        || (quality.is_dv && value.contains("dv"))
                }
            };
            return apply_negate(matched, condition.negate);
        }
        ConditionKind::Size => {
            let matched = size_condition_matches(&condition.value, release);
            return apply_negate(matched, condition.negate);
        }
    };
    apply_negate(term_matches(&condition.value, &raw), condition.negate)
}

fn apply_negate(matched: bool, negate: bool) -> bool {
    if negate {
        !matched
    } else {
        matched
    }
}

/// Group conditions by kind: all groups must match, and inside a group either
/// every `required` condition matches or (with no required condition) at least
/// one condition matches.
pub fn format_matches(format: &CustomFormat, release: &Release) -> bool {
    if format.conditions.is_empty() {
        return false;
    }
    let mut order: Vec<ConditionKind> = Vec::new();
    let mut groups: HashMap<ConditionKind, Vec<&FormatCondition>> = HashMap::new();
    for condition in &format.conditions {
        if !groups.contains_key(&condition.kind) {
            order.push(condition.kind);
        }
        groups.entry(condition.kind).or_default().push(condition);
    }
    for kind in order {
        let group = &groups[&kind];
        let required: Vec<&&FormatCondition> =
            group.iter().filter(|condition| condition.required).collect();
        let group_ok = if required.is_empty() {
            group
                .iter()
                .any(|condition| condition_matches(condition, release))
        } else {
            required
                .iter()
                .all(|condition| condition_matches(condition, release))
        };
        if !group_ok {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn release(title: &str) -> Release {
        let quality = crate::parser::parse_quality(title);
        Release {
            title: title.into(),
            magnet: format!("magnet:?xt=urn:btih:{}", "a".repeat(40)),
            torrent_url: None,
            source: "TestIndexer".into(),
            quality,
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2020),
            discovered_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
        }
    }

    fn term_rule(term: &str) -> ReleaseRule {
        ReleaseRule {
            name: "block cam".into(),
            enabled: true,
            match_terms: vec![term.into()],
            action: RuleAction::Reject {
                reason: "CAM rip".into(),
            },
            ..Default::default()
        }
    }

    #[test]
    fn term_matching_supports_literal_wildcard_and_regex() {
        assert!(term_matches("ita", "Movie.1080p.ITA.WEB-DL"));
        assert!(term_matches("1080*ita", "Movie.1080p.ITA.WEB-DL"));
        assert!(!term_matches("1080*ita", "Movie.1080p.ENG.WEB-DL"));
        assert!(term_matches("/1080p.*ita/i", "Movie.1080p.ITA.WEB-DL"));
        assert!(!term_matches("/^ita/i", "Movie.1080p.ITA.WEB-DL"));
        // A dangling slash is literal text, not a broken regex.
        assert!(!term_matches("/", "Movie.1080p.ITA.WEB-DL"));
    }

    #[test]
    fn reject_rule_fires_and_reports_its_reason() {
        let policy = Policy {
            release_rules: vec![term_rule("CAM")],
            ..Default::default()
        };
        let decision = policy.evaluate(&release("Movie.2020.CAM.1080p"));
        assert!(!decision.allowed);
        assert!(decision.reason().unwrap().contains("CAM rip"));
        assert!(policy.evaluate(&release("Movie.2020.1080p.WEB-DL")).allowed);
    }

    #[test]
    fn required_terms_need_at_least_one() {
        let rule = ReleaseRule {
            name: "needs italian".into(),
            enabled: true,
            required_terms: vec!["ITA".into(), "MULTI".into()],
            action: RuleAction::Reject {
                reason: "no Italian audio".into(),
            },
            ..Default::default()
        };
        let policy = Policy {
            release_rules: vec![rule],
            ..Default::default()
        };
        assert!(policy.evaluate(&release("Movie.1080p.ENG")).allowed == false);
        assert!(policy.evaluate(&release("Movie.1080p.ITA")).allowed);
        assert!(policy.evaluate(&release("Movie.1080p.MULTI")).allowed);
    }

    #[test]
    fn score_rules_sum_into_the_delta() {
        let policy = Policy {
            release_rules: vec![
                ReleaseRule {
                    name: "prefer x265".into(),
                    enabled: true,
                    match_terms: vec!["x265".into()],
                    action: RuleAction::Score { score: 300 },
                    ..Default::default()
                },
                ReleaseRule {
                    name: "penalize cam".into(),
                    enabled: true,
                    match_terms: vec!["CAM".into()],
                    action: RuleAction::Score { score: -500 },
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let decision = policy.evaluate(&release("Movie.2020.1080p.x265.WEB-DL"));
        assert_eq!(decision.score_delta, 300);
        let decision = policy.evaluate(&release("Movie.2020.CAM.x265"));
        assert_eq!(decision.score_delta, -200);
    }

    #[test]
    fn media_scope_limits_rules() {
        let mut rule = term_rule("CAM");
        rule.media = MediaScope::Series;
        let policy = Policy {
            release_rules: vec![rule],
            ..Default::default()
        };
        // A movie release is not affected by a series-only rule.
        assert!(policy.evaluate(&release("Movie.2020.CAM.1080p")).allowed);
    }

    #[test]
    fn size_rule_rejects_out_of_envelope() {
        let policy = Policy {
            size_rules: vec![SizeRule {
                resolution: "1080p".into(),
                min_mb: 1000,
                max_mb: 15000,
            }],
            ..Default::default()
        };
        let mut small = release("Movie.2020.1080p.WEB-DL");
        small.size_bytes = 500 * 1_048_576;
        assert!(!policy.evaluate(&small).allowed);
        let mut ok = release("Movie.2020.1080p.WEB-DL");
        ok.size_bytes = 4000 * 1_048_576;
        assert!(policy.evaluate(&ok).allowed);
        let mut huge = release("Movie.2020.1080p.WEB-DL");
        huge.size_bytes = 30_000 * 1_048_576;
        assert!(!policy.evaluate(&huge).allowed);
        // Unknown size is never rejected by size rules on its own.
        assert!(policy.evaluate(&release("Movie.2020.1080p.WEB-DL")).allowed);
    }

    #[test]
    fn any_size_rule_is_a_fallback() {
        let policy = Policy {
            size_rules: vec![
                SizeRule {
                    resolution: "2160p".into(),
                    min_mb: 5000,
                    max_mb: 0,
                },
                SizeRule {
                    resolution: "any".into(),
                    min_mb: 100,
                    max_mb: 0,
                },
            ],
            ..Default::default()
        };
        let mut hd = release("Movie.2020.1080p.WEB-DL");
        hd.size_bytes = 50 * 1_048_576;
        assert!(!policy.evaluate(&hd).allowed);
        let mut uhd = release("Movie.2020.2160p.WEB-DL");
        uhd.size_bytes = 6000 * 1_048_576;
        assert!(policy.evaluate(&uhd).allowed);
        let mut uhd_small = release("Movie.2020.2160p.WEB-DL");
        uhd_small.size_bytes = 1000 * 1_048_576;
        assert!(!policy.evaluate(&uhd_small).allowed);
    }

    #[test]
    fn custom_format_groups_or_within_kind_and_and_across_kinds() {
        // 1080p AND (x265 OR x264) AND group NTb.
        let format = CustomFormat {
            name: "Italian x265".into(),
            enabled: true,
            score: 500,
            conditions: vec![
                FormatCondition {
                    kind: ConditionKind::Resolution,
                    value: "1080p".into(),
                    ..Default::default()
                },
                FormatCondition {
                    kind: ConditionKind::Codec,
                    value: "x265".into(),
                    ..Default::default()
                },
                FormatCondition {
                    kind: ConditionKind::Codec,
                    value: "h265".into(),
                    ..Default::default()
                },
                FormatCondition {
                    kind: ConditionKind::Group,
                    value: "NTb".into(),
                    ..Default::default()
                },
            ],
        };
        let policy = Policy {
            custom_formats: vec![format],
            ..Default::default()
        };
        assert!(policy
            .evaluate(&release("Movie.2020.1080p.x265-NTb"))
            .allowed);
        assert_eq!(
            policy
                .evaluate(&release("Movie.2020.1080p.x265-NTb"))
                .score_delta,
            500
        );
        // Wrong group -> format does not match.
        assert_eq!(
            policy
                .evaluate(&release("Movie.2020.1080p.x265-OTHER"))
                .score_delta,
            0
        );
        // Wrong resolution -> format does not match.
        assert_eq!(
            policy
                .evaluate(&release("Movie.2020.720p.x265-NTb"))
                .score_delta,
            0
        );
    }

    #[test]
    fn required_condition_makes_its_group_mandatory() {
        let format = CustomFormat {
            name: "x265 with italian".into(),
            enabled: true,
            score: 100,
            conditions: vec![
                FormatCondition {
                    kind: ConditionKind::Codec,
                    value: "x265".into(),
                    ..Default::default()
                },
                FormatCondition {
                    kind: ConditionKind::Language,
                    value: "ita".into(),
                    required: true,
                    ..Default::default()
                },
            ],
        };
        let policy = Policy {
            custom_formats: vec![format],
            ..Default::default()
        };
        // x265 but no Italian: the required language condition fails the group.
        assert_eq!(
            policy.evaluate(&release("Movie.2020.1080p.x265.ENG")).score_delta,
            0
        );
        assert_eq!(
            policy.evaluate(&release("Movie.2020.1080p.x265.ITA")).score_delta,
            100
        );
    }

    #[test]
    fn negated_condition_excludes() {
        let format = CustomFormat {
            name: "no cam".into(),
            enabled: true,
            score: 50,
            conditions: vec![FormatCondition {
                kind: ConditionKind::Title,
                value: "CAM".into(),
                negate: true,
                ..Default::default()
            }],
        };
        let policy = Policy {
            custom_formats: vec![format],
            ..Default::default()
        };
        assert_eq!(policy.evaluate(&release("Movie.2020.CAM")).score_delta, 0);
        assert_eq!(
            policy.evaluate(&release("Movie.2020.WEB-DL")).score_delta,
            50
        );
    }

    #[test]
    fn minimum_custom_format_score_rejects() {
        let policy = Policy {
            custom_formats: vec![CustomFormat {
                name: "x265".into(),
                enabled: true,
                score: 10,
                conditions: vec![FormatCondition {
                    kind: ConditionKind::Codec,
                    value: "x265".into(),
                    ..Default::default()
                }],
            }],
            min_custom_format_score: 100,
            ..Default::default()
        };
        let decision = policy.evaluate(&release("Movie.2020.1080p.x265"));
        assert!(!decision.allowed);
        assert!(decision.reason().unwrap().contains("below the minimum"));
        // The disabled sentinel never rejects.
        let disabled = Policy {
            custom_formats: vec![],
            min_custom_format_score: i64::MIN,
            ..Default::default()
        };
        assert!(disabled.evaluate(&release("Movie.2020.1080p")).allowed);
    }

    #[test]
    fn seeders_and_size_predicates_ignore_unknown_values() {
        let rule = ReleaseRule {
            name: "enough seeders".into(),
            enabled: true,
            min_seeders: Some(5),
            source: "indexer".into(),
            action: RuleAction::Reject {
                reason: "too few seeders".into(),
            },
            ..Default::default()
        };
        let policy = Policy {
            release_rules: vec![rule],
            ..Default::default()
        };
        let mut release = release("Movie.2020.1080p");
        release.source = "Indexer A".into();
        release.seeders = -1;
        assert!(policy.evaluate(&release).allowed);
        release.seeders = 2;
        assert!(!policy.evaluate(&release).allowed);
        release.seeders = 10;
        assert!(policy.evaluate(&release).allowed);
    }

    #[test]
    fn size_condition_parses_ranges() {
        let mut release = release("Movie.2020.1080p");
        release.size_bytes = 2000 * 1_048_576;
        assert!(size_condition_matches("1000:3000", &release));
        assert!(!size_condition_matches("3000:5000", &release));
        assert!(size_condition_matches(":3000", &release));
        assert!(size_condition_matches("1000:", &release));
        assert!(size_condition_matches("2000", &release));
        // Unknown size never matches.
        release.size_bytes = 0;
        assert!(!size_condition_matches("1000:", &release));
    }

    #[test]
    fn unknown_size_does_not_trigger_a_size_reject_rule() {
        let rule = ReleaseRule {
            name: "too small".into(),
            enabled: true,
            min_size_bytes: 1_000_000_000,
            action: RuleAction::Reject {
                reason: "under 1 GB".into(),
            },
            ..Default::default()
        };
        let policy = Policy {
            release_rules: vec![rule],
            ..Default::default()
        };
        // Unknown size: the rule is configured but cannot be violated, so the
        // release is not rejected.
        let unknown = release("Movie.2020.1080p.WEB-DL");
        assert!(unknown.size_bytes == 0);
        assert!(policy.evaluate(&unknown).allowed);
        // Known small size: rejected.
        let mut small = release("Movie.2020.1080p.WEB-DL");
        small.size_bytes = 100 * 1_048_576;
        assert!(!policy.evaluate(&small).allowed);
        // Known sufficient size: allowed.
        let mut large = release("Movie.2020.1080p.WEB-DL");
        large.size_bytes = 4_000 * 1_048_576;
        assert!(policy.evaluate(&large).allowed);
    }

    #[test]
    fn default_policy_is_permissive() {
        let policy = Policy::default();
        let decision = policy.evaluate(&release("Anything.2020.1080p.x265"));
        assert!(decision.allowed);
        assert_eq!(decision.score_delta, 0);
        assert!(decision.rejections.is_empty());
    }
}
