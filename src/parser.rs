use crate::models::{Quality, Release};
use crate::utils::sanitize_magnet;
use chrono::Utc;

/// Combina due `Quality`: usa i campi di `primary` quando valorizzati, altrimenti
/// quelli di `fallback`. Serve a recuperare sorgente/codec/ecc. dal titolo
/// originale della release quando il nome file attuale li ha persi.
pub fn merge_quality(primary: Quality, fallback: Quality) -> Quality {
    fn pick(primary: &str, fallback: &str) -> String {
        if primary.trim().is_empty() || primary.eq_ignore_ascii_case("unknown") {
            fallback.to_string()
        } else {
            primary.to_string()
        }
    }
    Quality {
        resolution: pick(&primary.resolution, &fallback.resolution),
        source: pick(&primary.source, &fallback.source),
        codec: pick(&primary.codec, &fallback.codec),
        audio: pick(&primary.audio, &fallback.audio),
        hdr: pick(&primary.hdr, &fallback.hdr),
        group: pick(&primary.group, &fallback.group),
        is_ita: primary.is_ita || fallback.is_ita,
        is_dv: primary.is_dv || fallback.is_dv,
        is_repack: primary.is_repack || fallback.is_repack,
        is_proper: primary.is_proper || fallback.is_proper,
        is_real: primary.is_real || fallback.is_real,
        language: pick(&primary.language, &fallback.language),
        languages: if primary.languages.is_empty() {
            fallback.languages
        } else {
            primary.languages
        },
        has_subtitle: primary.has_subtitle || fallback.has_subtitle,
        subtitle_languages: if primary.subtitle_languages.is_empty() {
            fallback.subtitle_languages
        } else {
            primary.subtitle_languages
        },
    }
}

pub fn normalize_series_name(value: &str) -> String {
    // Memoised: series matching normalises every configured series name for
    // every candidate release (series × releases per cycle). The function is
    // pure, so results are safe to reuse. Bounded to keep memory in check.
    static CACHE: std::sync::LazyLock<
        std::sync::RwLock<std::collections::HashMap<String, String>>,
    > = std::sync::LazyLock::new(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    if let Some(hit) = CACHE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(value)
    {
        return hit.clone();
    }
    let normalized = value
        .to_ascii_lowercase()
        .replace(['.', '_', '-', '/', '\\'], " ");
    let normalized = normalized.replace("'s", "");
    let normalized = normalized
        .split_whitespace()
        .filter(|w| {
            !matches!(
                *w,
                "the" | "a" | "an" | "il" | "lo" | "la" | "i" | "gli" | "le" | "un" | "una"
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut cache = CACHE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() < 50_000 {
        cache.insert(value.to_owned(), normalized.clone());
    }
    normalized
}

pub fn series_names_match(a: &str, b: &str) -> bool {
    let left = strip_year_tokens(&normalize_series_name(a));
    let right = strip_year_tokens(&normalize_series_name(b));
    if tokens_match(&left, &right) {
        return true;
    }
    // Titoli "stilizzati" (es. `PLUR1BUS` per *Pluribus*): confronta una
    // variante con le cifre sciolte in lettere, ma solo quando una sola delle
    // due parti contiene cifre. Così non si confondono titoli numerici come
    // `9-1-1` e non si altera il confronto tra nomi già uguali.
    if has_ascii_digit(&left) != has_ascii_digit(&right) {
        let folded_left = leet_fold(&left);
        let folded_right = leet_fold(&right);
        if tokens_match(&folded_left, &folded_right) {
            return true;
        }
    }
    false
}

/// Confronto di due nomi già normalizzati: uguaglianza oppure token a token
/// con tolleranza per il plurale finale (`Show`/`Shows`).
fn tokens_match(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let left = left.split_whitespace().collect::<Vec<_>>();
    let right = right.split_whitespace().collect::<Vec<_>>();
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| {
            *a == b || a.strip_suffix('s') == Some(b) || b.strip_suffix('s') == Some(a)
        })
}

/// Rimuove i token che sono solo un anno racchiuso in parentesi/quadre
/// (`(2025)`, `[2024]`), frequenti nei nomi file legacy `SERIE (ANNO) - S01E01`.
/// Se la rimozione svuoterebbe il nome (serie intitolate a un anno, es. `1923`)
/// il valore resta invariato: un anno "nudo" non viene toccato.
fn strip_year_tokens(value: &str) -> String {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    let kept = tokens
        .iter()
        .copied()
        .filter(|token| !is_year_token(token))
        .collect::<Vec<_>>();
    if kept.is_empty() {
        value.to_string()
    } else {
        kept.join(" ")
    }
}

fn is_year_token(token: &str) -> bool {
    let wrapped = token.starts_with('(')
        || token.starts_with('[')
        || token.ends_with(')')
        || token.ends_with(']');
    if !wrapped {
        return false;
    }
    let trimmed = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    trimmed.len() == 4
        && (trimmed.starts_with("19") || trimmed.starts_with("20"))
        && trimmed.chars().all(|c| c.is_ascii_digit())
}

fn has_ascii_digit(value: &str) -> bool {
    value.chars().any(|c| c.is_ascii_digit())
}

/// Sostituisce le cifre usate come lettere nei titoli leet:
/// `1`→`i`, `0`→`o`, `3`→`e`, `4`→`a`, `5`→`s`, `7`→`t`, `6`/`9`→`g`, `8`→`b`.
fn leet_fold(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' => 'i',
            '3' => 'e',
            '4' => 'a',
            '5' => 's',
            '6' => 'g',
            '7' => 't',
            '8' => 'b',
            '9' => 'g',
            other => other,
        })
        .collect()
}

/// Chiave `(nome serie normalizzato, stagione, episodio)` da un nome file o
/// titolo di release. Usata per confrontare le release coi torrent già attivi
/// nella sessione.
pub fn parse_episode_key(name: &str) -> Option<(String, i64, i64)> {
    let pattern = crate::utils::cached_regex(
        r"(?i)^(?P<name>.+?)[ ._-]+(?:s(?P<s>\d{1,2})e|(?P<ns>\d{1,2})x)(?P<e>\d{1,4})",
    )
    .ok()?;
    let captures = pattern.captures(name)?;
    let series = normalize_series_name(captures.name("name")?.as_str());
    let season = captures
        .name("s")
        .or_else(|| captures.name("ns"))?
        .as_str()
        .parse::<i64>()
        .ok()?;
    let episode = captures.name("e")?.as_str().parse::<i64>().ok()?;
    Some((series, season, episode))
}

pub fn parse_quality(title: &str) -> Quality {
    let low = title.to_lowercase();
    // legacy normalisations: `t_norm` replaces [._-] with spaces; `t_norm_lang`
    // also removes square brackets so renamed files like "[DV HDR10][IT][h265]"
    // are recognised.
    let t_norm_lang = crate::utils::cached_regex(r"[._ \-\[\]]")
        .unwrap()
        .replace_all(&low, " ")
        .to_string();

    // --- Riscrittura dei marker di sottotitoli prima del rilevamento lingua ---
    // Il feed può riportare "ENG ... Sub ita ..." anche quando l'audio è solo
    // inglese: se non rimossi, "sub ita" verrebbe letto come audio italiano.
    let subtitle_marker = r"(?:subs?|subforced|subtitles?|forced|sdh|cc|closedcaptions?)";
    let subtitle_sep = r"[\s._\-/\\\[\]()+|,:;]*";
    let subtitle_lang = r"(?:[a-z]{2,3}|english|italian|italiano|german|deutsch|french|francais|spanish|portuguese|japanese|chinese|korean|russian)";
    let subtitle_tags = format!(
        r"\b{subtitle_marker}{subtitle_sep}(?:{subtitle_lang}{subtitle_sep})*(?:it|ita|italian|italiano)\b|\b(?:it|ita|italian|italiano){subtitle_sep}{subtitle_marker}\b"
    );
    let re_subtitle = crate::utils::cached_regex(&subtitle_tags).unwrap();
    let t_norm_lang_audio = re_subtitle.replace_all(&t_norm_lang, " ").to_string();
    let t_audio = re_subtitle.replace_all(&low, " ").to_string();

    // Tag dei servizi streaming che contengono "it"/"nf" e generano falsi
    // positivi (iT = iTunes, NF = Netflix). Il crate `regex` non supporta i
    // look-ahead, quindi "it web" viene sostituito con il solo " web".
    let strip_streaming = |text: &str| -> String {
        let without_it_web = crate::utils::cached_regex(r"(?i)\b(it|nf)(\s+web)\b")
            .unwrap()
            .replace_all(text, "$2")
            .to_string();
        crate::utils::cached_regex(r"(?i)\bitunes\b|\bamzn\b|\bdsnp\b|\bhmax\b|\bparamount\b")
            .unwrap()
            .replace_all(&without_it_web, " ")
            .to_string()
    };

    // --- Lingua italiana: tre livelli, dal più sicuro al più specifico ---
    let level1 = crate::utils::cached_regex(r"\bita\b|\bitalian\b|\bitaliano\b")
        .unwrap()
        .is_match(&strip_streaming(&t_norm_lang_audio));
    let level2 = crate::utils::cached_regex(r"\bit[\+\|]|\[it\]").unwrap().is_match(&t_audio);
    let ita = if level1 || level2 {
        true
    } else {
        // Livello 3: "\bit\b" solo dalla parte tecnica in poi, per non colpire
        // parole del titolo come "It Chapter" o "Feel It Still".
        let resolution = crate::utils::cached_regex(
            r"\b(2160p?|1080p?|720p?|480p?|4k|uhd|bluray|web[\s\-]?dl|webrip|hdtv)\b",
        )
        .unwrap();
        match resolution.find(&t_norm_lang_audio) {
            Some(found) => {
                let tech = strip_streaming(&t_norm_lang_audio[found.start()..]);
                crate::utils::cached_regex(r"\bit\b").unwrap().is_match(&tech)
                    && !crate::utils::cached_regex(
                        r"\bwith\b|\bbit\b|\bsplit\b|\bedit\b|\bunit\b|\bvisit\b|\blimit\b|\bexit\b|\bprofit\b|\bsubmit\b|\bcommit\b|\bpermit\b|\badmit\b|\bomit\b|\bhit\b|\bkit\b|\bpit\b|\bsit\b|\bfit\b|\bwit\b|\bknit\b|\bspit\b|\bslit\b|\bitunes\b",
                    )
                    .unwrap()
                    .is_match(&tech)
            }
            None => false,
        }
    };

    let subtitle_ita = crate::utils::cached_regex(
        r"(?i)(sub|subs|subtitle|subtitles)[._ \-\[\]\(\)+]*(ita|italian|it)([._ \-\[\]\(\)+]|$)",
    )
    .unwrap()
    .is_match(title);
    let subtitle_eng = crate::utils::cached_regex(
        r"(?i)(sub|subs|subtitle|subtitles)[._ \-\[\]\(\)+]*(eng|english|en)([._ \-\[\]\(\)+]|$)",
    )
    .unwrap()
    .is_match(title);
    let eng = !subtitle_eng
        && crate::utils::cached_regex(r"(?i)(^|[._ \-\[\]\(\)+])(eng|english|en)([._ \-\[\]\(\)+]|$)")
            .unwrap()
            .is_match(title);
    let mut languages = Vec::new();
    if ita {
        languages.push("ita".into());
    }
    if eng {
        languages.push("eng".into());
    }
    let mut subtitle_languages = Vec::new();
    if subtitle_ita {
        subtitle_languages.push("ita".into());
    }
    if subtitle_eng {
        subtitle_languages.push("eng".into());
    }

    // --- Risoluzione ---
    let resolution = if low.contains("2160p") || low.contains("4k") || low.contains("uhd") {
        "2160p"
    } else if low.contains("1080p") || low.contains("fullhd") {
        "1080p"
    } else if low.contains("720p") || low.contains("hd") {
        "720p"
    } else if low.contains("576p") || low.contains("pal") {
        "576p"
    } else if low.contains("480p") || low.contains("ntsc") {
        "480p"
    } else {
        "unknown"
    };

    // --- Sorgente ---
    let source = if low.contains("bluray") || low.contains("bdrip") || low.contains("brrip") {
        "bluray"
    } else if low.contains("web-dl") || low.contains("webdl") || low.contains("web") {
        "webdl"
    } else if low.contains("webrip") {
        "webrip"
    } else if low.contains("hdtv") || low.contains("hdtvrip") {
        "hdtv"
    } else if low.contains("dvdrip") || low.contains("dvd") {
        "dvdrip"
    } else {
        "unknown"
    };

    // --- HDR / Dolby Vision ---
    let padded_lang = format!(" {t_norm_lang} ");
    let is_dv = padded_lang.contains(" dv ")
        || t_norm_lang.contains("dovi")
        || t_norm_lang.contains("dolby vision");
    let hdr = if is_dv {
        "DV".to_string()
    } else if t_norm_lang.contains("hdr10+")
        || t_norm_lang.contains("hdr10plus")
        || t_norm_lang.contains("hdr10 plus")
    {
        "HDR10Plus".to_string()
    } else if t_norm_lang.contains("hdr10") {
        "HDR10".to_string()
    } else if crate::utils::cached_regex(r"\bhdr\b|\bhlg\b")
        .unwrap()
        .is_match(&t_norm_lang)
    {
        "HDR".to_string()
    } else {
        String::new()
    };

    // --- Codec ---
    let codec = if low.contains("x265")
        || low.contains("hevc")
        || low.contains("h.265")
        || padded_lang.contains(" h265 ")
    {
        "h265"
    } else if low.contains("x264")
        || low.contains("avc")
        || low.contains("h.264")
        || padded_lang.contains(" h264 ")
    {
        "h264"
    } else {
        "unknown"
    };

    // --- Audio (ordine di priorità legacy) ---
    let audio = if low.contains("dts-hd") || low.contains("dtshd") {
        "dts-hd"
    } else if low.contains("dts") {
        "dts"
    } else if low.contains("ddp5.1") || low.contains("ddp 5.1") || low.contains("eac3") {
        "ddp"
    } else if low.contains("ac3") || low.contains("dd5.1") {
        "ac3"
    } else if low.contains("5.1") {
        "5.1"
    } else if low.contains("mp3") {
        "mp3"
    } else if low.contains("aac") {
        "aac"
    } else {
        "unknown"
    };

    // --- Gruppo ---
    let group = crate::utils::cached_regex(r"[-]([a-z0-9]+)$|\[([a-z0-9]+)\]$")
        .unwrap()
        .captures(&low)
        .and_then(|capture| capture.get(1).or_else(|| capture.get(2)))
        .map(|value| value.as_str().to_owned())
        .unwrap_or_else(|| "unknown".into());

    Quality {
        resolution: resolution.into(),
        source: source.into(),
        codec: codec.into(),
        audio: audio.into(),
        hdr,
        group,
        is_ita: ita,
        is_dv,
        is_repack: low.contains("repack") || low.contains("rerip"),
        is_proper: low.contains("proper"),
        is_real: crate::utils::cached_regex(r"\breal\b").unwrap().is_match(&low),
        language: if ita {
            "ita".into()
        } else if eng {
            "eng".into()
        } else {
            "unknown".into()
        },
        languages,
        has_subtitle: crate::utils::cached_regex(
            r"(?i)(^|[._ \-\[\]\(\)+])(sub|subs|subtitle|subtitles)([._ \-\[\]\(\)+]|$)",
        )
        .unwrap()
        .is_match(title),
        subtitle_languages,
    }
}

/// legacy `Parser.parse_movie` gate: returns false when the release is clearly
/// not a movie (an episode, season pack, wrestling/sport, magazine, videogame,
/// console ROM, or a music release with a genre prefix).
pub fn passes_movie_filter(title: &str) -> bool {
    if title.is_empty() {
        return false;
    }
    let matched = |pattern: &str| -> bool {
        crate::utils::cached_regex(pattern)
            .map(|regex| regex.is_match(title))
            .unwrap_or(false)
    };
    if matched(r"(?i)[Ss]\d{1,2}[Ee]\d{1,2}") {
        return false;
    }
    if matched(r"(?i)\bStagion[ei]\b|\bSeason[ ._-]?\d|\bComplete[ ._-]?S\d+|\bCOMPLETA\b") {
        return false;
    }
    if matched(r"(?i)\bWWE\b|\bAEW\b|\bTNA\b|\bWWF\b|\bROH\b|\bImpact\s+Wrestling\b") {
        return false;
    }
    if matched(r"(?i)\bMotoGP\b|\bMotoE\b|\bFormula\s*E?\b|\bNASCAR\b|\bSuperBike\b") {
        return false;
    }
    if matched(r"\b\d{4}x\d{2,3}\b") {
        return false;
    }
    if matched(r"(?i)\bMagazine\b|\bRivista\b") {
        return false;
    }
    if matched(
        r"(?i)\bDLCs?\b|\bPortable\b|Build[ ._-]\d{5,}|\bv20\d\d[._]\d{2}[._]\d{2}\b|\bGameDrive\b|\bHypervisor\b|\bDenuvO\b",
    ) {
        return false;
    }
    if matched(
        r"(?i)PlayStation[ ._-]+\d|Nintendo[ ._-]+(DS|3DS|64|Switch|Wii)|\bN64\b|\bGBA\b|\bNDS\b|\bPSX\b|\bPS[123]\b",
    ) {
        return false;
    }
    let has_video = matched(
        r"(?i)\b(2160p|1080p|720p|576p|480p|4[Kk]|UHD|BluRay|BDRip|WEB-DL|WEBRip|HDTV|DVDRip|DVDScr)\b",
    );
    if !has_video {
        if matched(r"^\s*\([A-Za-zÀ-ÿ][A-Za-zÀ-ÿ0-9\s,/&-]{2,}\)\s+\S") {
            return false;
        }
        if matched(r"(?i)\bbootleg\b") {
            return false;
        }
    }
    true
}

pub fn parse_release(title: &str, magnet: &str, source: &str) -> Option<Release> {
    parse_release_at(title, magnet, source, Utc::now())
}

pub fn parse_release_at(
    title: &str,
    magnet: &str,
    source: &str,
    discovered_at: chrono::DateTime<Utc>,
) -> Option<Release> {
    parse_release_source(title, magnet, None, source, discovered_at)
}

/// True per un link HTTP(S) che punta a un file `.torrent`.
pub fn is_torrent_url(value: &str) -> bool {
    let value = value.trim();
    (value.starts_with("http://") || value.starts_with("https://"))
        && value
            .split(['?', '#'])
            .next()
            .unwrap_or(value)
            .to_ascii_lowercase()
            .ends_with(".torrent")
}

/// Come [`parse_release_at`], ma accetta anche un link `.torrent` diretto: molti
/// feed RSS (es. TorrentLeech) non espongono un magnet, solo il download del
/// `.torrent`. Una release è valida se ha almeno un magnet **o** un link.
pub fn parse_release_source(
    title: &str,
    magnet: &str,
    torrent_url: Option<&str>,
    source: &str,
    discovered_at: chrono::DateTime<Utc>,
) -> Option<Release> {
    // Alcune fonti (es. BTDigg) usano un magnet troncato come testo del link:
    // non è un titolo valido, scartalo prima di creare una release.
    if title.trim().to_ascii_lowercase().starts_with("magnet:") {
        return None;
    }
    let torrent_url = torrent_url
        .map(str::trim)
        .filter(|value| is_torrent_url(value))
        .map(str::to_owned);
    let magnet = match sanitize_magnet(magnet, Some(title)) {
        Some(value) => value,
        None if torrent_url.is_none() => return None,
        None => String::new(),
    };
    // Ordine di riconoscimento come legacy: range, multi-episodio concatenato,
    // SxxExx singolo, NxNN, data (YYYY-MM-DD), stagione completa.
    let range_re =
        crate::utils::cached_regex(r"(?i)^(.+?)[ ._-]+s(\d{1,2})e(\d{1,4})[-–]e?(\d{1,4})(?:[ ._-]|$)").unwrap();
    let multi_re = crate::utils::cached_regex(r"(?i)^(.+?)[ ._-]+s(\d{1,2})((?:e\d{1,4}){2,})(?:[ ._-]|$)").unwrap();
    let standard_re = crate::utils::cached_regex(r"(?i)^(.+?)[ ._-]+s(\d{1,2})e(\d{1,4})(?:[ ._-]|$)").unwrap();
    let nx = crate::utils::cached_regex(r"(?i)^(.+?)[ ._-]+(\d{1,2})x(\d{1,4})(?:[ ._-]|$)").unwrap();
    let date_re =
        crate::utils::cached_regex(r"(?i)^(.+?)[ ._-]+(\d{4})[-.](\d{1,2})[-.](\d{1,2})(?:[ ._-]|$)").unwrap();
    let season_pack = crate::utils::cached_regex(r"(?i)^(.+?)[ ._-]+(?:s|season[ ._-]?)(\d{1,2})(?:[ ._-]+(?:complete|completa))?(?:[ ._-]|$)").unwrap();
    let series_name = |capture: &regex::Captures<'_>| -> Option<String> {
        Some(capture.get(1)?.as_str().replace('.', " ").trim().to_owned())
    };
    let (series, season, episode, range) = if let Some(c) = range_re.captures(title) {
        let season: i64 = c.get(2)?.as_str().parse().ok()?;
        let episode: i64 = c.get(3)?.as_str().parse().ok()?;
        let end = c
            .get(4)
            .and_then(|value| value.as_str().parse::<i64>().ok())
            .filter(|end| *end >= episode)
            .unwrap_or(episode);
        (series_name(&c), Some(season), Some(episode), (episode..=end).collect())
    } else if let Some(c) = multi_re.captures(title) {
        let season: i64 = c.get(2)?.as_str().parse().ok()?;
        let joined = c.get(3)?.as_str();
        let episodes: Vec<i64> = crate::utils::cached_regex(r"(?i)e(\d{1,4})")
            .unwrap()
            .captures_iter(joined)
            .filter_map(|capture| capture.get(1)?.as_str().parse::<i64>().ok())
            .filter(|episode| *episode <= 99)
            .collect();
        if episodes.len() < 2 {
            (None, None, None, Vec::new())
        } else {
            (
                series_name(&c),
                Some(season),
                episodes.first().copied(),
                episodes,
            )
        }
    } else if let Some(c) = standard_re.captures(title) {
        let season: i64 = c.get(2)?.as_str().parse().ok()?;
        let episode: i64 = c.get(3)?.as_str().parse().ok()?;
        (series_name(&c), Some(season), Some(episode), vec![episode])
    } else if let Some(c) = nx.captures(title) {
        let season: i64 = c.get(2)?.as_str().parse().ok()?;
        if !(1..=40).contains(&season) {
            (None, None, None, Vec::new())
        } else {
            let episode: i64 = c.get(3)?.as_str().parse().ok()?;
            if episode > 99 {
                (None, None, None, Vec::new())
            } else {
                (series_name(&c), Some(season), Some(episode), vec![episode])
            }
        }
    } else if let Some(c) = date_re.captures(title) {
        let year: i32 = c.get(2)?.as_str().parse().ok()?;
        let month: u32 = c.get(3)?.as_str().parse().ok()?;
        let day: u32 = c.get(4)?.as_str().parse().ok()?;
        match chrono::NaiveDate::from_ymd_opt(year, month, day) {
            Some(date) => {
                use chrono::Datelike;
                let ordinal = date.ordinal() as i64;
                (
                    series_name(&c),
                    Some(year as i64),
                    Some(ordinal),
                    vec![ordinal],
                )
            }
            None => (None, None, None, Vec::new()),
        }
    } else if let Some(c) = season_pack.captures(title) {
        let season: i64 = c.get(2)?.as_str().parse().ok()?;
        (series_name(&c), Some(season), Some(0), vec![0])
    } else {
        (None, None, None, Vec::new())
    };
    let kind = if season.is_some() { "series" } else { "movie" };
    let year = crate::utils::cached_regex(r"\b(19\d{2}|20\d{2})\b")
        .unwrap()
        .captures(title)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok());
    Some(Release {
        title: title.into(),
        magnet,
        torrent_url,
        source: source.into(),
        quality: parse_quality(title),
        kind: kind.into(),
        series,
        season,
        episode,
        is_pack: range.len() > 1 || episode == Some(0),
        episode_range: range,
        year,
        discovered_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAGNET: &str = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";

    #[test]
    fn parses_partial_season_pack() {
        let release =
            parse_release("Example.Show.S02E01-05.1080p.WEB-DL.ITA", MAGNET, "test").unwrap();
        assert_eq!(release.series.as_deref(), Some("Example Show"));
        assert_eq!(release.season, Some(2));
        assert_eq!(release.episode_range, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn parses_complete_season_pack_variants() {
        for title in [
            "Breaking.Bad.Season.5.1080p.ITA",
            "Succession.S03.2160p.ITA",
        ] {
            let release = parse_release(title, MAGNET, "test").unwrap();
            assert!(release.is_pack, "{title}");
            assert_eq!(release.episode_range, vec![0], "{title}");
        }
    }

    #[test]
    fn parses_renamed_bracket_quality_tags_without_false_language_matches() {
        let quality = parse_quality("The Pitt - S01E01 - Pilot [1080p][DV HDR10][h265][IT+EN].mkv");
        assert_eq!(quality.resolution, "1080p");
        assert_eq!(quality.codec, "h265");
        assert!(quality.is_dv);
        assert_eq!(quality.languages, vec!["ita", "eng"]);
        assert!(!parse_quality("Series.S01E01.1080p.WEB-DL.with.subtitles").is_ita);
        assert!(!parse_quality("Series.S01E01.1080p.WEB-DL.ENG.sub[IT]").is_ita);
        let subtitles = parse_quality("Movie.2024.1080p.WEB-DL.SUB.ITA.SUB.ENG");
        assert_eq!(subtitles.subtitle_languages, vec!["ita", "eng"]);
        assert!(subtitles.languages.is_empty());
    }

    #[test]
    fn rejects_magnet_truncated_titles() {
        // BTDigg a volte usa un magnet troncato come testo del link: non è un
        // titolo valido e non deve produrre una release.
        assert!(parse_release(
            "magnet:?xt=urn:btih:0006c977cb45...",
            "magnet:?xt=urn:btih:0006c977cb45abcdefabcdefabcdefabcdefabcdef",
            "BTDigg"
        )
        .is_none());
    }

    #[test]
    fn merges_source_from_original_release_title() {
        // Il nome file ha perso la sorgente; il titolo originale la conserva.
        let file = parse_quality("Show - S01E01 - Titolo - [1080p][h264][AAC][IT].mkv");
        let original = parse_quality("Show.S01E01.1080p.WEB-DL.H.264.ITA.AAC");
        assert_eq!(file.source, "unknown");
        let merged = merge_quality(original, file);
        assert_eq!(merged.source, "webdl");
        assert_eq!(merged.resolution, "1080p");
        assert_eq!(merged.codec, "h264");
    }

    #[test]
    fn rejects_codec_as_nx_episode() {
        let release = parse_release("Example.Show.20x265.1080p.WEB-DL", MAGNET, "test").unwrap();
        assert_eq!(release.kind, "movie");
    }

    #[test]
    #[ignore = "micro-benchmark, run with --ignored --nocapture"]
    fn bench_parse_quality() {
        let titles = [
            "The.Pitt.S01E01.Pilot.1080p.WEB-DL.DDP5.1.H.264-ABC",
            "Movie.2024.2160p.UHD.BluRay.REMUX.DV.HDR10.ITA.ENG.DTS-HD.x265-GROUP",
            "Show.S02E01-05.720p.HDTV.x264.ITA.SUB.ITA",
            "Another.Movie.2021.1080p.WEBRip.iT.WEB-DL.AAC2.0.x264",
            "Daily.Show.2026-09-21.WEB-DL.1080p.ITA",
        ];
        let start = std::time::Instant::now();
        let mut acc = 0_i64;
        for _ in 0..20_000 {
            for title in titles {
                acc += parse_quality(title).score();
                acc += parse_release(title, MAGNET, "bench").map(|r| r.quality.score()).unwrap_or(0);
            }
        }
        eprintln!(
            "bench 200k classify: {:?} (acc {acc})",
            start.elapsed()
        );
    }

    #[test]
    fn matches_aliases_after_normalization() {
        assert!(series_names_match("Grey's Anatomy", "Greys.Anatomy"));
    }

    #[test]
    fn matches_stylized_titles_with_leet_and_parenthesized_year() {
        // File legacy `PLUR1BUS (2025) - S01E01 ...` con serie configurata
        // "Pluribus": il `1` leet e l'anno fra parentesi non devono impedire il
        // collegamento all'archivio.
        assert!(series_names_match("Pluribus", "PLUR1BUS (2025)"));
        assert!(series_names_match("Pluribus", "PLUR1BUS"));
        assert!(series_names_match("Pluribus", "Pluribus (2025)"));
        // I titoli numerici non devono essere sfuocati dal folding.
        assert!(series_names_match("9-1-1", "9-1-1"));
        assert!(!series_names_match("9-1-1", "Pluribus"));
        // Anno "nudo" e serie intitolate a un anno restano distinti.
        assert!(series_names_match("1923", "1923"));
        assert!(!series_names_match("1923", "1883"));
    }

    #[test]
    fn parses_episode_key_from_release_name() {
        assert_eq!(
            parse_episode_key("Neagley.S01E05.Trip.2160p.WEB-DL.mkv"),
            Some(("neagley".to_string(), 1, 5))
        );
        assert_eq!(
            parse_episode_key("PLUR1BUS (2025) - S01E01 - We Is Us.mkv"),
            Some(("plur1bus (2025)".to_string(), 1, 1))
        );
        assert_eq!(
            parse_episode_key("Example 2x03 Title.mkv"),
            Some(("example".to_string(), 2, 3))
        );
        assert!(parse_episode_key("Movie.2026.1080p.mkv").is_none());
    }

    #[test]
    fn detects_italian_without_streaming_tag_false_positives() {
        // iT = iTunes / NF = Netflix must not be read as Italian.
        assert!(!parse_quality("Movie.2026.2160p.iT.WEB-DL.ENG").is_ita);
        assert!(!parse_quality("Show.S01E01.NF.WEB-DL.ENG").is_ita);
        // "It" inside the episode title must not count.
        assert!(!parse_quality("Feel.It.Still.2026.1080p.WEB-DL.ENG").is_ita);
        // Explicit Italian markers do count.
        assert!(parse_quality("Movie.2026.1080p.ITA.WEB-DL").is_ita);
        assert!(parse_quality("Movie.2026.1080p.WEB-DL.[IT].x264").is_ita);
        assert!(parse_quality("Movie.2026.1080p.iT.WEB-DL.ITA").is_ita);
    }

    #[test]
    fn detects_extended_resolution_source_hdr_and_word_real() {
        assert_eq!(parse_quality("Movie.2026.UHD.BluRay").resolution, "2160p");
        assert_eq!(parse_quality("Movie.2026.FullHD.WEB").resolution, "1080p");
        assert_eq!(parse_quality("Movie.2026.1080p.BDRip").source, "bluray");
        assert_eq!(parse_quality("Movie.2026.DVDRip.XviD").source, "dvdrip");
        assert_eq!(parse_quality("Movie.2026.1080p.HDR10Plus").hdr, "HDR10Plus");
        assert_eq!(parse_quality("Movie.2026.1080p.HLG").hdr, "HDR");
        assert!(parse_quality("Movie.2026.1080p.REAL").is_real);
        assert!(!parse_quality("Movie.2026.1080p.Really.Good").is_real);
    }

    #[test]
    fn parses_concatenated_multi_episode() {
        let release = parse_release(
            "Example.Show.S02E01E02E03.1080p.WEB-DL.ITA",
            MAGNET,
            "test",
        )
        .unwrap();
        assert_eq!(release.series.as_deref(), Some("Example Show"));
        assert_eq!(release.season, Some(2));
        assert_eq!(release.episode, Some(1));
        assert_eq!(release.episode_range, vec![1, 2, 3]);
        assert!(release.is_pack);
    }

    #[test]
    fn movie_filter_rejects_non_movie_releases() {
        assert!(passes_movie_filter("The.Batman.2022.1080p.WEB-DL"));
        assert!(!passes_movie_filter("Show.S01E01.1080p"));
        assert!(!passes_movie_filter("Breaking.Bad.Season.5.1080p"));
        assert!(!passes_movie_filter("WWE.Raw.2026.1080p.WEB-DL"));
        assert!(!passes_movie_filter("MotoGP.2026.1080p.WEB-DL"));
        assert!(!passes_movie_filter("Some.Game.Build.12345.2024"));
        assert!(!passes_movie_filter("Nintendo.Switch.Game.2024"));
        assert!(!passes_movie_filter("(Italo-Disco) Artista - Titolo"));
        assert!(!passes_movie_filter("History.Magazine.2024.1080p"));
    }

    #[test]
    fn parses_date_based_episode() {
        let release = parse_release("Daily.Show.2026-09-21.1080p.WEB-DL", MAGNET, "test").unwrap();
        assert_eq!(release.kind, "series");
        assert_eq!(release.series.as_deref(), Some("Daily Show"));
        assert_eq!(release.season, Some(2026));
        assert_eq!(release.episode, Some(264));
        assert!(!release.is_pack);
    }
}
