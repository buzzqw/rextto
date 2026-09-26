use gloo_net::http::{Request, Response};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Title};
use serde_json::{json, Value};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

const NAV_GROUPS: &[(&str, &[(&str, &str)])] = &[
    ("Panoramica", &[("dashboard", "Dashboard")]),
    ("Download", &[("downloads", "Scarico")]),
    (
        "Libreria",
        &[
            ("series", "Serie TV"),
            ("movies", "Film"),
            ("gaps", "Mancanti"),
        ],
    ),
    (
        "Scoperta",
        &[
            ("search", "Esplora"),
            ("archive", "Archivio"),
            ("comics", "Fumetti"),
        ],
    ),
    (
        "Sistema",
        &[
            ("settings", "Configurazione"),
            ("integrations", "Integrazioni"),
            ("maintenance", "Manutenzione"),
            ("health", "Salute"),
            ("logs", "Log"),
            ("blocklist", "Blocklist"),
            ("manual", "Manuale"),
            ("license", "Licenza"),
        ],
    ),
];

const FULL_EUPL_LICENSE: &str = include_str!("../../../LICENSE");

/// Voci poco usate: restano nel menu su desktop ma sono nascoste su mobile,
/// così la barra di navigazione del telefono resta corta.
fn is_optional_nav(id: &str) -> bool {
    matches!(id, "license" | "blocklist")
}

/// On mobile keep the navigation focused on the daily workflow. These pages
/// remain available on desktop, but are intentionally not exposed in the
/// compact mobile menu.
fn is_mobile_hidden_nav(id: &str) -> bool {
    matches!(id, "gaps" | "search")
}

/// Log is part of the compact mobile menu even though it belongs to the
/// collapsible System group on desktop.
fn is_mobile_primary_nav(id: &str) -> bool {
    id == "logs"
}

fn page_label(page: &str) -> &'static str {
    for (_, items) in NAV_GROUPS {
        for (id, label) in *items {
            if *id == page {
                return label;
            }
        }
    }
    "Dashboard"
}

/// Registro condiviso delle impostazioni modificate ma non ancora salvate
/// (chiave → valore), usato dal banner "Salva tutte" in Configurazione.
#[derive(Clone, Copy)]
struct DirtySettings {
    items: RwSignal<std::collections::BTreeMap<String, String>>,
}

#[derive(Clone, Copy)]
struct GlobalSearch {
    query: RwSignal<String>,
    results: RwSignal<Vec<Value>>,
    loading: RwSignal<bool>,
    searched: RwSignal<bool>,
    elapsed: RwSignal<u32>,
}

/// Tab del pannello Configurazione (id interno -> etichetta).
const SETTINGS_TABS: &[(&str, &str)] = &[
    ("daemon", "Daemon"),
    ("sources", "Sorgenti"),
    ("libtorrent", "Libtorrent"),
    ("scores", "Punteggi"),
    ("rename", "Rinomina"),
    ("advanced", "Avanzate"),
    ("acquisition", "Acquisizione"),
    ("notify", "Notifiche"),
    ("paths", "Percorsi"),
    ("i18n", "Traduzioni"),
];

/// Voce dell'indice delle impostazioni: (etichetta italiana, tab, chiave
/// `setting_key` usata per ancorare il campo nel DOM).
const SETTINGS_INDEX: &[(&str, &str, &str)] = &[
    // daemon
    ("Ricerca automatica serie/film (secondi)", "daemon", "refresh_interval"),
    ("Età massima release (giorni)", "daemon", "max_release_age_days"),
    ("Gap massimi per serie/ciclo", "daemon", "gap_fill_max_per_series"),
    ("Gap filling attivo", "daemon", "gap_filling"),
    ("Intervallo deep search (ore)", "daemon", "gap_deep_interval_hours"),
    ("Deep search massime per ciclo", "daemon", "gap_deep_max_per_cycle"),
    ("Attivo", "daemon", "active"),
    // sources
    ("Blacklist (parole separate da virgola)", "sources", "blacklist"),
    ("Feed RSS", "sources", ""),
    ("Indexer Torznab (Jackett / Prowlarr)", "sources", ""),
    ("FlareSolverr", "sources", ""),
    ("Motori web", "sources", ""),
    ("Filtri contenuto esclusi", "sources", ""),
    ("Filtri per sorgente", "sources", ""),
    // libtorrent
    ("Client abilitato", "libtorrent", "libtorrent_enabled"),
    ("Auto-gestione dinamica coda e risorse", "libtorrent", "libtorrent_dynamic_queue"),
    ("Ottimizzazione continua (periodica)", "libtorrent", "libtorrent_auto_optimize"),
    ("Slot download dinamici minimi", "libtorrent", "libtorrent_dynamic_queue_min"),
    ("Slot download dinamici massimi", "libtorrent", "libtorrent_dynamic_queue_max"),
    ("Non contare i torrent fermi negli slot attivi", "libtorrent", "libtorrent_dont_count_slow_torrents"),
    ("Considera stalled dopo (minuti)", "libtorrent", "libtorrent_stall_after_min"),
    ("Retry stalled (minuti)", "libtorrent", "libtorrent_stall_retry_min"),
    ("Rimozione stalled (minuti, 0 = mai)", "libtorrent", "libtorrent_stall_giveup_min"),
    ("Download sequenziale", "libtorrent", "libtorrent_sequential"),
    ("Download attivi", "libtorrent", "libtorrent_active_downloads"),
    ("Seed attivi", "libtorrent", "libtorrent_active_seeds"),
    ("Limite torrent attivi", "libtorrent", "libtorrent_active_limit"),
    ("Seed ratio globale (0 = infinito)", "libtorrent", "libtorrent_seed_ratio"),
    ("Seed massimo (minuti, fallback)", "libtorrent", "libtorrent_seed_time"),
    ("Seed massimo (giorni)", "libtorrent", "libtorrent_seed_time_days"),
    ("Elimina i completati dopo il seed", "libtorrent", "auto_remove_completed"),
    ("Limite connessioni totali", "libtorrent", "libtorrent_connections_limit"),
    ("Slot upload", "libtorrent", "libtorrent_upload_slots_limit"),
    ("Half-open limit", "libtorrent", "libtorrent_half_open_limit"),
    ("Connessioni max per torrent", "libtorrent", "libtorrent_max_connections_per_torrent"),
    ("Upload max per torrent", "libtorrent", "libtorrent_max_uploads_per_torrent"),
    ("Thread AIO disco", "libtorrent", "libtorrent_aio_threads"),
    ("Cache disco (blocchi, -1 auto)", "libtorrent", "libtorrent_cache_size"),
    ("Scadenza cache (s)", "libtorrent", "libtorrent_cache_expiry"),
    ("Coda alert", "libtorrent", "libtorrent_alert_queue_size"),
    ("DHT", "libtorrent", "libtorrent_dht"),
    ("PEX", "libtorrent", "libtorrent_pex"),
    ("LSD", "libtorrent", "libtorrent_lsd"),
    ("UPnP", "libtorrent", "libtorrent_upnp"),
    ("NAT-PMP", "libtorrent", "libtorrent_natpmp"),
    ("uTP", "libtorrent", "libtorrent_utp"),
    ("Preferisci RC4", "libtorrent", "libtorrent_prefer_rc4"),
    ("Annuncia a tutti i tracker", "libtorrent", "libtorrent_announce_to_all_trackers"),
    ("Annuncia a tutti i tier", "libtorrent", "libtorrent_announce_to_all_tiers"),
    ("Più connessioni per IP", "libtorrent", "libtorrent_allow_multiple_connections_per_ip"),
    ("Intervallo announce (s)", "libtorrent", "libtorrent_announce_interval"),
    ("Connect boost", "libtorrent", "libtorrent_torrent_connect_boost"),
    ("Nodi bootstrap DHT", "libtorrent", "libtorrent_dht_bootstrap_nodes"),
    ("Cifratura", "libtorrent", "libtorrent_encryption"),
    ("Applica IP filter", "libtorrent", "libtorrent_apply_ip_filter"),
    ("IP filter (file/URL)", "libtorrent", "libtorrent_ipfilter_url"),
    ("Proxy host", "libtorrent", "libtorrent_proxy_host"),
    ("Proxy porta", "libtorrent", "libtorrent_proxy_port"),
    ("Interfacce listen", "libtorrent", "libtorrent_listen_interfaces"),
    ("Usa il RAM disk", "libtorrent", "libtorrent_ramdisk_enabled"),
    ("Dimensione massima per torrent (GB)", "libtorrent", "libtorrent_ramdisk_threshold_gb"),
    ("Margine libero da mantenere (GB)", "libtorrent", "libtorrent_ramdisk_margin_gb"),
    ("Spazio minimo libero (byte, 0 = dal margine)", "libtorrent", "libtorrent_ramdisk_min_free_bytes"),
    ("Porta minima", "libtorrent", "libtorrent_port_min"),
    ("Porta massima", "libtorrent", "libtorrent_port_max"),
    ("Download globale (KiB/s, 0 = illimitato)", "libtorrent", "libtorrent_dl_limit"),
    ("Upload globale (KiB/s, 0 = illimitato)", "libtorrent", "libtorrent_ul_limit"),
    ("Programmazione velocità attiva", "libtorrent", "libtorrent_sched_enabled"),
    ("Programmazione — ora inizio (HH:MM)", "libtorrent", "libtorrent_sched_start"),
    ("Programmazione — ora fine (HH:MM)", "libtorrent", "libtorrent_sched_end"),
    ("Programmazione — giorni (0=Lun … 6=Dom, es. 0,1,2,3,4)", "libtorrent", "libtorrent_sched_days"),
    ("Programmazione — download (KiB/s)", "libtorrent", "libtorrent_sched_dl_limit"),
    ("Programmazione — upload (KiB/s)", "libtorrent", "libtorrent_sched_ul_limit"),
    ("Impostazioni libtorrent avanzate", "libtorrent", "libtorrent_extra_settings"),
    ("Interfaccia VPN (killswitch)", "libtorrent", ""),
    ("Test porte torrent", "libtorrent", ""),
    ("RAM disk", "libtorrent", ""),
    ("IP filter", "libtorrent", ""),
    // scores
    ("Gruppi custom (release group)", "scores", ""),
    ("Simulatore punteggio", "scores", ""),
    // rename
    ("Rinomina episodi", "rename", "rename_episodes"),
    ("Lingua TVDB (es. ita, eng)", "rename", "tvdb_language"),
    ("Lingua TMDB (es. it-IT)", "rename", "tmdb_language"),
    ("Lingua predefinita (es. ita)", "rename", "default_language"),
    ("Cleanup upgrade", "rename", "cleanup_upgrades"),
    ("Differenza minima score per cleanup", "rename", "cleanup_min_score_diff"),
    ("Differenza minima score per upgrade", "rename", "upgrade_min_score_diff"),
    ("Token API Rextto", "rename", "api_token"),
    ("Formato rinomina", "rename", "rename-format"),
    ("Template rinomina", "rename", "rename-template"),
    ("TMDB API key", "rename", "tmdb_api_key"),
    ("TVDB API key", "rename", "tvdb_api_key"),
    // advanced
    ("Spazio libero minimo per scaricare (GB)", "advanced", "min_free_space_gb"),
    ("Trash — giorni di conservazione (0 = sempre tutto)", "advanced", "trash_retention_days"),
    ("Archivio — giorni di conservazione (0 = illimitato)", "advanced", "archive_retention_days"),
    ("Pulizia automatica archivio", "advanced", "archive_cleanup_enabled"),
    ("Archivio — età massima (giorni)", "advanced", "archive_max_age_days"),
    ("Archivio — mantieni almeno N voci", "advanced", "archive_keep_min"),
    ("Pagine feed da leggere", "advanced", "stop_on_old_page_threshold"),
    ("Verifica rinomina (ore)", "advanced", "rename_verify_interval"),
    ("Sposta gli episodi/pack in archivio (non copiare)", "advanced", "move_episodes"),
    ("Debug (log dettagliati)", "advanced", "debug_enabled"),
    // acquisition
    ("Delay serie (minuti, 0 = nessuno)", "acquisition", "delay_torrent_minutes"),
    ("Delay film (minuti, 0 = nessuno)", "acquisition", "delay_movies_minutes"),
    ("Bypassa il delay sopra questo punteggio (0 = mai)", "acquisition", "delay_bypass_score"),
    ("Housekeeping periodico attivo", "acquisition", "housekeeping_enabled"),
    ("Housekeeping — intervallo (ore)", "acquisition", "housekeeping_interval_hours"),
    ("Housekeeping — statistiche cicli di ricerca conservate", "acquisition", "housekeeping_retain_cycles"),
    ("Housekeeping — visti nel feed (giorni, 0 = mai)", "acquisition", "housekeeping_seen_days"),
    ("Housekeeping — storico download (giorni, 0 = conserva)", "acquisition", "housekeeping_history_days"),
    ("Backfill MediaInfo automatico", "acquisition", "media_info_backfill_enabled"),
    ("Backfill MediaInfo — intervallo (minuti)", "acquisition", "media_info_backfill_interval_minutes"),
    ("Backfill MediaInfo — file per volta", "acquisition", "media_info_backfill_batch"),
    ("Cartelle osservate", "acquisition", ""),
    ("Sorgenti in backoff", "acquisition", ""),
    // notify
    ("Telegram attivo", "notify", "notify_telegram"),
    ("Telegram bot token", "notify", "telegram_bot_token"),
    ("Telegram chat ID", "notify", "telegram_chat_id"),
    ("Webhook URL", "notify", "notify_webhook_url"),
    ("Webhook secret", "notify", "notify_webhook_secret"),
    ("Email attiva", "notify", "notify_email"),
    ("SMTP", "notify", "email_smtp"),
    ("Email mittente", "notify", "email_from"),
    ("Email destinatario", "notify", "email_to"),
    ("Password email", "notify", "email_password"),
    // paths
    ("Cartella archivio", "paths", "archive_root"),
    ("Cartella trash", "paths", "trash_path"),
    ("Azione cleanup", "paths", "cleanup_action"),
    ("Download libtorrent", "paths", "libtorrent_dir"),
    ("Cartella temporanea libtorrent", "paths", "libtorrent_temp_dir"),
    ("Percorsi NAS per categoria (tag)", "paths", ""),
    // i18n
    ("Importa/esporta traduzioni interfaccia", "i18n", ""),
];

/// Stato condiviso per aprire la Configurazione su una tab e su un campo
/// specifico (`focus` = `setting_key` da evidenziare e raggiungere).
#[derive(Clone, Copy)]
struct SettingsNav {
    tab: RwSignal<String>,
    focus: RwSignal<Option<String>>,
}

/// Stato del riquadro "Cerca impostazioni": il pulsante vive nel menu, mentre
/// l'overlay è renderizzato alla radice dell'app (la sidebar ha un
/// `backdrop-filter` che confinerebbe un `position: fixed`).
#[derive(Clone, Copy)]
struct SettingsSearchState {
    open: RwSignal<bool>,
}

/// Risultato della ricerca tra le impostazioni.
#[derive(Clone)]
struct SettingHit {
    label: String,
    tab: &'static str,
    key: &'static str,
}

fn settings_tab_label(tab: &str) -> &'static str {
    SETTINGS_TABS
        .iter()
        .find(|(id, _)| *id == tab)
        .map(|(_, label)| *label)
        .unwrap_or("Configurazione")
}

/// Cerca tra le etichette (italiane e tradotte) e le chiavi delle
/// impostazioni. Include anche i singoli campi punteggio di `SCORE_GROUPS`.
fn search_settings(data: RwSignal<Data>, query: &str) -> Vec<SettingHit> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.len() < 2 {
        return Vec::new();
    }
    let snapshot = data.get_untracked();
    let translate = |source: &str| {
        snapshot
            .i18n
            .get(source)
            .cloned()
            .unwrap_or_else(|| source.to_string())
    };
    let mut hits = Vec::new();
    for &(label, tab, key) in SETTINGS_INDEX {
        let translated = translate(label);
        let haystack = format!("{} {} {}", label, translated, key).to_ascii_lowercase();
        if haystack.contains(&needle) {
            hits.push(SettingHit {
                label: translated,
                tab,
                key,
            });
        }
    }
    for (group, fields) in SCORE_GROUPS {
        for (label, key, _default) in *fields {
            let display = format!("{} - {}", translate(group), translate(label));
            let haystack = format!("{display} {key}").to_ascii_lowercase();
            if haystack.contains(&needle) {
                hits.push(SettingHit {
                    label: display,
                    tab: "scores",
                    key,
                });
            }
        }
    }
    hits.truncate(14);
    hits
}

#[derive(Clone, Default)]
struct Data {
    status: Value,
    stats: Value,
    health: Value,
    config: Value,
    library: Value,
    torrents: Vec<Value>,
    torrent_delta: i64,
    torrent_tags: Vec<Value>,
    download_tags: Vec<String>,
    history: Vec<Value>,
    history_page: usize,
    history_pages: usize,
    history_total: i64,
    history_filter: String,
    movie_history: Vec<Value>,
    recent_downloads: Vec<Value>,
    db_info: Value,
    events: Vec<Value>,
    archive: Vec<Value>,
    archive_page: usize,
    archive_pages: usize,
    archive_total: i64,
    search: Vec<Value>,
    gaps: Vec<Value>,
    calendar: Vec<Value>,
    comics: Vec<Value>,
    comic_downloads: Vec<Value>,
    comics_history: Vec<Value>,
    comics_weekly: Vec<Value>,
    comics_weekly_enabled: bool,
    comics_weekly_from_date: String,
    comics_check_interval: u64,
    comics_last_check_ts: i64,
    blocklist: Vec<Value>,
    trakt: Value,
    simkl: Value,
    backups: Vec<Value>,
    setup: Value,
    language: String,
    i18n: std::sync::Arc<std::collections::BTreeMap<String, String>>,
    net_history: Vec<f64>,
    net_history_up: Vec<f64>,
    torrent_history: std::collections::BTreeMap<String, Vec<f64>>,
    live_history: std::collections::BTreeMap<String, Vec<f64>>,
    toasts: Vec<Value>,
    error: String,
    notice: String,
}

fn text(value: &Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

/// Reads a user-facing string from an API value and translates its fallback
/// when the backend did not provide one.
fn text_tr(data: RwSignal<Data>, value: &Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| tr(data, fallback))
}

/// Traduce una stringa sorgente (italiana) con le traduzioni attive; se manca
/// la traduzione restituisce la sorgente invariata. Usa `with` per leggere la
/// mappa senza clonare l'intero `Data` (era il costo principale della UI).
fn tr(data: RwSignal<Data>, source: &str) -> String {
    data.with(|current| {
        current
            .i18n
            .get(source)
            .cloned()
            .unwrap_or_else(|| source.to_string())
    })
}

fn tr_opt(data: Option<RwSignal<Data>>, source: &str) -> String {
    data.map(|signal| tr(signal, source))
        .unwrap_or_else(|| source.to_string())
}

/// Translates a message template and then substitutes its runtime values.
/// Keeping the template as the translation key lets formatted notices follow
/// the active language just like static labels do.
fn tr_format(data: RwSignal<Data>, source: &str, values: &[(&str, String)]) -> String {
    let mut message = tr(data, source);
    for (placeholder, value) in values {
        message = message.replace(placeholder, value);
    }
    message
}

/// Traduce una stringa usando il context `Data` dell'app. Serve per i
/// componenti riutilizzabili (Panel, Metric, StatLine, ...) che ricevono solo
/// un letterale: la traduzione avviene senza toccare ogni punto di chiamata.
fn ctx_tr(source: &'static str) -> Signal<String> {
    let data = use_context::<RwSignal<Data>>();
    Signal::derive(move || data.map(|signal| tr(signal, source)).unwrap_or_else(|| source.to_string()))
}

fn raw(value: &Value, key: &str, fallback: &str) -> String {
    match value.get(key) {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Null) => fallback.to_string(),
        Some(value) => value.to_string(),
        None => fallback.to_string(),
    }
}

fn number(value: &Value, key: &str) -> String {
    value
        .get(key)
        .map(ToString::to_string)
        .unwrap_or_else(|| "0".into())
}

/// Interpreta un valore di configurazione come booleano, accettando le stesse
/// forme usate dal backend (`yes/no`, `true/false`, `1/0`, `on/off`).
fn flag(value: &Value, key: &str, fallback: bool) -> bool {
    let raw_value = raw(value, key, if fallback { "yes" } else { "no" });
    matches!(
        raw_value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn array(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Filtro locale per le release già trovate: confronta titolo, fonte, origine e
/// chiave episodio. Le parole sono in AND, così `web ita` restringe davvero la
/// lista senza ripetere ricerche alle sorgenti remote.
fn release_matches_result_filter(result: &Value, filter: &str, season: i64, episode: i64) -> bool {
    let terms = filter
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return true;
    }
    let release = result.get("release").unwrap_or(result);
    let searchable = format!(
        "{} {} {} S{season:02}E{episode:02}",
        text(release, "title", ""),
        text(release, "source", ""),
        text(result, "origin", ""),
    )
    .to_ascii_lowercase();
    terms.iter().all(|term| searchable.contains(term))
}

fn size(value: &Value, key: &str) -> String {
    let mut amount = value.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut index = 0;
    while amount >= 1024.0 && index < units.len() - 1 {
        amount /= 1024.0;
        index += 1;
    }
    format!("{amount:.1} {}", units[index])
}

fn token() -> Option<String> {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item("rextto_api_token").ok().flatten())
}

fn save_token(value: &str) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item("rextto_api_token", value);
    }
}

fn format_remaining_seconds(remaining: i64) -> String {
    if remaining <= 0 {
        return "imminente".into();
    }
    let (hours, minutes, seconds) = (remaining / 3600, (remaining % 3600) / 60, remaining % 60);
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

async fn json_response(response: Response) -> Result<Value, String> {
    let status = response.status();
    if status == 401 {
        return Err("AUTH".into());
    }
    let value = response
        .json::<Value>()
        .await
        .map_err(|error| error.to_string())?;
    if !(200..300).contains(&status) {
        return Err(text(&value, "error", "Operazione non riuscita"));
    }
    Ok(value)
}

async fn request_token() -> Option<String> {
    let value = web_sys::window()
        .and_then(|window| window.prompt_with_message("Token API Rextto").ok().flatten())?;
    save_token(&value);
    Some(value)
}

async fn get(path: &str) -> Result<Value, String> {
    for _ in 0..2 {
        let mut request = Request::get(path);
        if let Some(value) = token() {
            request = request.header("x-rextto-token", &value);
        }
        let result = json_response(request.send().await.map_err(|error| error.to_string())?).await;
        match result {
            Err(error) if error == "AUTH" => {
                if request_token().await.is_none() {
                    return Err("Autenticazione richiesta".into());
                }
            }
            result => return result,
        }
    }
    Err("Autenticazione richiesta".into())
}

async fn get_text(path: &str) -> Result<String, String> {
    for _ in 0..2 {
        let mut request = Request::get(path);
        if let Some(value) = token() {
            request = request.header("x-rextto-token", &value);
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        if response.status() == 401 {
            if request_token().await.is_none() {
                return Err("Autenticazione richiesta".into());
            }
            continue;
        }
        let status = response.status();
        let body = response.text().await.map_err(|error| error.to_string())?;
        if !(200..300).contains(&status) {
            return Err(body);
        }
        return Ok(body);
    }
    Err("Autenticazione richiesta".into())
}

async fn send(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    for _ in 0..2 {
        let response = match (method, body.clone()) {
            ("DELETE", _) => {
                let mut request = Request::delete(path);
                if let Some(value) = token() {
                    request = request.header("x-rextto-token", &value);
                }
                request.send().await.map_err(|error| error.to_string())?
            }
            (_, Some(value)) => {
                let mut request = Request::post(path);
                if let Some(token) = token() {
                    request = request.header("x-rextto-token", &token);
                }
                request
                    .json(&value)
                    .map_err(|error| error.to_string())?
                    .send()
                    .await
                    .map_err(|error| error.to_string())?
            }
            (_, None) => {
                let mut request = Request::post(path);
                if let Some(token) = token() {
                    request = request.header("x-rextto-token", &token);
                }
                request.send().await.map_err(|error| error.to_string())?
            }
        };
        match json_response(response).await {
            Err(error) if error == "AUTH" => {
                if request_token().await.is_none() {
                    return Err("Autenticazione richiesta".into());
                }
            }
            result => return result,
        }
    }
    Err("Autenticazione richiesta".into())
}

/// Variante di `send` con un tetto massimo di attesa: se il server non risponde
/// entro `millis`, il chiamante riceve un errore invece di restare bloccato a
/// tempo indefinito (ad esempio con un upstream TMDB/TVDB momentaneamente lento).
async fn send_timeout(
    method: &str,
    path: &str,
    body: Option<Value>,
    millis: u32,
) -> Result<Value, String> {
    let request = send(method, path, body);
    let timeout = TimeoutFuture::new(millis);
    futures::pin_mut!(request, timeout);
    match futures::future::select(request, timeout).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right((_, _)) => {
            Err("Tempo scaduto: il server non ha risposto".into())
        }
    }
}

fn trigger_refresh() {
    if let Some(refresh) = use_context::<RwSignal<u32>>() {
        refresh.update(|value| *value += 1);
    }
}

fn flash(data: RwSignal<Data>, result: Result<Value, String>, success: &'static str) {
    match result {
        Ok(_) => {
            let message = tr(data, success);
            push_toast(data, "ok", message.clone());
            data.update(|current| {
                current.error.clear();
                current.notice = message;
            });
        }
        Err(error) => {
            let error = tr(data, &error);
            push_toast(data, "err", error.clone());
            data.update(|current| {
                current.notice.clear();
                current.error = error;
            });
        }
    }
}

fn flash_text(data: RwSignal<Data>, kind: &str, message: String) {
    let message = tr(data, &message);
    push_toast(data, kind, message.clone());
    data.update(|current| {
        if kind == "ok" {
            current.error.clear();
            current.notice = message;
        } else {
            current.notice.clear();
            current.error = message;
        }
    });
}

/// Mostra una notifica temporanea (toast) e la rimuove dopo qualche secondo.
fn push_toast(data: RwSignal<Data>, kind: &str, text: String) {
    if text.trim().is_empty() {
        return;
    }
    let text = tr(data, &text);
    let id = js_sys::Date::now();
    data.update(|current| {
        current.toasts.push(json!({"id": id, "kind": kind, "text": text}));
        if current.toasts.len() > 6 {
            let excess = current.toasts.len() - 6;
            current.toasts.drain(0..excess);
        }
    });
    spawn_local(async move {
        TimeoutFuture::new(5000).await;
        data.update(|current| {
            current
                .toasts
                .retain(|item| item.get("id").and_then(Value::as_f64) != Some(id))
        });
    });
}

fn run_post(data: RwSignal<Data>, path: &str, body: Option<Value>, success: &'static str) {
    let path = path.to_string();
    spawn_local(async move {
        let result = send("POST", &path, body).await;
        flash(data, result, success);
        trigger_refresh();
    });
}

fn run_delete(data: RwSignal<Data>, path: &str, success: &'static str) {
    let path = path.to_string();
    spawn_local(async move {
        let result = send("DELETE", &path, None).await;
        flash(data, result, success);
        trigger_refresh();
    });
}

/// Executes a per-series rename and updates the preview modal state.
///
/// `force` also reprocesses files that already pass the quick naming check, so
/// the exact target name is recomputed from the template and TMDB metadata.
#[allow(clippy::too_many_arguments)]
fn run_series_rename(
    data: RwSignal<Data>,
    path: String,
    items: RwSignal<Vec<Value>>,
    busy: RwSignal<bool>,
    message: RwSignal<String>,
    already_signal: RwSignal<usize>,
    force: bool,
) {
    busy.set(true);
    message.set(tr(
        data,
        if force {
            "Rinomina forzata in corso…"
        } else {
            "Rinomina in corso…"
        },
    ));
    spawn_local(async move {
        match send("POST", &path, Some(json!({ "force": force }))).await {
            Ok(value) => {
                let list = array(&value, "items");
                let already_ok = value
                    .get("already_ok_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let discarded = value
                    .get("discarded_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let errors = list.iter().filter(|item| item.get("error").is_some()).count();
                if errors > 0 {
                    message.set(tr_format(
                        data,
                        "Rinomina eseguita: {renamed} rinominati, {discarded} duplicati nel cestino, {errors} errori",
                        &[
                            ("{renamed}", list.len().saturating_sub(discarded + errors).to_string()),
                            ("{discarded}", discarded.to_string()),
                            ("{errors}", errors.to_string()),
                        ],
                    ));
                } else {
                    message.set(tr_format(
                        data,
                        "Rinomina eseguita: {renamed} rinominati, {discarded} duplicati nel cestino ({already_ok} già corretti)",
                        &[
                            ("{renamed}", list.len().saturating_sub(discarded).to_string()),
                            ("{discarded}", discarded.to_string()),
                            ("{already_ok}", already_ok.to_string()),
                        ],
                    ));
                }
                items.set(list);
                already_signal.set(already_ok);
                trigger_refresh();
            }
            Err(error) => message.set(tr_format(
                data,
                "Rinomina non riuscita: {error}",
                &[("{error}", error)],
            )),
        }
        busy.set(false);
    });
}

/// Pulisce i torrent completati e riporta l'esito reale (rimossi/saltati) invece
/// di un messaggio generico: rende chiaro che vengono tolti solo quelli che
/// hanno già raggiunto i limiti di seed e che ora sono nello Storico.
fn run_cleanup_completed(data: RwSignal<Data>) {
    spawn_local(async move {
        match send(
            "POST",
            "/api/torrents/remove_completed",
            Some(json!({"delete_files": false})),
        )
        .await
        {
            Ok(value) => {
                let removed = value.get("removed").and_then(Value::as_u64).unwrap_or(0);
                let skipped = value.get("skipped").and_then(Value::as_u64).unwrap_or(0);
                let http_removed = value.get("http_removed").and_then(Value::as_u64).unwrap_or(0);
                let message = if removed == 0 && http_removed == 0 {
                    tr_format(data, "Nessun torrent rimosso: {skipped} non hanno ancora raggiunto i limiti di seed (ratio/tempo) o sono in seed infinito.", &[("{skipped}", skipped.to_string())])
                } else {
                    tr_format(data, "Rimossi {removed} torrent e {http} download HTTP completati · saltati {skipped} (limiti di seed non raggiunti o seed infinito). I torrent sono ora nello Storico download.", &[("{removed}", removed.to_string()), ("{http}", http_removed.to_string()), ("{skipped}", skipped.to_string())])
                };
                flash_text(data, "ok", message);
            }
            Err(error) => flash_text(data, "err", error),
        }
        trigger_refresh();
    });
}

fn remove_monitored_comic(data: RwSignal<Data>, id: i64) {
    spawn_local(async move {
        let result = send("DELETE", &format!("/api/comics/{id}"), None).await;
        if result.is_ok() {
            data.update(|current| {
                current.comics.retain(|comic| {
                    comic.get("id").and_then(Value::as_i64) != Some(id)
                });
            });
        }
        flash(data, result, "Fumetto rimosso");
    });
}

async fn load(data: RwSignal<Data>, busy: RwSignal<bool>, silent: bool) {
    if !silent {
        busy.set(true);
    }
    let result = async {
        let status = get("/api/status").await?;
        let stats = get("/api/stats").await.unwrap_or_default();
        let health = get("/api/health").await?;
        let config = get("/api/config").await?;
        let library = get("/api/config/library").await?;
        let torrents = get("/api/torrents")
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default();
        let torrent_tags = array(&get("/api/torrent-tags").await?, "items");
        let download_tags = get("/api/download-tags")
            .await
            .ok()
            .and_then(|value| value.get("items").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item.as_str().map(ToOwned::to_owned))
            .collect::<Vec<String>>();
        // Storico download paginato: mantiene la pagina corrente anche durante i
        // refresh silenziosi, così la tabella non salta alla prima pagina.
        let history_page_wanted = data.get_untracked().history_page.max(1);
        let history_filter = data.get_untracked().history_filter;
        let history_response =
            get(&format!("/api/torrents/history?page={history_page_wanted}&limit=10&q={}", urlencoding::encode(&history_filter))).await?;
        let history = array(&history_response, "items");
        let history_page = history_response
            .get("page")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize;
        let history_pages = history_response
            .get("pages")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize;
        let history_total = history_response
            .get("total")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let movie_history = array(&get("/api/movies/history").await?, "items");
        let recent_downloads = array(&get("/api/recent-downloads").await?, "items");
        let db_info = get("/api/db/info").await.unwrap_or_default();
        let events = get("/api/torrent-events")
            .await
            .and_then(|value| {
                value
                    .as_array()
                    .cloned()
                    .ok_or_else(|| "invalid events response".to_string())
            })
            .unwrap_or_default();
        let archive_response = if silent {
            Value::Null
        } else {
            get("/api/archive?limit=100").await?
        };
        let archive = array(&archive_response, "items");
        let archive_page = archive_response
            .get("page")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize;
        let archive_pages = archive_response
            .get("pages")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize;
        let archive_total = archive_response
            .get("total")
            .and_then(Value::as_i64)
            .unwrap_or(archive.len() as i64);
        let gaps = get("/api/gaps")
            .await
            .map(|value| array(&value, "items"))
            .unwrap_or_default();
        let calendar = if silent {
            Vec::new()
        } else {
            get("/api/calendar")
                .await
                .map(|value| array(&value, "items"))
                .unwrap_or_default()
        };
        let comics = get("/api/comics")
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default();
        let comic_downloads = get("/api/comics/downloads")
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default();
        let comics_history = get("/api/comics/history")
            .await
            .map(|value| array(&value, "items"))
            .unwrap_or_default();
        let comics_weekly_response = get("/api/comics/weekly").await.unwrap_or_default();
        let comics_weekly = array(&comics_weekly_response, "items");
        let comics_weekly_enabled = comics_weekly_response
            .get("weekly_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let comics_weekly_from_date = text(&comics_weekly_response, "weekly_from_date", "");
        let comics_check_interval = comics_weekly_response
            .get("check_interval_secs")
            .and_then(Value::as_u64)
            .unwrap_or(604800);
        let comics_last_check_ts = comics_weekly_response
            .get("last_check_ts")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let trakt = get("/api/trakt/status").await?;
        let simkl = get("/api/simkl/status").await?;
        let backups = get("/api/backup")
            .await
            .map(|value| array(&value, "items"))
            .unwrap_or_default();
        let blocklist = get("/api/blocklist")
            .await
            .map(|value| array(&value, "items"))
            .unwrap_or_default();
        let setup = get("/api/setup").await.unwrap_or_default();
        // `/api/i18n/active` restituisce solo la lingua attiva: niente da
        // scaricare ad ogni polling (prima si scaricava l'intero dizionario).
        let language = get("/api/i18n/active")
            .await
            .map(|value| text(&value, "lang", "it"))
            .unwrap_or_else(|_| "it".into());
        // Il dizionario i18n è pesante: lo scarichiamo solo ai caricamenti
        // completi (non ai polling silenziosi).
        let i18n: Option<std::collections::BTreeMap<String, String>> = if silent {
            None
        } else {
            get(&format!("/api/i18n?lang={language}"))
                .await
                .ok()
                .map(|value| {
                    array(&value, "items")
                        .into_iter()
                        .filter_map(|item| {
                            let key = text(&item, "key", "");
                            if key.is_empty() {
                                return None;
                            }
                            Some((key, text(&item, "value", "")))
                        })
                        .collect()
                })
        };
        data.update(|current| {
            current.status = status;
            current.stats = stats;
            current.health = health;
            current.config = config;
            current.library = library;
            current.torrents = torrents;
            current.torrent_tags = torrent_tags;
            current.download_tags = download_tags;
            current.history = history;
            current.history_page = history_page;
            current.history_pages = history_pages;
            current.history_total = history_total;
            current.movie_history = movie_history;
            current.recent_downloads = recent_downloads;
            current.db_info = db_info;
            current.events = events;
            if !silent {
                current.archive = archive;
                current.archive_page = archive_page;
                current.archive_pages = archive_pages;
                current.archive_total = archive_total;
                current.calendar = calendar;
            }
            current.gaps = gaps;
            current.comics = comics;
            current.comic_downloads = comic_downloads;
            current.comics_history = comics_history;
            current.comics_weekly = comics_weekly;
            current.comics_weekly_enabled = comics_weekly_enabled;
            current.comics_weekly_from_date = comics_weekly_from_date;
            current.comics_check_interval = comics_check_interval;
            current.comics_last_check_ts = comics_last_check_ts;
            current.trakt = trakt;
            current.simkl = simkl;
            current.backups = backups;
            current.blocklist = blocklist;
            current.setup = setup;
            current.language = language;
            if let Some(i18n) = i18n {
                current.i18n = std::sync::Arc::new(i18n);
            }
            current.error.clear();
        });
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = result {
        data.update(|current| current.error = error);
    }
    if !silent {
        busy.set(false);
    }
}

/* ------------------------------------------------------------------ */
/* Shell                                                               */
/* ------------------------------------------------------------------ */

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    let page = RwSignal::new("dashboard".to_string());
    let data = RwSignal::new(Data::default());
    let busy = RwSignal::new(false);
    let refresh = RwSignal::new(0u32);
    provide_context(refresh);
    provide_context(data);
    provide_context(busy);
    provide_context(DirtySettings {
        items: RwSignal::new(std::collections::BTreeMap::new()),
    });
    provide_context(GlobalSearch {
        query: RwSignal::new(String::new()),
        results: RwSignal::new(Vec::<Value>::new()),
        loading: RwSignal::new(false),
        searched: RwSignal::new(false),
        elapsed: RwSignal::new(0),
    });
    provide_context(SettingsNav {
        tab: RwSignal::new("daemon".to_string()),
        focus: RwSignal::new(None),
    });
    provide_context(SettingsSearchState {
        open: RwSignal::new(false),
    });
    Effect::new(move |_| {
        refresh.get();
        spawn_local(load(data, busy, false));
    });
    spawn_local(async move {
        loop {
            TimeoutFuture::new(15_000).await;
            load(data, busy, true).await;
        }
    });
    spawn_local(async move {
        loop {
            TimeoutFuture::new(4_000).await;
            let process_metrics = get("/api/process-metrics").await.ok();
            if let Ok(value) = get("/api/torrents").await {
                if let Some(items) = value.as_array().cloned() {
                    let sample: f64 = items
                        .iter()
                        .filter_map(|item| item.get("download_rate").and_then(Value::as_f64))
                        .sum();
                    let sample_up: f64 = items
                        .iter()
                        .filter_map(|item| item.get("upload_rate").and_then(Value::as_f64))
                        .sum();
                    let samples: Vec<(String, f64)> = items
                        .iter()
                        .filter_map(|item| {
                            let hash = item.get("hash").and_then(Value::as_str)?.to_ascii_lowercase();
                            let rate = item.get("download_rate").and_then(Value::as_f64).unwrap_or(0.0);
                            Some((hash, rate))
                        })
                        .collect();
                    data.update(|current| {
                        if let Some(metrics) = process_metrics.as_ref() {
                            if let Some(health) = current.health.as_object_mut() {
                                if let Some(resident) = metrics.get("resident_bytes") {
                                    health.insert("resident_bytes".into(), resident.clone());
                                }
                                if let Some(cpu) = metrics.get("process_cpu_percent") {
                                    health.insert("process_cpu_percent".into(), cpu.clone());
                                }
                            }
                        }
                        // Campioni live per i grafici (CPU, RAM, disco, ramdisk).
                        let health = &current.health;
                        let cpu = health.get("cpu_percent").and_then(Value::as_f64).unwrap_or(0.0);
                        let memory_total = health.get("memory_total_bytes").and_then(Value::as_f64).unwrap_or(0.0);
                        let resident = health.get("resident_bytes").and_then(Value::as_f64).unwrap_or(0.0);
                        let ram = if memory_total > 0.0 { resident / memory_total * 100.0 } else { 0.0 };
                        let disk_free = health.get("disk_free_bytes").and_then(Value::as_f64).unwrap_or(0.0);
                        let ramdisk_free = health.get("ramdisk").and_then(|value| value.get("free_bytes")).and_then(Value::as_f64).unwrap_or(0.0);
                        for (key, value) in [("cpu", cpu), ("ram", ram), ("disk_free", disk_free), ("ramdisk_free", ramdisk_free), ("down", sample), ("up", sample_up)] {
                            let entry = current.live_history.entry(key.to_string()).or_default();
                            entry.push(value);
                            if entry.len() > 60 {
                                let drop = entry.len() - 60;
                                entry.drain(0..drop);
                            }
                        }
                        let previous = current.torrents.len() as i64;
                        current.torrent_delta = items.len() as i64 - previous;
                        current.torrents = items;
                        current.net_history.push(sample);
                        let len = current.net_history.len();
                        if len > 40 {
                            current.net_history.drain(0..len - 40);
                        }
                        current.net_history_up.push(sample_up);
                        let len_up = current.net_history_up.len();
                        if len_up > 40 {
                            current.net_history_up.drain(0..len_up - 40);
                        }
                        for (hash, rate) in samples {
                            let history = current.torrent_history.entry(hash).or_default();
                            history.push(rate);
                            if history.len() > 40 {
                                let drop = history.len() - 40;
                                history.drain(0..drop);
                            }
                        }
                    });
                }
            }
            // Anteprima live dei download HTTP (fumetti): così velocità e
            // progresso scorrono con la stessa cadenza dei torrent.
            if let Ok(value) = get("/api/comics/downloads").await {
                if let Some(items) = value.as_array().cloned() {
                    data.update(|current| current.comic_downloads = items);
                }
            }
        }
    });
    let reload = move || refresh.update(|value| *value += 1);
    let title = Signal::derive(move || tr(data, page_label(&page.get())));
    let stored_theme = web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item("rextto_theme").ok().flatten());
    let light = RwSignal::new(stored_theme.as_deref() == Some("light"));
    Effect::new(move |_| {
        let mode = if light.get() { "light" } else { "dark" };
        if let Some(document) = web_sys::window().and_then(|window| window.document()) {
            if let Some(element) = document.document_element() {
                let _ = element.set_attribute("data-theme", mode);
            }
        }
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            let _ = storage.set_item("rextto_theme", mode);
        }
    });
    let stored_scale = web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item("rextto_font_scale").ok().flatten())
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(100)
        .clamp(85, 140);
    let font_scale = RwSignal::new(stored_scale);
    Effect::new(move |_| {
        let percent = font_scale.get();
        if let Some(element) = web_sys::window().and_then(|window| window.document()).and_then(|document| document.document_element()) {
            let _ = element.set_attribute("style", &format!("font-size:{}px", 16 * percent / 100));
        }
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            let _ = storage.set_item("rextto_font_scale", &percent.to_string());
        }
    });
    let now_ms = RwSignal::new(js_sys::Date::now());
    spawn_local(async move {
        loop {
            TimeoutFuture::new(1_000).await;
            now_ms.set(js_sys::Date::now());
        }
    });
    let next_cycle = Signal::derive(move || {
        let (refresh, parsed) = data.with(|current| {
            let refresh = current
                .config
                .get("refresh_secs")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let parsed = current
                .status
                .get("last_cycle")
                .and_then(|cycle| cycle.get("last_started_at"))
                .and_then(Value::as_str)
                .map(js_sys::Date::parse)
                .filter(|value| !value.is_nan())
                .unwrap_or_else(|| now_ms.get());
            (refresh, parsed)
        });
        if refresh == 0 {
            return "—".to_string();
        }
        let remaining = parsed + refresh as f64 * 1000.0 - now_ms.get();
        tr(data, &format_remaining_seconds((remaining / 1000.0) as i64))
    });
    // Live torrent figures, always visible in the top bar. Lette con `with`
    // per non clonare tutto `Data` ad ogni aggiornamento.
    let live_dl = Signal::derive(move || {
        size_str(
            data.with(|current| {
                let torrents: f64 = current
                    .torrents
                    .iter()
                    .map(|item| value_f64(item, "download_rate"))
                    .sum();
                let http: f64 = current
                    .comic_downloads
                    .iter()
                    .map(|item| value_f64(item, "speed_bytes"))
                    .sum();
                torrents + http
            }),
        )
    });
    let live_ul = Signal::derive(move || {
        size_str(
            data.with(|current| {
                current
                    .torrents
                    .iter()
                    .map(|item| value_f64(item, "upload_rate"))
                    .sum()
            }),
        )
    });
    let live_cpu = Signal::derive(move || {
        data.with(|current| {
            current
                .health
                .get("process_cpu_percent")
                .and_then(Value::as_f64)
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| "—".into())
        })
    });
    let live_ram = Signal::derive(move || {
        data.with(|current| {
            size_str(
                current
                    .health
                    .get("resident_bytes")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
            )
        })
    });
    let live_count = Signal::derive(move || data.with(|current| current.torrents.len()));
    let live_peers = Signal::derive(move || {
        data.with(|current| {
            current
                .torrents
                .iter()
                .map(|item| value_f64(item, "num_peers"))
                .sum::<f64>() as i64
        })
    });
    let live_seeds = Signal::derive(move || {
        data.with(|current| {
            current
                .torrents
                .iter()
                .map(|item| value_f64(item, "num_seeds"))
                .sum::<f64>() as i64
        })
    });
    // Su mobile la voce "Sistema" è comprimibile: evita che la barra di
    // navigazione diventi una fila interminabile di voci poco usate.
    let nav_more = RwSignal::new(false);

    view! {
        <Title text="Rextto" />
        <div class="app-shell">
                <aside class="sidebar">
                    <div class="brand">
                        <span class="brand-mark">R</span>
                        <div><strong>Rextto</strong><small title=ctx_tr("Versione applicazione")>{move || data.with(|current| format!("Media daemon · v{}", text(&current.status, "version", "?")))}</small></div>
                    </div>
                    <nav>
                        <SettingsSearch />
                        {NAV_GROUPS.iter().enumerate().map(|(index, (_group, items))| {
                            let items = *items;
                            let is_system = index + 1 == NAV_GROUPS.len();
                            view! {
                                <div class="nav-group" class:system-group=is_system class:open=move || nav_more.get()>
                                    {is_system.then(|| view! {
                                        <button class="nav-more" on:click=move |_| nav_more.update(|open| *open = !*open)>
                                            <span>{ctx_tr("Sistema")}</span>
                                            <span>{move || if nav_more.get() { "▲" } else { "▾" }}</span>
                                        </button>
                                    })}
                                    {items.iter().map(|(id, label)| {
                                        let id = *id;
                                        let item_label = *label;
                                        let optional = is_optional_nav(id);
                                        let mobile_hidden = is_mobile_hidden_nav(id);
                                        let mobile_primary = is_mobile_primary_nav(id);
                                        view! {
                                            <button class="nav-item" class:nav-optional=optional class:nav-mobile-hidden=mobile_hidden class:nav-mobile-primary=mobile_primary class:active=move || page.get() == id on:click=move |_| page.set(id.to_string())>
                                                <span>{move || tr(data, item_label)}</span>
                                                <SidebarCount page=page id=id data />
                                            </button>
                                        }
                                    }).collect_view()}
                                </div>
                            }
                        }).collect_view()}
                    </nav>
                    <div class="sidebar-foot">
                        <span class="pulse"></span>
                         <span>{move || if data.get().status.get("dry_run").and_then(Value::as_bool).unwrap_or(true) { "dry-run".to_string() } else { tr(data, "attivo") }}</span>
                    </div>
                </aside>
                <main class="main-shell">
                    <header class="topbar">
                        <div class="topbar-title">
                            <div class="crumb">{ctx_tr("REXTTO")}</div>
                            <h1>{move || title.get()}</h1>
                        </div>
                        <div class="topbar-message">
                            <Show when=move || !data.get().error.is_empty()>
                                <div class="alert inline" title=move || data.get().error.clone()>{move || data.get().error.clone()}</div>
                            </Show>
                            <Show when=move || !data.get().notice.is_empty()>
                                <div class="notice inline" title=move || data.get().notice.clone()>{move || data.get().notice.clone()}</div>
                            </Show>
                        </div>
                        <div class="top-actions">
                            <Show when=move || busy.get()><span class="loading">{ctx_tr("Aggiornamento…")}</span></Show>
                             <div class="top-metric" title=ctx_tr("CPU del processo Rextto")><span class="top-metric-label">{ctx_tr("CPU")}</span><strong>{move || live_cpu.get()}</strong></div>
                             <div class="top-metric top-metric-ram" title=ctx_tr("RAM residente del processo Rextto")><span class="top-metric-label">{ctx_tr("RAM")}</span><strong>{move || live_ram.get()}</strong></div>
                             <div class="top-metric top-metric-rate" title=ctx_tr("Velocità di download")><span class="top-metric-label">{move || format!("↓ {}", tr(data, "Download"))}</span><strong>{move || format!("{}/s", live_dl.get())}</strong></div>
                             <div class="top-metric top-metric-rate" title=ctx_tr("Velocità di upload")><span class="top-metric-label">{move || format!("↑ {}", tr(data, "Upload"))}</span><strong>{move || format!("{}/s", live_ul.get())}</strong></div>
                             <div class="top-metric" title=ctx_tr("Torrent nella sessione")><span class="top-metric-label">{ctx_tr("Torrent")}</span><strong>{move || live_count.get()}</strong></div>
                             <div class="top-metric" title=ctx_tr("Peer connessi / Seed")><span class="top-metric-label">{ctx_tr("P/S")}</span><strong>{move || format!("{}/{}", live_peers.get(), live_seeds.get())}</strong></div>
                             <div class="top-metric top-metric-cycle" title=move || tr(data, "Tempo stimato al prossimo ciclo automatico")><span class="top-metric-label">{ctx_tr("Prossimo ciclo")}</span><strong>{move || next_cycle.get()}</strong></div>
                            <button class="btn" title=ctx_tr("Testo più piccolo") on:click=move |_| font_scale.update(|value| *value = (*value - 5).max(85))>{ctx_tr("A−")}</button>
                             <button class="btn" title=ctx_tr("Dimensione testo predefinita (100%)") on:click=move |_| font_scale.set(100)>{move || format!("{} {}%", tr(data, "Testo"), font_scale.get())}</button>
                            <button class="btn" title=ctx_tr("Testo più grande") on:click=move |_| font_scale.update(|value| *value = (*value + 5).min(140))>{ctx_tr("A+")}</button>
                             <select class="lang-select" title=ctx_tr("Lingua dell'interfaccia") prop:value=move || data.get().language on:change=move |event| {
                                 let chosen = event_target_value(&event);
                                 spawn_local(async move {
                                     if send("POST", "/api/i18n/active", Some(json!({"lang": chosen}))).await.is_ok() {
                                         data.update(|current| current.language = chosen.clone());
                                        reload();
                                    }
                                 });
                             }>
                                 <option value="it">{ctx_tr("Italiano")}</option>
                                 <option value="en">{ctx_tr("English")}</option>
                             </select>
                             <button class="btn" title=move || tr(data, "Cambia tra tema chiaro e scuro") on:click=move |_| light.update(|value| *value = !*value)>{move || tr(data, if light.get() { "Tema scuro" } else { "Tema chiaro" })}</button>
                            <button class="btn" title=move || tr(data, "Ricarica i dati mostrati") on:click=move |_| reload()>{move || tr(data, "Aggiorna")}</button>
                            {move || {
                                let dry = data.get().status.get("dry_run").and_then(Value::as_bool).unwrap_or(true);
                                if dry {
                                    view! { <span class="mode-badge dry">{move || tr(data, "Dry-run · solo test")}</span> }.into_any()
                                } else {
                                    view! { <span class="mode-badge active">{move || tr(data, "Attivo")}</span> }.into_any()
                                }
                            }}
                            <span class="system-pill"><span class="pulse"></span>{move || text(&data.get().health, "status", "offline")}</span>
                        </div>
                    </header>
                    <div class="content">
                        <Show when=move || page.get() == "dashboard"><Dashboard data page next_cycle now_ms /></Show>
                        <Show when=move || page.get() == "downloads"><Downloads data /></Show>
                        <Show when=move || page.get() == "series"><Library data mode="series" /></Show>
                        <Show when=move || page.get() == "movies"><Library data mode="movies" /></Show>
                        <Show when=move || page.get() == "search"><Discovery data page /></Show>
                        <Show when=move || page.get() == "comics"><ComicsView data /></Show>
                        <Show when=move || page.get() == "archive"><ArchiveView data /></Show>
                        <Show when=move || page.get() == "settings"><SettingsView data /></Show>
                        <Show when=move || page.get() == "integrations"><IntegrationsView data /></Show>
                        <Show when=move || page.get() == "maintenance"><MaintenanceView data /></Show>
                        <Show when=move || page.get() == "logs"><LogsView data /></Show>
                        <Show when=move || page.get() == "health"><HealthView data /></Show>
                        <Show when=move || page.get() == "gaps"><MissingView data /></Show>
                        <Show when=move || page.get() == "blocklist"><BlocklistView data /></Show>
                        <Show when=move || page.get() == "manual"><ManualView data /></Show>
                        <Show when=move || page.get() == "license"><LicenseView /></Show>
                    </div>
                </main>
                <ToastHost data />
                <SettingsSearchOverlay page=page />
            </div>
    }
}

#[component]
fn SettingsSearch() -> impl IntoView {
    let state = use_context::<SettingsSearchState>();
    view! {
        <button
            type="button"
            class="nav-item nav-search"
            title=ctx_tr("Cerca tra tutte le impostazioni e vai al campo")
            on:click=move |_| {
                if let Some(state) = state {
                    state.open.set(true);
                }
            }
        >
            <span class="nav-search-icon">"🔍"</span>
            <span>{ctx_tr("Cerca impostazioni")}</span>
        </button>
    }
}

#[component]
fn SettingsSearchOverlay(page: RwSignal<String>) -> impl IntoView {
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let nav = use_context::<SettingsNav>();
    let open = use_context::<SettingsSearchState>()
        .map(|state| state.open)
        .unwrap_or_else(|| RwSignal::new(false));
    let query = RwSignal::new(String::new());
    let input_ref = NodeRef::<leptos::html::Input>::new();
    let results = Signal::derive(move || search_settings(data, &query.get()));
    let choose = move |tab: &'static str, key: &'static str| {
        if let Some(nav) = nav {
            nav.tab.set(tab.to_string());
            nav.focus
                .set(if key.is_empty() { None } else { Some(key.to_string()) });
        }
        page.set("settings".to_string());
        query.set(String::new());
        open.set(false);
    };
    let has_query = move || query.get().trim().len() >= 2;
    let has_results = move || !results.get().is_empty();
    Effect::new(move |_| {
        if open.get() {
            if let Some(input) = input_ref.get() {
                let _ = input.focus();
            }
        }
    });
    view! {
        <Show when=move || open.get()>
            <div class="modal-backdrop" on:click=move |_| open.set(false)>
                <div
                    class="modal settings-search-modal"
                    on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()
                >
                    <div class="modal-head">
                        <h3>{ctx_tr("Cerca impostazioni")}</h3>
                        <button class="btn sm" type="button" on:click=move |_| open.set(false)>
                            {ctx_tr("Chiudi")}
                        </button>
                    </div>
                    <div class="modal-body">
                        <input
                            node_ref=input_ref
                            type="search"
                            prop:value=query
                            on:input=move |event| query.set(event_target_value(&event))
                            on:keydown=move |event| {
                                if event.key() == "Escape" {
                                    open.set(false);
                                } else if event.key() == "Enter" {
                                    if let Some(hit) = results.get_untracked().into_iter().next() {
                                        choose(hit.tab, hit.key);
                                    }
                                }
                            }
                            placeholder=ctx_tr("Scrivi almeno 2 lettere (es. proxy, seed, lingua)…")
                        />
                        <div class="settings-search-results">
                            <Show
                                when=has_query
                                fallback=move || view! { <span class="muted">{ctx_tr("Scrivi per cercare tra le impostazioni.")}</span> }
                            >
                                <Show
                                    when=has_results
                                    fallback=move || view! { <span class="muted">{ctx_tr("Nessuna impostazione trovata.")}</span> }
                                >
                                    {move || results.get().into_iter().map(|hit| {
                                        let tab = hit.tab;
                                        let key = hit.key;
                                        view! {
                                            <button
                                                type="button"
                                                class="settings-search-hit"
                                                on:click=move |_| choose(tab, key)
                                            >
                                                <span class="settings-search-label">{hit.label}</span>
                                                <span class="badge">{settings_tab_label(tab)}</span>
                                            </button>
                                        }
                                    }).collect_view()}
                                </Show>
                            </Show>
                        </div>
                    </div>
                </div>
            </div>
        </Show>
    }
}

#[component]
fn SidebarCount(page: RwSignal<String>, id: &'static str, data: RwSignal<Data>) -> impl IntoView {
    let count = Signal::derive(move || {
        data.with(|current| match id {
            "downloads" => {
                let http = current
                    .comic_downloads
                    .iter()
                    .filter(|item| text(item, "status", "") == "downloading")
                    .count();
                current.torrents.len() + http
            }
            "series" => array(&current.library, "series").len(),
            "movies" => array(&current.library, "movies").len(),
            "comics" => current.comics.len(),
            "gaps" => current.gaps.len(),
            "blocklist" => current.blocklist.len(),
            _ => 0,
        })
    });
    view! {
        <Show when=move || { count.get() > 0 }>
            <span class="nav-count">{move || count.get()}</span>
        </Show>
        <span style="display:none">{move || page.get() == id}</span>
    }
}

/* ------------------------------------------------------------------ */
/* Reusable pieces                                                     */
/* ------------------------------------------------------------------ */

#[component]
fn Metric(
    label: &'static str,
    value: Signal<String>,
    tone: &'static str,
    #[prop(optional)] sub: Option<Signal<String>>,
) -> impl IntoView {
    let label = ctx_tr(label);
    view! {
        <article class=format!("metric {tone}")>
            <span>{label}</span>
            <strong>{value}</strong>
            {sub.map(|signal| view! { <small class="metric-sub">{signal}</small> })}
        </article>
    }
}

#[component]
fn Panel(title: &'static str, children: Children) -> impl IntoView {
    let heading = ctx_tr(title);
    view! {
        <section class="panel">
            <div class="panel-head"><h3>{heading}</h3><span class="hint">{ctx_tr("REXTTO")}</span></div>
            <div class="panel-body">{children()}</div>
        </section>
    }
}

#[component]
fn Empty(text: &'static str) -> impl IntoView {
    let message = ctx_tr(text);
    view! { <div class="empty">{message}</div> }
}

#[component]
fn SettingGroup(title: &'static str, children: Children) -> impl IntoView {
    let heading = ctx_tr(title);
    view! {
        <section class="setting-group">
            <h4>{heading}</h4>
            <div class="setting-group-body">{children()}</div>
        </section>
    }
}

#[component]
fn BrowseButton(value: RwSignal<String>) -> impl IntoView {
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let open = RwSignal::new(false);
    let current = RwSignal::new(String::new());
    let parent = RwSignal::new(Option::<String>::None);
    let dirs = RwSignal::new(Vec::<Value>::new());
    let new_name = RwSignal::new(String::new());
    let browse = move |path: String| {
        let current = current;
        let parent = parent;
        let dirs = dirs;
        spawn_local(async move {
            let url = if path.trim().is_empty() {
                "/api/browse_dir".to_string()
            } else {
                format!("/api/browse_dir?path={}", urlencoding::encode(&path))
            };
            if let Ok(response) = get(&url).await {
                current.set(text(&response, "path", ""));
                parent.set(response.get("parent").and_then(Value::as_str).map(str::to_owned));
                dirs.set(array(&response, "dirs"));
            }
        });
    };
    view! {
        <button type="button" class="btn" title=ctx_tr("Sfoglia le cartelle") on:click=move |_| {
            browse(value.get());
            open.set(true);
        }>{ctx_tr("Sfoglia")}</button>
        <Show when=move || open.get()>
            <div class="modal-backdrop" on:click=move |_| open.set(false)>
                <div class="modal path-modal" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                    <div class="modal-head">
                        <strong>{ctx_tr("Sfoglia cartelle")}</strong>
                        <button type="button" class="btn sm" on:click=move |_| open.set(false)>{ctx_tr("Chiudi")}</button>
                    </div>
                    <div class="modal-body">
                        <div class="toolbar" style="margin-bottom:10px">
                            <button type="button" class="btn sm" title=ctx_tr("Vai alla cartella superiore") on:click=move |_| {
                                if let Some(value) = parent.get() { browse(value); }
                            }>{ctx_tr("↑ Su")}</button>
                            <input type="text" class="mono" prop:value=current title=ctx_tr("Percorso corrente: modificalo e premi Invio per navigare") on:change=move |event| browse(event_target_value(&event)) />
                            <button type="button" class="btn sm primary" title=ctx_tr("Usa questa cartella") on:click=move |_| {
                                value.set(current.get());
                                open.set(false);
                            }>{ctx_tr("Seleziona")}</button>
                            <button type="button" class="btn sm" title=ctx_tr("Crea una nuova cartella dentro quella corrente") on:click=move |_| {
                                let base = current.get();
                                let provided = new_name.get().trim().to_string();
                                spawn_local(async move {
                                    let name = if provided.is_empty() {
                                        let Some(window) = web_sys::window() else { return };
                                        let Ok(Some(name)) = window.prompt_with_message(&tr(data, "Nome nuova cartella")) else { return };
                                        name.trim().to_string()
                                    } else {
                                        provided
                                    };
                                    if name.is_empty() { return; }
                                    let path = format!("{}/{}", base.trim_end_matches('/'), name);
                                    if send("POST", "/api/mkdir", Some(json!({"path": path.clone()}))).await.is_ok() {
                                        value.set(path.clone());
                                        new_name.set(String::new());
                                        browse(path);
                                        open.set(false);
                                    }
                                });
                            }>{ctx_tr("Crea cartella")}</button>
                        </div>
                        <div class="stack path-list">
                            {move || dirs.get().iter().cloned().map(|dir| {
                                let path = dir.as_str().unwrap_or_default().to_string();
                                let target = path.clone();
                                view! {
                                    <button type="button" class="list-item path-item" on:click=move |_| browse(target.clone())>
                                        <span class="mono truncate">{path}</span>
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                        <Show when=move || dirs.get().is_empty()><Empty text="Nessuna sottocartella." /></Show>
                        <div class="toolbar" style="margin-top:10px">
                            <input prop:value=new_name title=ctx_tr("Nome della nuova cartella da creare nella cartella corrente") on:input=move |event| new_name.set(event_target_value(&event)) placeholder=ctx_tr("Nuova cartella") />
                            <button type="button" class="btn sm primary" title=ctx_tr("Crea la cartella e selezionala") on:click=move |_| {
                                let name = new_name.get().trim().to_string();
                                if name.is_empty() { return; }
                                let base = current.get();
                                let path = format!("{}/{}", base.trim_end_matches('/'), name);
                                spawn_local(async move {
                                    if send("POST", "/api/mkdir", Some(json!({"path": path.clone()}))).await.is_ok() {
                                        value.set(path.clone());
                                        new_name.set(String::new());
                                        browse(path);
                                        open.set(false);
                                    }
                                });
                            }>{ctx_tr("Crea e usa")}</button>
                        </div>
                    </div>
                </div>
            </div>
        </Show>
    }
}

#[component]
fn PathPicker(
    label: &'static str,
    value: RwSignal<String>,
    placeholder: &'static str,
) -> impl IntoView {
    let label_text = ctx_tr(label);
    view! {
        <div class="field">
            <span>{label_text}</span>
            <div class="path-picker">
                <input prop:value=value title=label on:input=move |event| value.set(event_target_value(&event)) placeholder=ctx_tr(placeholder) />
                <BrowseButton value />
            </div>
        </div>
    }
}

/* ------------------------------------------------------------------ */
/* Dashboard                                                           */
/* ------------------------------------------------------------------ */

#[component]
fn Dashboard(data: RwSignal<Data>, page: RwSignal<String>, next_cycle: Signal<String>, now_ms: RwSignal<f64>) -> impl IntoView {
    let last = Signal::derive(move || data.get().status.get("last_cycle").cloned().unwrap_or_default());
    let series = Signal::derive(move || array(&data.get().library, "series"));
    let movies = Signal::derive(move || array(&data.get().library, "movies"));
    let series_enabled = Signal::derive(move || {
        series
            .get()
            .iter()
            .filter(|item| item.get("enabled").and_then(Value::as_bool).unwrap_or(false))
            .count()
    });
    let movies_downloaded = Signal::derive(move || {
        value_at(&data.get().db_info, &["info", "downloaded_movies"])
    });
    let episodes_downloaded = Signal::derive(move || {
        value_at(&data.get().db_info, &["info", "downloaded_episodes"])
    });
    let down_rate = Signal::derive(move || {
        data.get().torrents.iter().filter_map(|item| item.get("download_rate").and_then(Value::as_f64)).sum::<f64>()
    });
    let up_rate = Signal::derive(move || {
        data.get().torrents.iter().filter_map(|item| item.get("upload_rate").and_then(Value::as_f64)).sum::<f64>()
    });
    let ram_percent = Signal::derive(move || {
        let total = data.get().health.get("memory_total_bytes").and_then(Value::as_f64).unwrap_or(0.0);
        let resident = data.get().health.get("resident_bytes").and_then(Value::as_f64).unwrap_or(0.0);
        if total > 0.0 { format!("{:.1}%", resident / total * 100.0) } else { "-".into() }
    });
    let consumption = Signal::derive(move || data.get().stats.get("consumption").cloned().unwrap_or_default());
    // Cestino: contenuto caricato solo all'apertura della modale.
    let trash_open = RwSignal::new(false);
    let trash_items = RwSignal::new(Vec::<Value>::new());
    let trash_total = RwSignal::new(0_i64);
    let trash_path = RwSignal::new(String::new());
    let trash_busy = RwSignal::new(false);
    let load_trash = move || {
        spawn_local(async move {
            if let Ok(value) = get("/api/trash").await {
                trash_items.set(array(&value, "items"));
                trash_total.set(value.get("total_bytes").and_then(Value::as_i64).unwrap_or(0));
                trash_path.set(text(&value, "path", ""));
            }
        });
    };
    let search = use_context::<GlobalSearch>().expect("global search context");
    let global_query = search.query;
    let global_results = search.results;
    let global_loading = search.loading;
    let global_searched = search.searched;
    let global_elapsed = search.elapsed;
    let feed_items = RwSignal::new(Vec::<Value>::new());
    let feed_loading = RwSignal::new(false);
    let feed_loaded = RwSignal::new(false);
    let comics_next_cycle = Signal::derive(move || {
        let current = data.get();
        let interval = current.comics_check_interval as i64;
        if interval == 0 {
            return tr(data, "ad ogni ciclo");
        }
        if current.comics_last_check_ts <= 0 {
            return tr(data, "in attesa");
        }
        let now = (now_ms.get() / 1000.0) as i64;
        tr(data, &format_remaining_seconds(current.comics_last_check_ts + interval - now))
    });
    view! {
        <div class="view">
            <div class="mode-strip">
                {move || {
                    let dry = data.get().status.get("dry_run").and_then(Value::as_bool).unwrap_or(true);
                    if dry {
                        view! { <span class="mode-badge dry" title=ctx_tr("Nessun download reale: Rextto analizza i candidati ma non scarica.")>{ctx_tr("DRY-RUN")}</span> }.into_any()
                    } else {
                        view! { <span class="mode-badge active" title=ctx_tr("Rextto sta scaricando e archiviando i contenuti.")>{ctx_tr("ATTIVA")}</span> }.into_any()
                    }
                }}
                <form class="global-search" on:submit=move |event| {
                    event.prevent_default();
                    let query = global_query.get();
                    if query.trim().is_empty() { return; }
                    let results = global_results;
                    let loading = global_loading;
                    let searched = global_searched;
                    let elapsed = global_elapsed;
                    loading.set(true);
                    searched.set(false);
                    elapsed.set(0);
                    spawn_local(async move {
                        while loading.get_untracked() {
                            TimeoutFuture::new(1000).await;
                            if loading.get_untracked() {
                                elapsed.update(|value| *value += 1);
                            }
                        }
                    });
                    spawn_local(async move {
                        match send("POST", "/api/search", Some(json!({"query": query}))).await {
                            Ok(value) => results.set(array(&value, "results")),
                            Err(error) => data.update(|current| current.error = error),
                        }
                        loading.set(false);
                        searched.set(true);
                    });
                }>
                    <input prop:value=global_query on:input=move |event| global_query.set(event_target_value(&event)) placeholder=ctx_tr("Cerca in archivio + indexer + motori web…") title=ctx_tr("Cerca contemporaneamente nell'archivio, negli indexer e nei motori web") />
                     <button class="btn primary" disabled=move || global_loading.get()>{move || if global_loading.get() { tr(data, "Cerco…") } else { tr(data, "Cerca") }}</button>
                </form>
                <div class="toolbar cycle-actions">
                    <span class="cycle-label">{move || tr(data, "Avvia ciclo")}</span>
                    <button class="btn primary" title=move || tr(data, "Cerca ora nuovi episodi, film e fumetti e riempi i gap") on:click=move |_| { run_post(data, "/api/run_now", None, "Ciclo completo avviato"); }>{move || tr(data, "Tutto")}</button>
                    <button class="btn" title=move || tr(data, "Ciclo limitato alle serie TV") on:click=move |_| { run_post(data, "/api/run_now?domain=series", None, "Ciclo serie avviato"); }>{move || tr(data, "Serie TV")}</button>
                    <button class="btn" title=move || tr(data, "Ciclo limitato ai film") on:click=move |_| { run_post(data, "/api/run_now?domain=movies", None, "Ciclo film avviato"); }>{move || tr(data, "Film")}</button>
                    <button class="btn" title=move || tr(data, "Ciclo limitato ai fumetti") on:click=move |_| { run_post(data, "/api/run_now?domain=comics", None, "Ciclo fumetti avviato"); }>{move || tr(data, "Fumetti")}</button>
                    <button class="btn" title=move || tr(data, "Crea ora uno snapshot di backup dei database") on:click=move |_| { run_post(data, "/api/backup", None, "Backup creato"); }>{move || tr(data, "Backup")}</button>
                </div>
            </div>
             <Show when=move || global_loading.get()>
                 <div class="search-status"><span class="spinner"></span>{move || format!("{} {}s", tr(data, "Interrogazione archivio, RSS, indexer e motori web…"), global_elapsed.get())}</div>
            </Show>
            <Show when=move || !global_loading.get() && global_searched.get() && global_results.get().is_empty()>
                <div class="notice">{ctx_tr("Nessun risultato. Prova un termine più corto o verifica le sorgenti in Impostazioni → Sorgenti.")}</div>
            </Show>
            <Show when=move || global_loading.get() && global_results.get().is_empty()>
                <div class="search-skeleton">
                    <div class="skeleton-line"></div>
                    <div class="skeleton-line"></div>
                    <div class="skeleton-line"></div>
                </div>
            </Show>
            <Show when=move || !global_results.get().is_empty()>
                <Panel title="Risultati ricerca">
                    <div class="table-wrap">
                        <table class="data-table">
                            <thead><tr><th>{ctx_tr("Release")}</th><th>{ctx_tr("Sorgente")}</th><th>{ctx_tr("Risoluzione")}</th><th>{ctx_tr("Codec")}</th><th></th></tr></thead>
                            <tbody>
                                {move || global_results.get().iter().cloned().map(|item| {
                                    let release = item.clone();
                                    let quality = item.get("quality").cloned().unwrap_or_default();
                                    view! {
                                        <tr>
                                            <td class="truncate">{text(&item, "title", "Release")}</td>
                                            <td class="muted">{text(&item, "source", "-")}</td>
                                            <td class="muted">{text(&quality, "resolution", "-")}</td>
                                            <td class="muted">{text(&quality, "codec", "-")}</td>
                                            <td><button class="btn sm primary" on:click=move |_| { let release = release.clone(); run_post(data, "/api/search/add", Some(json!({"release": release})), "Release accodata"); }>{ctx_tr("Accoda")}</button></td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                </Panel>
            </Show>
            <Panel title="Prossima ricerca automatica">
                 <div class="stack next-search-grid">
                     <StatLine label="Serie TV" value=next_cycle />
                     <StatLine label="Film" value=next_cycle />
                     <StatLine label="Fumetti" value=comics_next_cycle />
                </div>
            </Panel>
            <div class="metrics">
                <Metric label="Serie TV configurate" value=Signal::derive(move || series.get().len().to_string()) tone="mint"
                     sub=Signal::derive(move || format!("{} {} · {} {}", series_enabled.get(), tr(data, "abilitate"), series.get().len().saturating_sub(series_enabled.get()), tr(data, "in pausa"))) />
                <Metric label="Film configurati" value=Signal::derive(move || movies.get().len().to_string()) tone="blue"
                     sub=Signal::derive(move || format!("{} {}", movies_downloaded.get(), tr(data, "scaricati"))) />
                <Metric label="File scaricati" value=Signal::derive(move || episodes_downloaded.get().to_string()) tone="amber"
                     sub=Signal::derive(move || format!("{} {} · {}", movies_downloaded.get(), tr(data, "film"), size(&consumption.get(), "total_bytes"))) />
                <Metric label="Spazio libero" value=Signal::derive(move || size(&data.get().health, "disk_free_bytes")) tone="violet"
                    sub=Signal::derive(move || {
                        let free = data.get().health.get("disk_free_bytes").and_then(Value::as_f64).unwrap_or(0.0);
                        let total = data.get().health.get("disk_total_bytes").and_then(Value::as_f64).unwrap_or(0.0);
                        if total > 0.0 { format!("{:.0}% di {}", free / total * 100.0, size_str(total)) } else { String::new() }
                    }) />
                <Metric label="Magnet in archivio" value=Signal::derive(move || data.get().archive_total.to_string()) tone="blue"
                     sub=Signal::derive(move || format!("{} {} · {} {}", series.get().len(), tr(data, "serie"), movies.get().len(), tr(data, "film monitorati"))) />
                <Metric label="Visti nei feed" value=Signal::derive(move || data.get().status.get("seen").and_then(|seen| seen.get("groups")).and_then(Value::as_i64).unwrap_or(0).to_string()) tone="violet"
                    sub=Signal::derive(move || {
                        let seen = data.get().status.get("seen").cloned().unwrap_or_default();
                         format!("{} {} · {} {}", number(&seen, "movies"), tr(data, "film"), number(&seen, "series"), tr(data, "serie"))
                    }) />
                <Metric label="Torrent in sessione" value=Signal::derive(move || data.get().torrents.len().to_string()) tone="mint"
                    sub=Signal::derive(move || {
                        let torrents = data.get().torrents;
                        let downloading = torrents.iter().filter(|item| text(item, "state", "").contains("download")).count();
                        let seeding = torrents.iter().filter(|item| text(item, "state", "").contains("seed")).count();
                        let queued = torrents.iter().filter(|item| text(item, "state", "").contains("coda") || text(item, "state", "").contains("queued")).count();
                        let delta = data.get().torrent_delta;
                        let arrow = if delta > 0 { format!(" ▲{delta}") } else if delta < 0 { format!(" ▼{}", delta.abs()) } else { String::new() };
                         format!("{downloading} {} · {seeding} {} · {queued} {}{arrow}", tr(data, "scarico"), tr(data, "seed"), tr(data, "coda"))
                    }) />
            </div>
            <Panel title="Rete e download attivi">
                <div class="net-panel">
                    <Metric label="Load average" value=Signal::derive(move || data.get().health.get("load_average").and_then(Value::as_f64).map(|value| format!("{value:.2}")).unwrap_or_else(|| "-".into())) tone="blue" />
                    <Metric label="CPU sistema" value=Signal::derive(move || data.get().health.get("cpu_percent").and_then(Value::as_f64).map(|value| format!("{value:.0}%")).unwrap_or_else(|| "-".into())) tone="blue" />
                    <Metric label="RAM processo" value=ram_percent tone="mint" />
                    <div class="sparkline-wrap">
                        <div class="sparkline-legend" title=ctx_tr("Velocità di rete in tempo reale e andamento degli ultimi 40 campioni (ogni 4s)")>
                            <span class="spark-down">"↓ "{move || format!("{}/s", size_str(down_rate.get()))}</span>
                            <span class="spark-up">"↑ "{move || format!("{}/s", size_str(up_rate.get()))}</span>
                        </div>
                        <svg class="sparkline" viewBox="0 0 200 44" preserveAspectRatio="none" aria-label="Andamento velocità di rete (download e upload)">
                            <polyline class="spark-down" points=move || {
                                let metrics = data.get();
                                let max = metrics.net_history.iter().chain(metrics.net_history_up.iter()).cloned().fold(0.0_f64, f64::max);
                                sparkline_points_scaled(&metrics.net_history, max, 200.0, 44.0)
                            } fill="none" />
                            <polyline class="spark-up" points=move || {
                                let metrics = data.get();
                                let max = metrics.net_history.iter().chain(metrics.net_history_up.iter()).cloned().fold(0.0_f64, f64::max);
                                sparkline_points_scaled(&metrics.net_history_up, max, 200.0, 44.0)
                            } fill="none" />
                        </svg>
                        <span class="muted">{ctx_tr("Ultimi 40 campioni · 4s")}</span>
                    </div>
                </div>
            </Panel>
            <div class="grid-2">
                <Panel title="Azioni rapide">
                    <div class="toolbar">
                        <button class="btn primary" on:click=move |_| { run_post(data, "/api/run_now", None, "Ciclo completo avviato"); }>{ctx_tr("Ricontrolla tutto")}</button>
                        <button class="btn" on:click=move |_| { run_post(data, "/api/run_now?domain=series", None, "Ciclo serie avviato"); }>{ctx_tr("Serie TV")}</button>
                        <button class="btn" on:click=move |_| { run_post(data, "/api/run_now?domain=movies", None, "Ciclo film avviato"); }>{ctx_tr("Film")}</button>
                        <button class="btn" on:click=move |_| { run_post(data, "/api/run_now?domain=comics", None, "Ciclo fumetti avviato"); }>{ctx_tr("Fumetti")}</button>
                        <button class="btn" on:click=move |_| { run_post(data, "/api/backup", None, "Backup creato"); }>{ctx_tr("Backup")}</button>
                        <button class="btn" title=ctx_tr("Mostra il contenuto del cestino e permette di eliminare i file singolarmente o tutti insieme") on:click=move |_| { trash_open.set(true); load_trash(); }>{ctx_tr("Mostra/Svuota cestino")}</button>
                    </div>
                    <div class="stack" style="margin-top:12px">
                        <StatLine label="Ultimo ciclo" value=Signal::derive(move || short_datetime(&text(&last.get(), "last_started_at", ""))) />
                        <StatLine label="Candidati / avviati" value=Signal::derive(move || format!("{} / {}", number(&last.get(), "candidates"), number(&last.get(), "downloads_started"))) />
                        <StatLine label="Gap riempiti" value=Signal::derive(move || number(&last.get(), "gaps_filled")) />
                        <StatLine label="Feed RSS / Indexer" value=Signal::derive(move || format!("{} / {}", array(&data.get().config, "feed_urls").len(), array(&data.get().config, "indexers").len())) />
                         <StatLine label="Cestino" value=Signal::derive(move || tr_format(data, "{size} · {count} file", &[("{size}", size(&data.get().health, "trash_bytes")), ("{count}", number(&data.get().health, "trash_file_count"))])) />
                    </div>
                </Panel>
                <Panel title="Consumo banda e spazio disco">
                    <div class="stack">
                        <StatLine label="Consumo totale" value=Signal::derive(move || size(&consumption.get(), "total_bytes")) />
                        <StatLine label="Ultimi 30 giorni" value=Signal::derive(move || size(&consumption.get(), "last_30_days_bytes")) />
                        <StatLine label="Ultimi 7 giorni" value=Signal::derive(move || size(&consumption.get(), "last_7_days_bytes")) />
                        <StatLine label="Disco totale" value=Signal::derive(move || size(&data.get().health, "disk_total_bytes")) />
                        <StatLine label="Trash" value=Signal::derive(move || size(&data.get().health, "trash_bytes")) />
                    </div>
                    <div class="stack" style="margin-top:12px">
                        <span class="muted" title=ctx_tr("Filesystem montati con spazio libero e percentuale usata")>{ctx_tr("Dischi")}</span>
                        {move || data.get().health.get("disks").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().map(|disk| {
                            let mount = text(&disk, "mount", "-");
                            let mount_hint = mount.clone();
                            let total = disk.get("total_bytes").and_then(Value::as_f64).unwrap_or(0.0);
                            let free = disk.get("free_bytes").and_then(Value::as_f64).unwrap_or(0.0);
                            let used = if total > 0.0 { (total - free) / total * 100.0 } else { 0.0 };
                            view! {
                                <div class="disk-row" title=format!("{} · {}", text(&disk, "filesystem", "-"), mount_hint)>
                                    <span class="mono truncate">{mount}</span>
                                    <span class="muted">{tr_format(data, "{free} liberi / {total} · {used}%", &[("{free}", size_str(free)), ("{total}", size_str(total)), ("{used}", format!("{used:.0}"))])}</span>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </Panel>
            </div>
            <div class="grid-3">
                <Panel title="Prossime uscite">
                    <div class="list">
                        {move || data.get().calendar.iter().take(6).cloned().map(|item| {
                            let episode = item.get("episode").cloned().unwrap_or_default();
                            let poster = item.get("poster").and_then(Value::as_str).map(str::to_owned);
                            let series_name = text_tr(data, &item, "series", "Serie");
                            view! {
                                <div class="list-item">
                                    {match poster {
                                        Some(url) => view! { <img class="list-poster" src=url alt=series_name.clone() loading="lazy" /> }.into_any(),
                                        None => view! { <div class="list-poster placeholder">{ctx_tr("N/D")}</div> }.into_any(),
                                    }}
                                    <div style="flex:1"><strong>{series_name.clone()}</strong><small>{format!("S{}E{} · {}", number(&episode, "season_number"), number(&episode, "episode_number"), text(&episode, "air_date", "-"))}</small></div>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </Panel>
                <Panel title="Ultimi download">
                    <div class="list">
                        {move || data.get().recent_downloads.iter().take(6).cloned().map(|item| {
                            let is_series = text(&item, "kind", "") == "series";
                            let name_label = text(&item, "name", "-");
                            let label = if is_series {
                                format!("{} S{}E{}", name_label, number(&item, "season"), number(&item, "episode"))
                            } else {
                                name_label.clone()
                            };
                            let detail = if is_series {
                                format!("S{}E{} · {} · {}", number(&item, "season"), number(&item, "episode"), text(&item, "downloaded_at", "-"), size(&item, "size_bytes"))
                            } else {
                                format!("{} · {}", text(&item, "downloaded_at", "-"), size(&item, "size_bytes"))
                            };
                            let poster = item.get("poster").and_then(Value::as_str).map(str::to_owned);
                            view! {
                                <div class="list-item">
                                    {match poster {
                                        Some(url) => view! { <img class="list-poster" src=url alt=label.clone() loading="lazy" /> }.into_any(),
                                        None => view! { <div class="list-poster placeholder">{ctx_tr("N/D")}</div> }.into_any(),
                                    }}
                                    <div style="flex:1"><strong>{name_label}</strong><small>{detail}</small></div>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </Panel>
            </div>
            <Panel title="Ultimi trovati nelle sorgenti">
                <div class="toolbar" style="margin-bottom:10px">
                    <span class="hint">{ctx_tr("Release in archivio provenienti da RSS, indexer e motori web, abbinate agli elementi monitorati.")}</span>
                    <button class="btn" disabled=move || feed_loading.get() on:click=move |_| {
                        feed_loading.set(true);
                        let feed_items = feed_items;
                        let feed_loading = feed_loading;
                        let feed_loaded = feed_loaded;
                        spawn_local(async move {
                            match get("/api/feed/status").await {
                                Ok(value) => feed_items.set(array(&value, "items")),
                                Err(error) => data.update(|current| current.error = error),
                            }
                            feed_loaded.set(true);
                            feed_loading.set(false);
                        });
                    }>{move || if feed_loading.get() { tr(data, "Caricamento…") } else if feed_loaded.get() { tr(data, "Aggiorna risultati") } else { tr(data, "Carica risultati") }}</button>
                </div>
                <Show when=move || feed_loading.get()>
                    <div class="search-status"><span class="spinner"></span>{ctx_tr("Cerco le release archiviate per le serie e i film monitorati…")}</div>
                </Show>
                <Show when=move || feed_loaded.get() && !feed_loading.get() && feed_items.get().iter().all(|item| array(item, "matches").is_empty())>
                    <Empty text="Nessuna release recente nelle sorgenti per gli elementi monitorati." />
                </Show>
                <Show when=move || !feed_items.get().is_empty()>
                    <div class="table-wrap">
                        <table class="data-table">
                            <thead><tr><th>{ctx_tr("Monitorato")}</th><th>{ctx_tr("Tipo")}</th><th>{ctx_tr("Release")}</th><th>{ctx_tr("Fonte")}</th><th></th></tr></thead>
                            <tbody>
                                {move || feed_items.get().iter().cloned().flat_map(|item| {
                                    let name = text(&item, "name", "Libreria");
                                    let kind = if text(&item, "kind", "") == "movie" { tr(data, "Film") } else { tr(data, "Serie TV") };
                                    array(&item, "matches").into_iter().map(move |release| {
                                        let title = text(&release, "title", "Release");
                                        let magnet = text(&release, "magnet", "");
                                        let source = text_tr(data, &release, "source", "sorgente");
                                        let add_title = title.clone();
                                        let add_magnet = magnet.clone();
                                        let add_source = source.clone();
                                        view! {
                                            <tr>
                                                <td>{name.clone()}</td>
                                                <td class="muted">{kind.clone()}</td>
                                                <td class="truncate" title=title.clone()>{title.clone()}</td>
                                                <td class="muted">{source}</td>
                                                <td><button class="btn sm primary" disabled=magnet.is_empty() on:click=move |_| run_post(data, "/api/archive/add", Some(json!({"title": add_title.clone(), "magnet": add_magnet.clone(), "source": add_source.clone()})), "Release accodata")>{ctx_tr("Accoda")}</button></td>
                                            </tr>
                                        }
                                    })
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                </Show>
            </Panel>
            <div class="dashboard-explore-panel">
                <Panel title="Esplora">
                    <div class="toolbar">
                        <button class="btn" on:click=move |_| page.set("series".into())>{ctx_tr("📺 Serie TV")}</button>
                        <button class="btn" on:click=move |_| page.set("movies".into())>{ctx_tr("🎬 Film")}</button>
                        <button class="btn" on:click=move |_| page.set("search".into())>{ctx_tr("🔎 Esplora release")}</button>
                        <button class="btn" on:click=move |_| page.set("comics".into())>{ctx_tr("📚 Fumetti")}</button>
                        <button class="btn" on:click=move |_| page.set("archive".into())>{ctx_tr("📦 Archivio")}</button>
                        <button class="btn" on:click=move |_| page.set("maintenance".into())>{ctx_tr("🛠 Manutenzione")}</button>
                    </div>
                </Panel>
            </div>
            <Show when=move || trash_open.get()>
                <div class="modal-backdrop" on:click=move |_| trash_open.set(false)>
                    <div class="modal" style="width:min(900px,96vw)" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                        <div class="modal-head">
                            <strong>{ctx_tr("Cestino")}</strong>
                            <button type="button" class="btn sm" on:click=move |_| trash_open.set(false)>{ctx_tr("Chiudi")}</button>
                        </div>
                        <div class="modal-body">
                            <p class="muted mono truncate" title=move || trash_path.get()>{move || trash_path.get()}</p>
                            <div class="toolbar" style="margin:8px 0 10px">
                                <span class="muted">{move || tr_format(data, "{count} elementi · {size}", &[("{count}", trash_items.get().len().to_string()), ("{size}", size_str(trash_total.get() as f64))])}</span>
                                <button class="btn sm danger" disabled=move || trash_busy.get() || trash_items.get().is_empty() on:click=move |_| {
                                    trash_busy.set(true);
                                    spawn_local(async move {
                                        match send("POST", "/api/trash/delete", Some(json!({"all": true}))).await {
                                            Ok(value) => push_toast(data, "ok", tr_format(data, "Cestino svuotato: {count} elementi", &[("{count}", number(&value, "removed"))])),
                                            Err(error) => push_toast(data, "err", error),
                                        }
                                        if let Ok(value) = get("/api/trash").await {
                                            trash_items.set(array(&value, "items"));
                                            trash_total.set(value.get("total_bytes").and_then(Value::as_i64).unwrap_or(0));
                                        }
                                        trash_busy.set(false);
                                        trigger_refresh();
                                    });
                                }>{ctx_tr("Svuota tutto")}</button>
                                <button class="btn sm" disabled=move || trash_busy.get() on:click=move |_| load_trash()>{ctx_tr("Aggiorna")}</button>
                            </div>
                            <Show when=move || !trash_busy.get() && trash_items.get().is_empty()>
                                <Empty text="Cestino vuoto." />
                            </Show>
                            <div class="table-wrap">
                                <table class="data-table">
                                    <thead><tr><th>{ctx_tr("Nome")}</th><th>{ctx_tr("Tipo")}</th><th>{ctx_tr("Dimensione")}</th><th></th></tr></thead>
                                    <tbody>
                                        {move || trash_items.get().into_iter().map(|item| {
                                            let name = text(&item, "name", "-");
                                            let name_title = name.clone();
                                            let is_dir = item.get("is_dir").and_then(Value::as_bool).unwrap_or(false);
                                            let delete_name = name.clone();
                                            view! {
                                                <tr>
                                                    <td class="truncate mono" title=name_title>{name}</td>
                                                     <td class="muted">{if is_dir { tr(data, "cartella") } else { tr(data, "file") }}</td>
                                                    <td class="numeric">{size(&item, "size_bytes")}</td>
                                                    <td>
                                                        <button class="btn sm danger" disabled=move || trash_busy.get() on:click=move |_| {
                                                            let name = delete_name.clone();
                                                            trash_busy.set(true);
                                                            spawn_local(async move {
                                                                match send("POST", "/api/trash/delete", Some(json!({"names": [name]}))).await {
                                                                     Ok(_) => push_toast(data, "ok", tr(data, "Elemento eliminato dal cestino")),
                                                                    Err(error) => push_toast(data, "err", error),
                                                                }
                                                                if let Ok(value) = get("/api/trash").await {
                                                                    trash_items.set(array(&value, "items"));
                                                                    trash_total.set(value.get("total_bytes").and_then(Value::as_i64).unwrap_or(0));
                                                                }
                                                                trash_busy.set(false);
                                                                trigger_refresh();
                                                            });
                                                        }>{ctx_tr("Elimina")}</button>
                                                    </td>
                                                </tr>
                                            }
                                        }).collect_view()}
                                    </tbody>
                                </table>
                            </div>
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    }
}

fn value_at(value: &Value, path: &[&str]) -> i64 {
    let mut current = value;
    for key in path {
        current = match current.get(*key) {
            Some(next) => next,
            None => return 0,
        };
    }
    current.as_i64().or_else(|| current.as_str().and_then(|value| value.parse().ok())).unwrap_or(0)
}

/// Data (senza ora) da un timestamp ISO/SQL, per le tabelle.
fn short_date(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "—".into();
    }
    trimmed
        .split(['T', ' '])
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

/// Data e ora leggibili da un timestamp ISO/SQL, nel fuso orario del browser.
fn short_datetime(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "non avviato".into();
    }
    let timestamp = js_sys::Date::parse(trimmed);
    if timestamp.is_nan() {
        return trimmed.to_string();
    }
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(timestamp));
    format!(
        "{:02}/{:02}/{:04} {:02}:{:02}",
        date.get_date(),
        date.get_month() + 1,
        date.get_full_year(),
        date.get_hours(),
        date.get_minutes(),
    )
}

/// Mostra le lingue da `language_requirements` (JSON o lista separata da virgola).
/// Interpreta `language_requirements` (JSON legacy `[{language,required}]` o CSV)
/// in coppie `(lingua, obbligatoria)`.
fn parse_language_entries(raw: &str) -> Vec<(String, bool)> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        if let Ok(items) = serde_json::from_str::<Vec<Value>>(trimmed) {
            return items
                .iter()
                .map(|item| {
                    (
                        item.get("language")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        item.get("required").and_then(Value::as_bool).unwrap_or(true),
                    )
                })
                .filter(|(language, _)| !language.is_empty())
                .collect();
        }
    }
    trimmed
        .split([',', '+'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| (value.to_ascii_lowercase(), true))
        .collect()
}

/// Renders stored requirements for display: a JSON array (`[]` or
/// `[{"language":"ita","required":true}]`) becomes a readable comma list.
fn display_requirements(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return String::new();
    }
    if trimmed.starts_with('[') {
        if let Ok(items) = serde_json::from_str::<Vec<Value>>(trimmed) {
            let languages: Vec<String> = items
                .iter()
                .filter_map(|item| item.get("language").and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != "-")
                .map(str::to_string)
                .collect();
            return languages.join(", ");
        }
        return String::new();
    }
    trimmed.to_string()
}

/// Serializza le richieste lingua nel formato JSON usato dal backend.
fn serialize_language_entries(entries: &[(String, bool)]) -> String {
    let items: Vec<Value> = entries
        .iter()
        .filter(|(language, _)| !language.trim().is_empty())
        .map(|(language, required)| {
            serde_json::json!({"language": language.trim().to_ascii_lowercase(), "required": *required})
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
}

/// Una riga dell'editor lingue: select + flag "obbligatoria".
fn language_row(lang: RwSignal<String>, req: RwSignal<bool>) -> impl IntoView {
    view! {
        <div class="lang-row">
            <select prop:value=lang on:change=move |event| lang.set(event_target_value(&event))>
                <option value="">{ctx_tr("—")}</option>
                {LANGUAGE_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}
            </select>
            <label class="check" title=ctx_tr("La release deve contenere questa lingua")><input type="checkbox" disabled=move || lang.get().is_empty() prop:checked=move || !lang.get().is_empty() && req.get() on:change=move |event| req.set(event_target_checked(&event)) /> <span>{ctx_tr("obbligatoria")}</span></label>
        </div>
    }
}

fn size_str(bytes: f64) -> String {
    let mut amount = bytes;
    let units = ["B", "KiB", "MiB", "GiB"];
    let mut index = 0;
    while amount >= 1024.0 && index < units.len() - 1 {
        amount /= 1024.0;
        index += 1;
    }
    format!("{amount:.1} {}", units[index])
}

fn setting_tooltip(key: &str) -> &'static str {
    match key {
        "libtorrent_enabled" => "Attiva o disattiva del tutto il client libtorrent integrato.",
        "libtorrent_dynamic_queue" => "Regola automaticamente quanti torrent sono attivi in base al carico.",
        "libtorrent_dynamic_queue_min" => "Numero minimo di download dinamici. La coda cambia al massimo di uno per volta.",
        "libtorrent_dynamic_queue_max" => "Numero massimo di download dinamici. Servono campioni consecutivi coerenti prima di aumentare la coda.",
        "libtorrent_dont_count_slow_torrents" => "I torrent che non trasferiscono dati non consumano uno slot attivo: gli swarm senza peer non bloccano i download sani.",
        "libtorrent_stall_after_min" => "Dopo questi minuti senza aumento dei byte completati il torrent viene considerato stalled. La presenza di peer non basta a resettare il timer.",
        "libtorrent_stall_retry_min" => "Intervallo tra i tentativi di reannounce dei torrent stalled, mantenuti fuori dalla coda attiva.",
        "libtorrent_stall_giveup_min" => "Dopo questo periodo senza progresso il torrent viene rimosso; 0 disabilita la rimozione automatica.",
        "libtorrent_auto_optimize" => "Applica periodicamente l'ottimizzazione di cache, buffer e coda in base alle risorse, senza dover premere Ottimizza.",
        "libtorrent_sequential" => "Scarica i file in ordine sequenziale invece che a pezzi sparsi.",
        "libtorrent_extra_settings" => "Impostazioni libtorrent avanzate, una per riga nel formato chiave=valore (es. max_peerlist_size=4000). Solo le chiavi riconosciute vengono applicate.",
        "libtorrent_active_downloads" => "Valore base dei download attivi. Con la coda dinamica Rextto lo adatta a runtime tra il minimo e il massimo configurati.",
        "libtorrent_active_seeds" => "Valore base dei seed attivi. Con la coda dinamica scende a 1 quando ci sono download in coda.",
        "libtorrent_active_limit" => "Valore base del limite di torrent attivi. Con la coda dinamica diventa max(base, download + seed + 2).",
        "libtorrent_seed_ratio" => "Rapporto upload/download dopo cui fermare il seeding (0 = infinito).",
        "libtorrent_seed_time" => "Limite di seeding in minuti, usato solo se Seed massimo (giorni) è 0; utile per limiti inferiori a 24 ore.",
        "libtorrent_seed_time_days" => "Limite principale di seeding in giorni; se maggiore di 0 prevale sul limite in minuti.",
        "auto_remove_completed" => "Elimina dalla sessione i torrent completati appena raggiungono i limiti di seed (ratio/tempo), senza dover premere Pulisci completati. I file si cancellano solo se la copia è già archiviata sul NAS.",
        "libtorrent_connections_limit" => "Numero massimo di connessioni peer simultanee a livello di sessione.",
        "libtorrent_upload_slots_limit" => "Numero di peer non bloccati in upload (-1 = automatico).",
        "libtorrent_half_open_limit" => "Numero massimo di connessioni in fase di apertura (-1 = automatico).",
        "libtorrent_max_connections_per_torrent" => "Limite di connessioni per singolo torrent (-1 = illimitato).",
        "libtorrent_max_uploads_per_torrent" => "Limite di upload per singolo torrent (-1 = illimitato).",
        "libtorrent_aio_threads" => "Thread dedicati alle operazioni su disco (-1 = automatico).",
        "libtorrent_cache_size" => "Dimensione della cache disco in blocchi (-1 = automatico).",
        "libtorrent_cache_expiry" => "Secondi di inattività dopo cui un blocco esce dalla cache.",
        "libtorrent_alert_queue_size" => "Dimensione della coda degli alert di libtorrent.",
        "libtorrent_dht" => "Abilita la rete DHT per trovare peer senza tracker.",
        "libtorrent_pex" => "Peer Exchange: scambio peer con altri client.",
        "libtorrent_lsd" => "Local Service Discovery: trova peer nella rete locale.",
        "libtorrent_upnp" => "Apre le porte del router automaticamente con UPnP.",
        "libtorrent_natpmp" => "Apre le porte del router automaticamente con NAT-PMP.",
        "libtorrent_utp" => "Abilita il protocollo uTP (UDP) oltre a TCP.",
        "libtorrent_prefer_rc4" => "Preferisce la cifratura RC4 sulle connessioni.",
        "libtorrent_announce_to_all_trackers" => "Annuncia a tutti i tracker, non solo al primo di ogni tier.",
        "libtorrent_announce_to_all_tiers" => "Annuncia a tutti i tier, non solo al primo.",
        "libtorrent_allow_multiple_connections_per_ip" => "Permette più connessioni dallo stesso indirizzo IP.",
        "libtorrent_announce_interval" => "Intervallo minimo (secondi) tra due announce allo stesso tracker.",
        "libtorrent_torrent_connect_boost" => "Numero di tentativi di connessione extra all'avvio del torrent.",
        "libtorrent_dht_bootstrap_nodes" => "Nodi DHT iniziali (host:porta separati da virgola).",
        "libtorrent_encryption" => "Politica di cifratura: 0 disabilitata, 1 abilitata, 2 forzata.",
        "libtorrent_apply_ip_filter" => "Applica il filtro IP anche ai tracker.",
        "libtorrent_ipfilter_url" => "File locale o URL della lista IP da bloccare.",
        "libtorrent_proxy_host" => "Host del proxy per il traffico torrent.",
        "libtorrent_proxy_port" => "Porta del proxy (0 = nessun proxy).",
        "libtorrent_listen_interfaces" => "Interfacce e porte di ascolto (es. 0.0.0.0:6881-6891).",
        "libtorrent_outgoing_interface" => "Killswitch VPN: interfaccia usata per tutto il traffico BitTorrent in uscita (es. tun0, wg0).",
        "libtorrent_dl_limit" => "Limite globale di download in KiB/s (0 = illimitato).",
        "libtorrent_ul_limit" => "Limite globale di upload in KiB/s (0 = illimitato).",
        "libtorrent_sched_enabled" => "Attiva la fascia oraria con limiti di velocità diversi.",
        "libtorrent_sched_start" => "Ora di inizio della programmazione (HH:MM).",
        "libtorrent_sched_end" => "Ora di fine della programmazione (HH:MM).",
        "libtorrent_sched_days" => "Giorni attivi: 0=Lun … 6=Dom (es. 0,1,2,3,4).",
        "libtorrent_sched_dl_limit" => "Limite di download in KiB/s durante la programmazione.",
        "libtorrent_sched_ul_limit" => "Limite di upload in KiB/s durante la programmazione.",
        "flaresolverr_url" => "URL del servizio FlareSolverr per aggirare Cloudflare.",
        "refresh_interval" => "Intervallo tra le ricerche automatiche di serie e film, in secondi (21600 = 6 ore). I fumetti seguono il proprio intervallo settimanale.",
        "max_release_age_days" => "Ignora le release più vecchie di N giorni (0 = nessun limite).",
        "gap_filling" => "Attiva il riempimento dei buchi (episodi mancanti) dalle release disponibili.",
        "gap_fill_max_per_series" => "Numero massimo di gap da cercare per serie in un ciclo (0 = illimitato).",
        "gap_deep_interval_hours" => "Ogni quante ore fare una ricerca live mirata sugli indexer per i gap.",
        "gap_deep_max_per_cycle" => "Numero massimo di ricerche live (deep) per ciclo.",
        "delay_torrent_minutes" => "Ritarda l'avvio dei download delle serie per questo numero di minuti dopo che una release è stata trovata. 0 avvia senza attesa; serve a lasciare arrivare versioni migliori.",
        "delay_movies_minutes" => "Ritarda l'avvio dei download dei film per questo numero di minuti dopo che una release è stata trovata. 0 avvia senza attesa.",
        "delay_bypass_score" => "Se una release raggiunge almeno questo punteggio, ignora il delay configurato. 0 disattiva il bypass e lascia attivo sempre il delay.",
        "housekeeping_enabled" => "Attiva la pulizia periodica dei dati tecnici e dello storico. Non cancella i file della libreria né le release dell'Archivio.",
        "housekeeping_interval_hours" => "Intervallo tra due housekeeping automatici, in ore. Esempio: 24 esegue la pulizia una volta al giorno; il comando manuale resta sempre disponibile.",
        "housekeeping_retain_cycles" => "Numero di statistiche dei cicli di ricerca da conservare. Ogni statistica conta release analizzate, candidate, download avviati, gap riempiti ed errori. Esempio: 200 conserva gli ultimi 200 cicli e, quando arriva il 201°, elimina solo i relativi contatori: non elimina release, torrent o file.",
        "housekeeping_seen_days" => "Elimina solo le righe storiche delle release già viste nei feed più vecchie di questo numero di giorni. Esempio: 30 pulisce i record dei feed oltre 30 giorni; 0 non applica questa pulizia.",
        "housekeeping_history_days" => "Elimina dallo Storico download della sezione Scarico le righe dei torrent già rimossi più vecchie di questo numero di giorni, senza cancellare i file della libreria. Esempio: 90 rimuove solo le righe di download già rimossi oltre 90 giorni; 0 conserva lo storico.",
        "media_info_backfill_enabled" => "Analizza periodicamente con ffprobe i file già presenti che non hanno ancora MediaInfo. Non modifica né sposta i file.",
        "media_info_backfill_interval_minutes" => "Minuti tra due passaggi del backfill MediaInfo. Intervalli più bassi completano prima l'analisi ma usano più disco/CPU.",
        "media_info_backfill_batch" => "Numero massimo di file analizzati in ogni passaggio MediaInfo. Valori più alti accelerano il recupero ma aumentano il carico temporaneo.",
        "notify_telegram" => "Invia le notifiche su Telegram.",
        "telegram_bot_token" => "Token del bot Telegram (da @BotFather).",
        "telegram_chat_id" => "ID della chat/canale dove inviare le notifiche.",
        "notify_email" => "Invia le notifiche via email.",
        "email_smtp" => "Server SMTP nel formato host:porta (es. smtp.gmail.com:587).",
        "email_from" => "Indirizzo mittente delle email di notifica.",
        "email_to" => "Destinatari delle email (separati da virgola).",
        "email_password" => "Password/app-password SMTP (non visualizzata).",
        "notify_webhook_url" => "URL del webhook a cui inviare gli eventi.",
        "notify_webhook_secret" => "Segreto HMAC per firmare le richieste al webhook.",
        "rename_episodes" => "Rinomina i file scaricati usando i metadati TMDB.",
        "cleanup_upgrades" => "Sostituisce versioni inferiori già archiviate con upgrade migliori.",
        "cleanup_action" => "Cosa fare con i file sostituiti: sposta nel trash o elimina.",
        "api_token" => "Token per proteggere le API e la UI (vuoto = nessuna autenticazione).",
        "tmdb_language" => "Lingua usata per i metadati TMDB (codice BCP-47, es. it-IT, en-US). Influisce su titoli episodi e descrizioni.",
        "default_language" => "Lingua preferita di default per serie e film (es. ita, eng).",
        "blacklist" => "Parole vietate separate da virgola: le release che le contengono vengono scartate (es. cam, ts, screener).",
        "archive_root" => "Cartella di archivio predefinita per i contenuti senza percorso dedicato.",
        "trash_path" => "Cartella dove vengono spostati i file sostituiti/duplicati.",
        "libtorrent_dir" => "Cartella di download predefinita di libtorrent.",
        "libtorrent_temp_dir" => "Cartella temporanea per i download in corso.",
        "libtorrent_ramdisk_dir" => "RAM disk da usare per i download in corso, se disponibile.",
        "libtorrent_ramdisk_enabled" => "Scarica in RAM i torrent che rientrano nella soglia; i file più grandi o lo spazio insufficiente vengono spostati su disco appena arrivano i metadati.",
        "libtorrent_ramdisk_threshold_gb" => "Dimensione massima di un singolo torrent ammesso sul RAM disk (in GB). Oltre questo valore il download va direttamente su disco.",
        "libtorrent_ramdisk_margin_gb" => "Spazio libero da lasciare sul RAM disk una volta completato il download (in GB).",
        "libtorrent_ramdisk_min_free_bytes" => "Spazio minimo libero in byte richiesto per usare il RAM disk. 0 = usa il margine configurato sopra.",
        "libtorrent_port_min" => "Porta minima della sessione libtorrent (richiede il riavvio del servizio).",
        "libtorrent_port_max" => "Porta massima della sessione libtorrent (richiede il riavvio del servizio).",
        "min_free_space_gb" => "Spazio libero minimo (GB) sulla cartella download: sotto questa soglia il ciclo non avvia nuovi download.",
        "trash_retention_days" => "La pulizia del trash elimina solo i file più vecchi di N giorni (0 = elimina tutto).",
        "archive_retention_days" => "Elimina dall'archivio le release più vecchie di N giorni (0 = conserva sempre).",
        "archive_cleanup_enabled" => "Abilita la pulizia automatica dell'archivio secondo età massima e numero minimo da conservare.",
        "archive_max_age_days" => "Età massima delle release in archivio, in giorni (0 = nessun limite).",
        "archive_keep_min" => "Numero minimo di release recenti da conservare sempre in archivio, anche se più vecchie dell'età massima.",
        "cleanup_min_score_diff" => "Differenza minima di punteggio per sostituire un file esistente con uno migliore (cleanup).",
        "upgrade_min_score_diff" => "Differenza minima di punteggio per sostituire un file con un upgrade migliore.",
        "tvdb_api_key" => "Chiave API v4 di TheTVDB per ricerca serie e metadati.",
        "tvdb_language" => "Lingua preferita per i metadati TVDB (es. ita, eng).",
        "stop_on_old_page_threshold" => "Quante pagine di elenco leggere per ogni feed (3 è un buon compromesso).",
        "rename_verify_interval" => "Ogni quante ore verificare che i file archiviati/rinominati siano ancora presenti.",
        "move_episodes" => "Sposta gli episodi e i season pack nella cartella archivio invece di copiarli; i file spuri del pack vanno nel trash.",
        "debug_enabled" => "Attiva log dettagliati e diagnostiche periodiche (RAM, torrent, loop) per il debug.",
        _ => "",
    }
}

/// Tooltip descrittivo per ogni peso di valutazione (Configurazione → Punteggi).
fn score_tooltip(key: &str) -> &'static str {
    match key {
        "score_res_2160p" => "Punteggio assegnato alle release in 4K/UHD (2160p).",
        "score_res_1080p" => "Punteggio assegnato alle release in Full HD (1080p).",
        "score_res_720p" => "Punteggio assegnato alle release in HD (720p).",
        "score_res_576p" => "Punteggio assegnato alle release in SD (576p e inferiori).",
        "score_source_bluray" => "Punteggio per la sorgente BluRay (massima qualità video).",
        "score_source_remux" => "Punteggio per la sorgente Remux (video non ricompresso).",
        "score_source_webdl" => "Punteggio per la sorgente WEB-DL (download dai servizi streaming).",
        "score_source_webrip" => "Punteggio per la sorgente WEBRip (cattura da streaming, qualità inferiore).",
        "score_source_hdtv" => "Punteggio per la sorgente HDTV (registrazione televisiva).",
        "score_codec_h265" => "Punteggio per il codec video H.265/x265 (HEVC).",
        "score_codec_h264" => "Punteggio per il codec video H.264/x264 (AVC).",
        "score_audio_truehd" => "Punteggio per la traccia audio Dolby TrueHD (senza perdita).",
        "score_audio_dts-hd" => "Punteggio per la traccia audio DTS-HD.",
        "score_audio_dts" => "Punteggio per la traccia audio DTS.",
        "score_audio_ddp" => "Punteggio per la traccia audio Dolby Digital Plus (EAC3/DDP).",
        "score_audio_ac3" => "Punteggio per la traccia audio AC3 (Dolby Digital 5.1).",
        "score_audio_aac" => "Punteggio per la traccia audio AAC.",
        "score_audio_mp3" => "Punteggio per la traccia audio MP3.",
        "score_bonus_dv" => "Bonus per le release con Dolby Vision.",
        "score_bonus_hdr" => "Bonus per le release con HDR (HDR10/HDR10+).",
        "score_bonus_proper" => "Bonus per le release PROPER (correzione di una release precedente).",
        "score_bonus_repack" => "Bonus per le release REPACK (ri-pacchettizzate dallo stesso gruppo).",
        "score_bonus_real" => "Bonus per le release REAL (versione autentica, non fake).",
        _ => "Peso usato per scegliere la release migliore: più alto = più preferito.",
    }
}

fn save_message(value: &Value) -> String {    if value
        .get("restart_required")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "Salvato — richiede riavvio".into()
    } else {
        "Salvato".into()
    }
}

/// Rapporto upload/download totale di un torrent. For an already-present
/// completed torrent, libtorrent can report zero downloaded bytes; use its
/// payload size as the denominator in that case.
fn ratio_label(item: &Value) -> String {
    let up = item.get("all_time_upload").and_then(Value::as_i64).unwrap_or(0) as f64;
    let reported_down = item.get("all_time_download").and_then(Value::as_i64).unwrap_or(0) as f64;
    let down = if reported_down > 0.0 {
        reported_down
    } else {
        item.get("total_size").and_then(Value::as_i64).unwrap_or(0) as f64
    };
    if down <= 0.0 {
        "—".into()
    } else {
        format!("{:.2}", up / down)
    }
}

/// Nome file (ultimo segmento) di un percorso, per anteprime compatte.
fn file_name(path: &str) -> String {
    path.rsplit(|character| character == '/' || character == '\\')
        .next()
        .unwrap_or(path)
        .to_string()
}

/// Chiede conferma all'utente prima di un'azione distruttiva.
fn confirm_dialog(message: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.confirm_with_message(message).ok())
        .unwrap_or(false)
}

/// Copia un testo negli appunti del browser (Clipboard API asincrona).
fn copy_to_clipboard(text: &str) {
    let Some(window) = web_sys::window() else { return };
    let text = text.to_string();
    spawn_local(async move {
        let _ =
            wasm_bindgen_futures::JsFuture::from(window.navigator().clipboard().write_text(&text))
                .await;
    });
}

/// Tag assegnato a un torrent (dalla tabella torrent_meta).
fn torrent_tag_of(tags: &[Value], hash: &str) -> String {
    tags.iter()
        .find(|item| text(item, "hash", "").eq_ignore_ascii_case(hash))
        .map(|item| text(item, "tag", ""))
        .unwrap_or_default()
}

/// Divide il valore tag in chip. Il caso normale è un tag singolo; virgole e
/// punti e virgola sono accettati così un valore composto si legge come elenco.
fn tag_chips(tag: &str) -> Vec<String> {
    tag.split([',', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Applica il filtro tag della barra strumenti a torrent e download HTTP.
/// Il valore salvato può contenere più etichette separate da virgola/punto e
/// virgola: il filtro trova l'elemento se contiene l'etichetta cercata.
fn tag_matches(term: &str, tag: &str) -> bool {
    match term {
        "" => true,
        "__none__" => tag_chips(tag).is_empty(),
        _ => tag_chips(tag)
            .iter()
            .any(|chip| chip.eq_ignore_ascii_case(term)),
    }
}

/// Aggiunge un'etichetta alla lista separata da virgole, senza duplicati e
/// ignorando il case; anche il valore aggiunto viene spezzato nei suoi chip.
fn merge_tag(current: &str, tag: &str) -> String {
    let mut chips = tag_chips(current);
    for candidate in tag_chips(tag) {
        if !chips
            .iter()
            .any(|chip| chip.eq_ignore_ascii_case(&candidate))
        {
            chips.push(candidate);
        }
    }
    chips.join(", ")
}

/// Rimuove le etichette indicate dalla lista; con `tag` vuoto la svuota.
fn remove_tag(current: &str, tag: &str) -> String {
    let removals = tag_chips(tag);
    let chips: Vec<String> = if removals.is_empty() {
        Vec::new()
    } else {
        tag_chips(current)
            .into_iter()
            .filter(|chip| {
                !removals
                    .iter()
                    .any(|remove| remove.eq_ignore_ascii_case(chip))
            })
            .collect()
    };
    chips.join(", ")
}

/// Adatta un download HTTP (fumetti) alla forma usata dai torrent così la
/// tabella unificata può filtrarlo, ordinarlo e disegnarlo con le stesse
/// colonne. `__kind`/`__id` distinguono la riga e la identificano stabilmente.
fn normalized_http_item(item: &Value) -> Value {
    let status = text(item, "status", "");
    let total = item.get("total_bytes").and_then(Value::as_u64).unwrap_or(0);
    let done = item.get("downloaded_bytes").and_then(Value::as_u64).unwrap_or(0);
    json!({
        "__kind": "http",
        "__id": text(item, "id", ""),
        "hash": "",
        "name": text(item, "title", ""),
        "title": text(item, "title", ""),
        "method": text(item, "method", ""),
        "status": status,
        "state": status,
        "progress": item.get("progress").and_then(Value::as_f64).unwrap_or(-1.0),
        "download_rate": value_f64(item, "speed_bytes"),
        "upload_rate": 0.0,
        "num_peers": 0,
        "num_seeds": 0,
        "all_time_upload": 0.0,
        "all_time_download": done,
        "total_size": total,
        "total_done": done,
        "has_metadata": true,
        "tag": text(item, "tag", ""),
        "error": text(item, "error", ""),
        "updated_at": text(item, "updated_at", ""),
    })
}

fn value_f64(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn torrent_ratio_value(value: &Value) -> f64 {
    let up = value_f64(value, "all_time_upload");
    let reported_down = value_f64(value, "all_time_download");
    let down = if reported_down > 0.0 {
        reported_down
    } else {
        value_f64(value, "total_size")
    };
    if down <= 0.0 { 0.0 } else { up / down }
}

fn torrent_order(a: &Value, b: &Value, key: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match key {
        "name" => text(a, "name", "").to_lowercase().cmp(&text(b, "name", "").to_lowercase()),
        "progress" => value_f64(a, "progress").partial_cmp(&value_f64(b, "progress")).unwrap_or(Ordering::Equal),
        "dl" => value_f64(a, "download_rate").partial_cmp(&value_f64(b, "download_rate")).unwrap_or(Ordering::Equal),
        "ul" => value_f64(a, "upload_rate").partial_cmp(&value_f64(b, "upload_rate")).unwrap_or(Ordering::Equal),
        "peers" => value_f64(a, "num_peers").partial_cmp(&value_f64(b, "num_peers")).unwrap_or(Ordering::Equal),
        "ratio" => torrent_ratio_value(a).partial_cmp(&torrent_ratio_value(b)).unwrap_or(Ordering::Equal),
        "eta" => torrent_eta_seconds(a).partial_cmp(&torrent_eta_seconds(b)).unwrap_or(Ordering::Equal),
        _ => Ordering::Equal,
    }
}

/// Secondi stimati al completamento del torrent (`f64::INFINITY` se non calcolabili).
fn torrent_eta_seconds(item: &Value) -> f64 {
    if !item.get("has_metadata").and_then(Value::as_bool).unwrap_or(false) {
        return f64::INFINITY;
    }
    let total = item.get("total_size").and_then(Value::as_i64).unwrap_or(0) as f64;
    let done = item.get("total_done").and_then(Value::as_i64).unwrap_or(0) as f64;
    let rate = item.get("download_rate").and_then(Value::as_f64).unwrap_or(0.0);
    let remaining = total - done;
    if total <= 0.0 || remaining <= 0.0 {
        return if total > 0.0 { 0.0 } else { f64::INFINITY };
    }
    if rate <= 0.0 {
        return f64::INFINITY;
    }
    remaining / rate
}

/// Tempo stimato al completamento del torrent, in forma compatta.
fn eta_label(data: RwSignal<Data>, item: &Value) -> String {
    let total = item.get("total_size").and_then(Value::as_i64).unwrap_or(0);
    let done = item.get("total_done").and_then(Value::as_i64).unwrap_or(0);
    let seconds = torrent_eta_seconds(item);
    if seconds == 0.0 && total > 0 && done >= total {
        return tr(data, "completo");
    }
    if !seconds.is_finite() {
        return "—".into();
    }
    format_eta(data, seconds as u64)
}

fn format_eta(data: RwSignal<Data>, seconds: u64) -> String {
    let (days, rem) = (seconds / 86_400, seconds % 86_400);
    let (hours, rem) = (rem / 3_600, rem % 3_600);
    let (minutes, secs) = (rem / 60, rem % 60);
    if days > 0 {
        tr_format(
            data,
            "{days}g {hours}h",
            &[("{days}", days.to_string()), ("{hours}", hours.to_string())],
        )
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {secs:02}s")
    } else {
        format!("{secs}s")
    }
}

/// Punti per una sparkline SVG normalizzata su un massimo condiviso (così due
/// serie, es. ↑ e ↓, restano confrontabili sullo stesso grafico).
fn sparkline_points_scaled(values: &[f64], max: f64, width: f64, height: f64) -> String {
    if values.len() < 2 {
        return String::new();
    }
    let max = max.max(1.0);
    let step = width / (values.len() - 1) as f64;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = index as f64 * step;
            let y = height - (value / max * (height - 2.0)) - 1.0;
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Punti per una sparkline normalizzata sull'intervallo della singola serie.
fn sparkline_points(values: &[f64], width: f64, height: f64) -> String {
    let max = values.iter().cloned().fold(0.0_f64, f64::max);
    sparkline_points_scaled(values, max, width, height)
}

/// Converte il payload di Trakt/Simkl in righe (titolo, dettaglio, data/stato)
/// per mostrarle come tabella invece del JSON grezzo.
fn provider_rows(value: &Value) -> Vec<(String, String, String)> {
    fn title_of(item: &Value) -> String {
        for key in ["show", "movie", "episode", "series"] {
            if let Some(inner) = item.get(key) {
                if let Some(title) = inner.get("title").and_then(Value::as_str) {
                    return title.to_string();
                }
            }
        }
        item.get("title")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .unwrap_or("—")
            .to_string()
    }
    fn detail_of(item: &Value) -> String {
        if let Some(episode) = item.get("episode") {
            let season = episode.get("season").and_then(Value::as_i64).unwrap_or(0);
            let number = episode.get("number").and_then(Value::as_i64).unwrap_or(0);
            let mut detail = format!("S{season:02}E{number:02}");
            if let Some(title) = episode.get("title").and_then(Value::as_str) {
                if !title.is_empty() {
                    detail.push_str(&format!(" · {title}"));
                }
            }
            return detail;
        }
        if let Some(year) = item.get("year").and_then(Value::as_i64) {
            return year.to_string();
        }
        String::new()
    }
    fn date_of(item: &Value) -> String {
        for key in ["first_aired", "listed_at", "date", "released"] {
            if let Some(value) = item.get(key).and_then(Value::as_str) {
                return value.chars().take(10).collect();
            }
        }
        item.get("status").and_then(Value::as_str).unwrap_or("").to_string()
    }
    fn collect(value: &Value, out: &mut Vec<(String, String, String)>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    collect(item, out);
                }
            }
            Value::Object(map) => {
                let is_item = map.contains_key("show")
                    || map.contains_key("movie")
                    || map.contains_key("episode")
                    || map.contains_key("title");
                if is_item {
                    out.push((title_of(value), detail_of(value), date_of(value)));
                } else {
                    for (key, nested) in map {
                        if let Value::Array(items) = nested {
                            for item in items {
                                let mut row = (title_of(item), detail_of(item), date_of(item));
                                if row.1.is_empty() {
                                    row.1 = key.clone();
                                }
                                out.push(row);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut rows = Vec::new();
    if !value.is_null() && value.get("error").is_none() {
        collect(value, &mut rows);
    }
    rows
}

fn torrent_state_label(state: &str) -> (&'static str, &'static str) {
    match state {
        "checking_files" => ("Verifica file", ""),
        "downloading_metadata" => ("Metadata", "warn"),
        "downloading" => ("In scarico", "ok"),
        "finished" => ("Completato", "ok"),
        "seeding" => ("In seed", "ok"),
        "checking_resume_data" => ("Ripristino", ""),
        "queued" => ("In coda", "warn"),
        _ if state.to_lowercase().contains("paus") => ("In pausa", "warn"),
        _ if state.to_lowercase().contains("coda") => ("In coda", "warn"),
        _ => ("Sconosciuto", ""),
    }
}

/// Etichetta leggibile del motivo per cui una release è stata messa in download.
fn torrent_reason_label(data: RwSignal<Data>, reason: &str) -> String {
    match reason {
        "approved" => tr(data, "nuova release"),
        "upgrade" => tr(data, "sostituisce una qualità inferiore"),
        "gap_fill" | "gap_filled" => tr(data, "puntata mancante"),
        "manual" => tr(data, "aggiunta manuale"),
        "restored" => tr(data, "ripristino"),
        _ => reason.replace('_', " "),
    }
}

fn torrent_version_label(data: RwSignal<Data>, version: &str) -> String {
    match version {
        "v1" => tr(data, "BitTorrent v1"),
        "v2" => tr(data, "BitTorrent v2"),
        "hybrid" => tr(data, "Hybrid (v1 + v2)"),
        _ => tr(data, "non disponibile"),
    }
}

fn friendly_slug(url: &str) -> String {    let trimmed = url.trim().trim_end_matches('/');
    let without_query = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let slug = without_query.rsplit('/').next().unwrap_or(without_query);
    let words = slug
        .split(['-', '_'])
        .filter(|part| !part.is_empty() && !part.chars().all(|c| c.is_ascii_digit()))
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>();
    if words.is_empty() {
        slug.to_string()
    } else {
        words.join(" ")
    }
}

#[component]
fn StatLine(label: &'static str, value: Signal<String>) -> impl IntoView {
    let label = ctx_tr(label);
    view! { <div class="row"><span class="muted">{label}</span><strong>{value}</strong></div> }
}

#[component]
fn LanguagePresetField(value: RwSignal<String>) -> impl IntoView {
    view! {
        <div class="language-select-row">
            <select prop:value=value on:change=move |event| value.set(event_target_value(&event))>
                {SERIES_LANGUAGE_OPTIONS.iter().map(|(option, label)| view! { <option value=*option>{ctx_tr(label)}</option> }).collect_view()}
                {move || {
                    let current = value.get();
                    let known = SERIES_LANGUAGE_OPTIONS.iter().any(|(option, _)| *option == current.as_str());
                    if !current.trim().is_empty() && !known {
                        let custom_value = current.clone();
                        view! { <option value=custom_value>{format!("Custom: {current}")}</option> }.into_any()
                    } else {
                        view! {}.into_any()
                    }
                }}
            </select>
            <input prop:value=value on:input=move |event| value.set(event_target_value(&event)) placeholder=ctx_tr("custom: ita,eng") title=ctx_tr("Codici lingua custom separati da virgole") />
        </div>
    }
}

/* ------------------------------------------------------------------ */
/* Downloads / torrents                                                */
/* ------------------------------------------------------------------ */

#[component]
fn ToastHost(data: RwSignal<Data>) -> impl IntoView {
    view! {
        <div class="toast-host">
            {move || data.get().toasts.iter().cloned().map(|item| {
                let id = item.get("id").and_then(Value::as_f64).unwrap_or(0.0);
                let kind = text(&item, "kind", "info");
                let class = format!("toast {kind}");
                view! {
                    <div class=class>
                        <span>{text(&item, "text", "")}</span>
                        <button type="button" class="toast-close" title=ctx_tr("Chiudi") on:click=move |_| data.update(|current| current.toasts.retain(|entry| entry.get("id").and_then(Value::as_f64) != Some(id)))>{ctx_tr("×")}</button>
                    </div>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn Downloads(data: RwSignal<Data>) -> impl IntoView {
    let magnet = RwSignal::new(String::new());
    let upload_message = RwSignal::new(String::new());
    let temp_dl = RwSignal::new(String::new());
    let temp_ul = RwSignal::new(String::new());
    let temp_min = RwSignal::new("60".to_string());
    let temp_message = RwSignal::new(String::new());
    let temp_loaded = RwSignal::new(false);
    Effect::new(move |_| {
        let config = data.get().config;
        if temp_loaded.get_untracked() || config.as_object().is_none_or(|value| value.is_empty()) {
            return;
        }
        temp_dl.set(raw(&config, "libtorrent_temp_dl_limit", ""));
        temp_ul.set(raw(&config, "libtorrent_temp_ul_limit", ""));
        let enabled = matches!(raw(&config, "libtorrent_temp_limit_enabled", "0").as_str(), "1" | "true" | "yes");
        let until = raw(&config, "libtorrent_temp_limit_until", "0").parse::<i64>().unwrap_or(0);
        let minutes = if !enabled || until == 0 {
            0
        } else {
            ((until - (js_sys::Date::now() / 1000.0) as i64).max(0) + 59) / 60
        };
        temp_min.set(minutes.to_string());
        temp_loaded.set(true);
    });
    let tag_filter = RwSignal::new(String::new());
    let history_filter = RwSignal::new(data.get_untracked().history_filter.clone());
    let sort_key = RwSignal::new("name".to_string());
    let sort_asc = RwSignal::new(true);
    let selected_torrents = RwSignal::new(Vec::<String>::new());
    let selected_http = RwSignal::new(Vec::<String>::new());
    let tag_assign = RwSignal::new(String::new());
    let tag_assign_new = RwSignal::new(String::new());
    let add_save_path = RwSignal::new(String::new());
    let add_start = RwSignal::new(true);
    let add_no_rename = RwSignal::new(false);
    let add_sequential = RwSignal::new(false);
    let add_seed_mode = RwSignal::new(false);
    let add_queue_top = RwSignal::new(false);
    let add_first_last = RwSignal::new(false);
    let add_metadata_only = RwSignal::new(false);
    // Configured default download folder, proposed as the placeholder so an
    // empty field keeps the automatic RAM-disk/temp/final tier.
    let default_download_path = Signal::derive(move || {
        data.get()
            .config
            .get("paths")
            .and_then(|paths| paths.get("libtorrent_dir"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    });
    let filtered = Signal::derive(move || {
        let snapshot = data.get();
        let term = tag_filter.get();
        let mut items: Vec<Value> = snapshot
            .torrents
            .iter()
            .filter(|item| tag_matches(&term, &torrent_tag_of(&snapshot.torrent_tags, &text(item, "hash", ""))))
            .cloned()
            .map(|mut item| {
                if let Some(object) = item.as_object_mut() {
                    object.insert("__kind".into(), json!("torrent"));
                }
                item
            })
            .collect();
        items.extend(
            snapshot
                .comic_downloads
                .iter()
                .map(normalized_http_item)
                .filter(|item| tag_matches(&term, &text(item, "tag", ""))),
        );
        let key = sort_key.get();
        let asc = sort_asc.get();
        items.sort_by(|a, b| {
            let order = torrent_order(a, b, &key);
            if asc { order } else { order.reverse() }
        });
        items
    });
    let bulk_post = move |suffix: &'static str, body: Option<Value>| {
        let hashes = selected_torrents.get();
        if hashes.is_empty() {
            return;
        }
        spawn_local(async move {
            for hash in hashes {
                let path = format!("/api/torrents/{hash}/{suffix}");
                let _ = send("POST", &path, body.clone()).await;
            }
            trigger_refresh();
        });
    };
    let selected_count = Signal::derive(move || selected_torrents.get().len() + selected_http.get().len());
    // Unione dei tag esistenti (registro persistente + torrent + download HTTP),
    // condivisa dalla tendina di filtro e da quella di assegnazione.
    let all_tags = Signal::derive(move || {
        let snapshot = data.get();
        let mut tags: Vec<String> = snapshot
            .download_tags
            .iter()
            .flat_map(|tag| tag_chips(tag))
            .collect();
        tags.extend(
            snapshot
                .torrent_tags
                .iter()
                .flat_map(|item| tag_chips(&text(item, "tag", ""))),
        );
        tags.extend(
            snapshot
                .comic_downloads
                .iter()
                .flat_map(|item| tag_chips(&text(item, "tag", ""))),
        );
        tags.sort_by(|left, right| left.to_lowercase().cmp(&right.to_lowercase()));
        tags.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        tags
    });
    // Un tag per i selezionati, torrent e download HTTP insieme: un solo punto
    // di ingresso al posto del pulsante per riga. La voce "__new__" crea un tag
    // che viene registrato e ricompare nelle tendine. `remove` distingue tra
    // aggiunta (unione alle etichette esistenti) e rimozione; con il tag vuoto
    // entrambe azzerano la lista, come prima.
    let update_tags = move |remove: bool| {
        let chosen = tag_assign.get();
        let is_new = chosen == "__new__";
        let tag = if is_new {
            tag_assign_new.get().trim().to_string()
        } else {
            chosen.clone()
        };
        let snapshot = data.get();
        let torrents = selected_torrents.get();
        let http = selected_http.get();
        let count = torrents.len() + http.len();
        if is_new && tag.is_empty() {
            flash_text(data, "err", tr(data, "Scrivi il nome del nuovo tag."));
            return;
        }
        if !is_new && count == 0 {
            flash_text(data, "err", tr(data, "Seleziona almeno un torrent o un download."));
            return;
        }
        let apply = |current: &str| {
            if remove {
                remove_tag(current, &tag)
            } else if tag.is_empty() {
                String::new()
            } else {
                merge_tag(current, &tag)
            }
        };
        let torrent_updates: Vec<(String, String)> = torrents
            .iter()
            .map(|hash| {
                let current = torrent_tag_of(&snapshot.torrent_tags, hash);
                (hash.clone(), apply(&current))
            })
            .collect();
        let http_updates: Vec<(String, String)> = http
            .iter()
            .map(|id| {
                let current = snapshot
                    .comic_downloads
                    .iter()
                    .find(|item| text(item, "id", "") == *id)
                    .map(|item| text(item, "tag", ""))
                    .unwrap_or_default();
                (id.clone(), apply(&current))
            })
            .collect();
        spawn_local(async move {
            if !remove && !tag.is_empty() {
                // Registra ogni etichetta così ricompare nelle tendine anche
                // quando il valore contiene più tag separati da virgola.
                for part in tag_chips(&tag) {
                    let _ = send("POST", "/api/download-tags", Some(json!({"tag": part}))).await;
                }
            }
            for (hash, value) in torrent_updates {
                let _ = send(
                    "POST",
                    "/api/torrent-tags",
                    Some(json!({"hash": hash, "tag": value})),
                )
                .await;
            }
            for (id, value) in http_updates {
                let _ = send(
                    "POST",
                    "/api/comics/downloads/tag",
                    Some(json!({"id": id, "tag": value})),
                )
                .await;
            }
            if count == 0 {
                flash_text(data, "ok", tr(data, "Tag creato"));
            } else {
                flash_text(
                    data,
                    "ok",
                    tr_format(
                        data,
                        if remove {
                            "Tag rimosso da {count} elementi"
                        } else {
                            "Tag assegnato a {count} elementi"
                        },
                        &[("{count}", count.to_string())],
                    ),
                );
            }
            tag_assign.set(String::new());
            tag_assign_new.set(String::new());
            trigger_refresh();
        });
    };
    let assign_tag = move |_| update_tags(false);
    let remove_tags = move |_| update_tags(true);
    let set_sort = move |key: &'static str| {
        if sort_key.get() == key {
            sort_asc.update(|value| *value = !*value);
        } else {
            sort_key.set(key.to_string());
            sort_asc.set(true);
        }
    };
    let all_selected = Signal::derive(move || {
        let items = filtered.get();
        let torrents = items
            .iter()
            .filter(|item| text(item, "__kind", "torrent") != "http")
            .collect::<Vec<_>>();
        let http = items
            .iter()
            .filter(|item| text(item, "__kind", "torrent") == "http")
            .collect::<Vec<_>>();
        let torrents_selected = torrents.iter().all(|item| {
            let hash = text(item, "hash", "");
            hash.is_empty() || selected_torrents.get().contains(&hash)
        });
        let http_selected = http
            .iter()
            .all(|item| selected_http.get().contains(&text(item, "__id", "")));
        (!torrents.is_empty() || !http.is_empty()) && torrents_selected && http_selected
    });
    let on_upload = move |event: leptos::ev::Event| {
        let input = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok());
        let Some(input) = input else { return };
        let Some(files) = input.files() else { return };
        let Some(file) = files.get(0) else { return };
        let message = upload_message;
        spawn_local(async move {
            let buffer = match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                Ok(buffer) => js_sys::Uint8Array::new(&buffer).to_vec(),
                Err(_) => {
                    message.set(tr(data, "Lettura file fallita"));
                    return;
                }
            };
            let mut builder = Request::post("/api/upload-torrent")
                .header("content-type", "application/octet-stream");
            if let Some(value) = token() {
                builder = builder.header("x-rextto-token", &value);
            }
            match builder.body(js_sys::Uint8Array::from(buffer.as_slice())) {
                Ok(request) => match request.send().await {
                    Ok(response) if response.ok() => {
                        message.set(tr(data, "Torrent caricato"));
                        trigger_refresh();
                    }
                    Ok(response) => message.set(tr_format(data, "Errore upload ({status})", &[("{status}", response.status().to_string())])),
                    Err(error) => message.set(error.to_string()),
                },
                Err(error) => message.set(error.to_string()),
            }
        });
    };
    view! {
        <div class="view">
            <Panel title="Aggiungi torrent">
                <form class="form" on:submit=move |event| {
                    event.prevent_default();
                    let value = magnet.get();
                    if !value.trim().is_empty() {
                        run_post(data, "/api/send-magnet", Some(json!({
                            "magnet": value,
                            "save_path": add_save_path.get(),
                            "start_paused": !add_start.get(),
                            "no_rename": add_no_rename.get(),
                            "sequential": add_sequential.get(),
                            "seed_mode": add_seed_mode.get(),
                            "queue_top": add_queue_top.get(),
                            "first_last": add_first_last.get(),
                            "stop_at_metadata": add_metadata_only.get()
                        })), "Torrent accodato");
                        magnet.set(String::new());
                    }
                }>
                    <div class="add-torrent-grid">
                        <label class="field" title=ctx_tr("Incolla un link magnet oppure un indirizzo http(s) che punta a un file .torrent")>
                            <span>{ctx_tr("Opzione 1 — Magnet o URL")}</span>
                            <input prop:value=magnet on:input=move |event| magnet.set(event_target_value(&event)) placeholder=ctx_tr("magnet:?xt=urn:btih:… / urn:btmh:1220… oppure https://…/file.torrent") />
                            <small class="hint">{ctx_tr("Copia/incolla un link: Rextto lo aggiunge e inizia a scaricare.")}</small>
                        </label>
                        <label class="field" title=ctx_tr("Carica un file .torrent salvato sul tuo computer, utile quando non c'è un link magnet")>
                            <span>{ctx_tr("Opzione 2 — File .torrent")}</span>
                            <input type="file" accept=".torrent" on:change=on_upload />
                            <small class="hint">{ctx_tr("Carica un file .torrent locale (in alternativa al link a sinistra).")}</small>
                            <Show when=move || data.get().status.get("dry_run").and_then(Value::as_bool).unwrap_or(false)>
                                <small class="hint">{ctx_tr("Dry-run attivo: nessun download reale parte finché non passi in modalità attiva.")}</small>
                            </Show>
                            <small class="muted">{move || upload_message.get()}</small>
                        </label>
                        <label class="field" title=ctx_tr("Cartella di salvataggio. Lascia vuoto per usare la predefinita: i torrent entro la soglia vanno in RAM disk e poi su disco.")>
                            <span>{ctx_tr("Percorso (opzionale)")}</span>
                            <div class="path-picker">
                                <input prop:value=add_save_path on:input=move |event| add_save_path.set(event_target_value(&event)) placeholder=move || default_download_path.get() />
                                <BrowseButton value=add_save_path />
                            </div>
                            <small class="hint">{move || format!("{} ({})", tr(data, "Vuoto = predefinita"), default_download_path.get())}</small>
                        </label>
                        <div class="add-torrent-action">
                            <button class="btn primary" title=ctx_tr("Aggiungi il magnet o il link .torrent alla sessione")>{ctx_tr("Aggiungi")}</button>
                        </div>
                    </div>
                    <div class="toolbar" style="margin-top:10px;flex-wrap:wrap">
                        <label class="check" title=ctx_tr("Se disattivato, il torrent viene aggiunto in pausa e non parte finché non lo riprendi")><input type="checkbox" prop:checked=add_start on:change=move |event| add_start.set(event_target_checked(&event)) /> <span>{ctx_tr("Scarica subito")}</span></label>
                        <label class="check" title=ctx_tr("Il torrent non verrà rinominato al termine del download")><input type="checkbox" prop:checked=add_no_rename on:change=move |event| add_no_rename.set(event_target_checked(&event)) /> <span>{ctx_tr("Non rinominare")}</span></label>
                        <label class="check" title=ctx_tr("Scarica i file in ordine sequenziale (utile per la visione immediata)")><input type="checkbox" prop:checked=add_sequential on:change=move |event| add_sequential.set(event_target_checked(&event)) /> <span>{ctx_tr("Sequenziale")}</span></label>
                        <label class="check" title=ctx_tr("Salta la verifica dell'hash: da usare solo se i dati sono già completi")><input type="checkbox" prop:checked=add_seed_mode on:change=move |event| add_seed_mode.set(event_target_checked(&event)) /> <span>{ctx_tr("Salta verifica")}</span></label>
                        <label class="check" title=ctx_tr("Metti il torrent in cima alla coda")><input type="checkbox" prop:checked=add_queue_top on:change=move |event| add_queue_top.set(event_target_checked(&event)) /> <span>{ctx_tr("In cima alla coda")}</span></label>
                        <label class="check" title=ctx_tr("Dai priorità al primo e all'ultimo pezzo di ogni file")><input type="checkbox" prop:checked=add_first_last on:change=move |event| add_first_last.set(event_target_checked(&event)) /> <span>{ctx_tr("Primo/ultimo pezzo")}</span></label>
                        <label class="check" title=ctx_tr("Scarica solo i metadati e metti in pausa: utile per valutare la release prima di partire")><input type="checkbox" prop:checked=add_metadata_only on:change=move |event| add_metadata_only.set(event_target_checked(&event)) /> <span>{ctx_tr("Solo metadati")}</span></label>
                    </div>
                </form>
            </Panel>
            <Panel title="Download session">
                <p class="muted">{ctx_tr("Torrent e download HTTP (fumetti) nella stessa lista. Il seed si applica solo ai torrent; i download HTTP restano visibili fino al completamento o all'errore. Il tag li filtra insieme ai torrent.")}</p>
                <div class="toolbar" style="margin-bottom:10px">
                    <button class="btn sm" title=ctx_tr("Toglie dalla coda libtorrent i torrent completati che hanno già raggiunto i limiti di seed (ratio/tempo). Esclude il seed infinito e non cancella l'archivio NAS: i torrent escono dalla Sessione e passano allo Storico download.") on:click=move |_| run_cleanup_completed(data)>{ctx_tr("Pulisci completati")}</button>
                    <label class="check" title=ctx_tr("Elimina dalla sessione i torrent completati appena raggiungono i limiti di seed (ratio/tempo). Non cancella l'archivio NAS: la copia in libreria resta.")>
                        <input type="checkbox" prop:checked=move || flag(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "auto_remove_completed", false) on:change=move |event| {
                            let value = if event_target_checked(&event) { "true" } else { "false" };
                            run_post(data, "/api/config/settings", Some(json!({"key": "auto_remove_completed", "value": value})), "Impostazione salvata");
                        } />
                        <span>{ctx_tr("Elimina i completati dopo il seed")}</span>
                    </label>
                    <select style="width:auto" title=ctx_tr("Filtra torrent e download HTTP per tag") prop:value=tag_filter on:change=move |event| tag_filter.set(event_target_value(&event))>
                        <option value="">{ctx_tr("Tutti i tag")}</option>
                        <option value="__none__">{ctx_tr("Senza tag")}</option>
                        {move || all_tags.get().into_iter().map(|tag| { let value = tag.clone(); view! { <option value=value>{tag}</option> } }).collect_view()}
                    </select>
                    <span class="cycle-label" style="margin-left:12px">{ctx_tr("Limite temporaneo")}</span>
                    <input style="width:90px" prop:value=temp_dl on:input=move |event| temp_dl.set(event_target_value(&event)) placeholder=ctx_tr("DL KiB/s") title=ctx_tr("Download temporaneo in KiB/s (0 = illimitato)") />
                    <input style="width:90px" prop:value=temp_ul on:input=move |event| temp_ul.set(event_target_value(&event)) placeholder=ctx_tr("UL KiB/s") title=ctx_tr("Upload temporaneo in KiB/s (0 = illimitato)") />
                     <input style="width:80px" prop:value=temp_min on:input=move |event| temp_min.set(event_target_value(&event)) placeholder=ctx_tr("minuti") title=ctx_tr("Durata in minuti (0 = permanente; la programmazione ha la precedenza)") />
                     <button class="btn sm primary" title=ctx_tr("Applica i limiti temporanei; con durata 0 restano attivi finché non li rimuovi o interviene la programmazione") on:click=move |_| {
                        let body = json!({
                            "download_kib": temp_dl.get().trim().parse::<i64>().unwrap_or(0),
                            "upload_kib": temp_ul.get().trim().parse::<i64>().unwrap_or(0),
                            "minutes": temp_min.get().trim().parse::<i64>().unwrap_or(60),
                        });
                        let message = temp_message;
                        spawn_local(async move {
                            match send("POST", "/api/torrents/temp-limits", Some(body)).await {
                                Ok(_) => message.set(tr(data, "Limite temporaneo applicato")),
                                Err(error) => message.set(error),
                            }
                        });
                    }>{ctx_tr("Applica")}</button>
                    <button class="btn sm" title=ctx_tr("Rimuove subito il limite temporaneo") on:click=move |_| {
                        let message = temp_message;
                        spawn_local(async move {
                         match send("POST", "/api/torrents/temp-limits", Some(json!({"clear": true}))).await {
                                Ok(_) => message.set(tr(data, "Limite temporaneo rimosso")),
                                Err(error) => message.set(error),
                            }
                        });
                    }>{ctx_tr("Rimuovi")}</button>
                    <small class="muted">{temp_message}</small>
                </div>
                <div class="toolbar" style="margin-bottom:10px">
                    <span class="muted">{move || tr_format(data, "{count} selezionati", &[("{count}", selected_count.get().to_string())])}</span>
                    <select style="width:auto" title=ctx_tr("Tag da assegnare ai torrent e ai download HTTP selezionati") prop:value=tag_assign on:change=move |event| tag_assign.set(event_target_value(&event))>
                        <option value="">{ctx_tr("— rimuovi il tag —")}</option>
                        {move || all_tags.get().into_iter().map(|tag| { let value = tag.clone(); view! { <option value=value>{tag}</option> } }).collect_view()}
                        <option value="__new__">{ctx_tr("➕ Nuovo tag…")}</option>
                    </select>
                    <Show when=move || tag_assign.get() == "__new__">
                        <input style="width:150px" prop:value=tag_assign_new on:input=move |event| tag_assign_new.set(event_target_value(&event)) placeholder=ctx_tr("Nuovo tag") title=ctx_tr("Nome del nuovo tag: verrà salvato e comparirà nelle tendine") />
                    </Show>
                    <button class="btn sm primary" title=ctx_tr("Assegna il tag scelto a tutti gli elementi selezionati (vuoto = rimuove il tag)") on:click=assign_tag>{ctx_tr("Assegna tag")}</button>
                    <button class="btn sm" title=ctx_tr("Rimuovi il tag scelto dagli elementi selezionati (vuoto = rimuove tutti i tag)") on:click=remove_tags>{ctx_tr("Rimuovi tag")}</button>
                    <button class="btn sm" title=ctx_tr("Metti in pausa i torrent selezionati") on:click=move |_| bulk_post("pause", None)>{ctx_tr("Pausa")}</button>
                    <button class="btn sm" title=ctx_tr("Riprendi i torrent selezionati") on:click=move |_| bulk_post("resume", None)>{ctx_tr("Riprendi")}</button>
                    <button class="btn sm" title=ctx_tr("Riavvia il check dei torrent selezionati") on:click=move |_| bulk_post("recheck", None)>{ctx_tr("Recheck")}</button>
                    <button class="btn sm" title=ctx_tr("Rimuove il blocco pin e riporta il torrent nella coda normale") on:click=move |_| { run_post(data, "/api/torrents/unpin", None, "Pin rimosso"); }>{ctx_tr("Sblocca pin")}</button>
                    <button class="btn sm danger" title=ctx_tr("Rimuovi dalla sessione i torrent selezionati") on:click=move |_| bulk_post("remove", Some(json!({"delete_files": false})))>{ctx_tr("Rimuovi")}</button>
                </div>
                <div class="table-wrap">
                    <table class="data-table torrent-table">
                        <thead><tr>
                            <th><input type="checkbox" title=ctx_tr("Seleziona tutti") prop:checked=move || all_selected.get() on:change=move |_| {
                                let items = filtered.get();
                                let torrent_hashes: Vec<String> = items.iter()
                                    .filter(|item| text(item, "__kind", "torrent") != "http")
                                    .map(|item| text(item, "hash", ""))
                                    .filter(|hash| !hash.is_empty())
                                    .collect();
                                let http_ids: Vec<String> = items.iter()
                                    .filter(|item| text(item, "__kind", "torrent") == "http")
                                    .map(|item| text(item, "__id", ""))
                                    .filter(|id| !id.is_empty())
                                    .collect();
                                let all = all_selected.get_untracked();
                                selected_torrents.update(|current| {
                                    if all {
                                        current.retain(|hash| !torrent_hashes.contains(hash));
                                    } else {
                                        for hash in &torrent_hashes {
                                            if !current.contains(hash) { current.push(hash.clone()); }
                                        }
                                    }
                                });
                                selected_http.update(|current| {
                                    if all {
                                        current.retain(|id| !http_ids.contains(id));
                                    } else {
                                        for id in &http_ids {
                                            if !current.contains(id) { current.push(id.clone()); }
                                        }
                                    }
                                });
                            } /></th>
                            <th><button type="button" class="th-sort" on:click=move |_| set_sort("name")>{ctx_tr("Nome")}</button></th>
                            <th>{ctx_tr("Stato")}</th>
                            <th><button type="button" class="th-sort" on:click=move |_| set_sort("progress")>{ctx_tr("Progresso")}</button></th>
                            <th><button type="button" class="th-sort" on:click=move |_| set_sort("dl")>{ctx_tr("↓")}</button></th>
                            <th><button type="button" class="th-sort" on:click=move |_| set_sort("ul")>{ctx_tr("↑")}</button></th>
                            <th><button type="button" class="th-sort" on:click=move |_| set_sort("eta")>{ctx_tr("ETA")}</button></th>
                            <th><button type="button" class="th-sort" on:click=move |_| set_sort("peers")>{ctx_tr("Peer / Seed")}</button></th>
                            <th><button type="button" class="th-sort" on:click=move |_| set_sort("ratio")>{ctx_tr("Ratio")}</button></th>
                            <th>{ctx_tr("Azioni")}</th>
                        </tr></thead>
                        <tbody>
                            <For
                                each=move || filtered.get()
                                key=|item: &Value| {
                                    let kind = text(item, "__kind", "torrent");
                                    if kind == "http" {
                                        format!("http:{}", text(item, "__id", ""))
                                    } else {
                                        format!("torrent:{}", text(item, "hash", ""))
                                    }
                                }
                                children=move |item| {
                                    if text(&item, "__kind", "torrent") == "http" {
                                        view! { <HttpDownloadRow id=text(&item, "__id", "") data selected=selected_http /> }.into_any()
                                    } else {
                                        view! { <TorrentRow hash=text(&item, "hash", "") data selected=selected_torrents /> }.into_any()
                                    }
                                }
                            />
                        </tbody>
                    </table>
                </div>
                <Show when=move || data.get().torrents.is_empty() && data.get().comic_downloads.is_empty()><Empty text="Nessun download nella sessione." /></Show>
            </Panel>
            <Panel title="Storico download">
                <p class="muted">{ctx_tr("Download conclusi negli ultimi 30 giorni. Il badge NAS indica che il file è stato archiviato (percorso in libreria/NAS); il tag è la regola di cartella applicata.")}</p>
                <div class="search-row" style="margin-top:10px">
                    <input prop:value=history_filter on:input=move |event| {
                        let value = event_target_value(&event);
                        history_filter.set(value.clone());
                        data.update(|current| {
                            current.history_filter = value;
                            current.history_page = 1;
                        });
                        history_goto(data, 1);
                    } placeholder=ctx_tr("Cerca in nome, tipo, tag NAS, stato o cartella…") title=ctx_tr("Cerca nello storico completo, non solo nella pagina visibile") />
                    <button type="button" class="btn" title=ctx_tr("Azzera il filtro storico") on:click=move |_| {
                        history_filter.set(String::new());
                        data.update(|current| { current.history_filter.clear(); current.history_page = 1; });
                        history_goto(data, 1);
                    }>{ctx_tr("Pulisci")}</button>
                </div>
                <Show when=move || data.get().history.is_empty()><Empty text="Nessun download concluso." /></Show>
                <div class="table-wrap" style="margin-top:10px">
                    <table class="data-table history-table">
                        <thead><tr><th>{ctx_tr("Nome")}</th><th>{ctx_tr("Tipo")}</th><th>{ctx_tr("Tag NAS")}</th><th>{ctx_tr("Score")}</th><th>{ctx_tr("Stato")}</th><th>{ctx_tr("Cartella libreria / NAS")}</th><th>{ctx_tr("Concluso")}</th></tr></thead>
                        <tbody>
                            {move || data.get().history.iter().cloned().map(|item| {
                                let progress = item.get("progress").and_then(Value::as_f64).unwrap_or(0.0);
                                let status = text(&item, "status", "queued");
                                let error = text(&item, "error", "");
                                let path = text(&item, "processed_path", "");
                                let archived = !path.is_empty();
                                let (state_label, state_tone) = if status == "error" || !error.is_empty() {
                                    (tr(data, "Errore"), "danger")
                                } else if status == "completed" || progress >= 1.0 {
                                    (tr(data, "Completato"), "ok")
                                } else {
                                    (tr(data, "In corso"), "")
                                };
                                let kind = text(&item, "kind", "");
                                let season = item.get("season").and_then(Value::as_i64).unwrap_or(0);
                                let episode = item.get("episode").and_then(Value::as_i64).unwrap_or(0);
                                let year = item.get("year").and_then(Value::as_i64).unwrap_or(0);
                                let type_label = if kind == "series" {
                                     if season > 0 && episode > 0 { format!("{} · S{season:02}E{episode:02}", tr(data, "Serie")) }
                                     else if season > 0 { format!("{} · S{season:02}", tr(data, "Serie")) }
                                     else { tr(data, "Serie") }
                                } else if kind == "movie" {
                                     if year > 0 { format!("{} · {year}", tr(data, "Film")) } else { tr(data, "Film") }
                                  } else { tr(data, "—") };
                                 let tag = text(&item, "tag", "");
                                let tag_empty = tag.is_empty();
                                let tag_label = tag.clone();
                                let name = text_tr(data, &item, "name", "Senza nome");
                                let when_full = {
                                    let completed = text(&item, "completed_at", "");
                                    if completed.is_empty() { text(&item, "updated_at", "-") } else { completed }
                                };
                                let when_short: String = when_full.chars().take(16).collect();
                                // In tabella basta la cartella finale; il percorso
                                // completo resta nel tooltip.
                                let folder = if archived { archive_folder_label(&path) } else { String::new() };
                                let path_display = if !folder.is_empty() {
                                    folder
                                } else if archived {
                                    path.clone()
                                } else {
                                    "—".to_string()
                                };
                                // Sotto il nome: perché era in download e la fonte.
                                let origin_label = {
                                    let mut parts = Vec::new();
                                    let reason_label = torrent_reason_label(data, &text(&item, "reason", ""));
                                    if !reason_label.is_empty() {
                                        parts.push(format!("{}: {reason_label}", tr(data, "Perché")));
                                    }
                                    let source_label = text(&item, "source", "");
                                    if !source_label.is_empty() {
                                        parts.push(format!("{}: {source_label}", tr(data, "Fonte")));
                                    }
                                    parts.join(" · ")
                                };
                                let origin_show = origin_label.clone();
                                let name_title = name.clone();
                                let path_title = path.clone();
                                let when_title = when_full.clone();
                                view! {
                                    <tr>
                                         <td class="truncate" title=name_title>
                                             <div class="torrent-name">
                                                 <span class="torrent-name-text">{name}</span>
                                             </div>
                                            <Show when=move || !origin_show.is_empty()>
                                                <div class="muted" style="font-size:11px;white-space:normal">{origin_label.clone()}</div>
                                            </Show>
                                        </td>
                                        <td class="muted">{type_label}</td>
                                        <td>
                                            <div class="history-tags">
                                                <Show when=move || archived>
                                                    <span class="badge ok" title=ctx_tr("File archiviato nella cartella libreria/NAS")>{ctx_tr("NAS")}</span>
                                                </Show>
                                                <Show when=move || !tag_empty>
                                                    <span class="badge" title=ctx_tr("Regola di cartella applicata")>{tag_label.clone()}</span>
                                                </Show>
                                                <Show when=move || !archived && tag_empty>
                                                    <span class="muted">{"—"}</span>
                                                </Show>
                                            </div>
                                        </td>
                                        <td class="numeric">{number(&item, "quality_score")}</td>
                                        <td><span class=format!("badge {state_tone}")>{state_label}</span></td>
                                        <td class="truncate mono muted" title=path_title>{path_display}</td>
                                        <td class="mono muted" title=when_title>{when_short}</td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
                <div class="toolbar" style="margin-top:8px">
                    <span class="muted">{move || tr_format(data, "{count} download", &[("{count}", data.get().history_total.to_string())])}</span>
                    <button class="btn sm" disabled=move || { data.get().history_page <= 1 } on:click=move |_| history_goto(data, data.get().history_page.saturating_sub(1))>{ctx_tr("Precedente")}</button>
                    <span>{move || format!("Pagina {} / {}", data.get().history_page.max(1), data.get().history_pages.max(1))}</span>
                    <button class="btn sm" disabled=move || { data.get().history_page >= data.get().history_pages } on:click=move |_| history_goto(data, data.get().history_page + 1)>{ctx_tr("Successiva")}</button>
                </div>
            </Panel>
        </div>
    }
}

#[component]
fn HttpDownloadRow(id: String, data: RwSignal<Data>, selected: RwSignal<Vec<String>>) -> impl IntoView {
    let id_lookup = id.clone();
    let id_check = id.clone();
    let id_toggle = id.clone();
    let id_pause = StoredValue::new(id.clone());
    let id_resume = StoredValue::new(id.clone());
    let id_remove = StoredValue::new(id.clone());
    let item = Signal::derive(move || {
        data.get()
            .comic_downloads
            .iter()
            .find(|entry| text(entry, "id", "") == id_lookup)
            .cloned()
            .unwrap_or(Value::Null)
    });
    // Visione normalizzata: condivide con i torrent ETA, ordinamento e unità.
    let row = Signal::derive(move || normalized_http_item(&item.get()));
    let name = Signal::derive(move || text(&item.get(), "title", "—"));
    let method = Signal::derive(move || text(&item.get(), "method", "http").to_uppercase());
    let is_http = Signal::derive(move || text(&item.get(), "method", "") == "http");
    let status = Signal::derive(move || text(&item.get(), "status", ""));
    let status_label = Signal::derive(move || match status.get().as_str() {
        "downloading" => tr(data, "In scarico"),
        "paused" => tr(data, "In pausa"),
        "completed" => tr(data, "Completato"),
        "error" => tr(data, "errore"),
        other => other.to_string(),
    });
    let tag = Signal::derive(move || text(&item.get(), "tag", ""));
    let tag_label = Signal::derive(move || {
        let value = tag.get();
        if value.is_empty() { "—".to_string() } else { value }
    });
    let error = Signal::derive(move || text(&item.get(), "error", ""));
    let url = Signal::derive(move || text(&item.get(), "url", "—"));
    let destination = Signal::derive(move || {
        let value = text(&item.get(), "destination", "");
        if value.is_empty() { "—".into() } else { value }
    });
    let temporary = Signal::derive(move || {
        let value = text(&item.get(), "temporary", "");
        if value.is_empty() { "—".into() } else { value }
    });
    let downloaded = Signal::derive(move || size(&item.get(), "downloaded_bytes"));
    let total = Signal::derive(move || {
        item.get()
            .get("total_bytes")
            .and_then(Value::as_u64)
            .map(|value| size_str(value as f64))
            .unwrap_or_else(|| "—".into())
    });
    let speed = Signal::derive(move || format!("{}/s", size(&item.get(), "speed_bytes")));
    let updated = Signal::derive(move || text(&item.get(), "updated_at", "—"));
    let progress = Signal::derive(move || {
        let value = item.get().get("progress").and_then(Value::as_f64).unwrap_or(-1.0);
        if value < 0.0 { None } else { Some(value) }
    });
    let expanded = RwSignal::new(false);
    let remove_confirm = RwSignal::new(false);
    // Rimuove la riga; con `delete_files` cancella anche il `.part` e il file
    // finale. Il closure cattura solo handle Copy, quindi resta riusabile.
    let remove = move |delete_files: bool| {
        let id = id_remove.get_value();
        spawn_local(async move {
            let result = send(
                "POST",
                &format!("/api/comics/downloads/{id}/remove"),
                Some(json!({"delete_files": delete_files})),
            )
            .await;
            flash(data, result, "Download rimosso");
            trigger_refresh();
        });
    };
    view! {
        <tr>
            <td><input type="checkbox" title=ctx_tr("Seleziona il download") prop:checked=move || selected.get().contains(&id_check) on:change=move |_| { let id = id_toggle.clone(); selected.update(|items| { if items.contains(&id) { items.retain(|value| value != &id); } else { items.push(id); } }); } /></td>
            <td class="truncate" title=move || name.get()>
                <div class="torrent-name">
                    <span class="torrent-name-text">{move || name.get()}</span>
                    <span class="badge" title=ctx_tr("Download HTTP, non un torrent")>{move || method.get()}</span>
                </div>
                <Show when=move || !tag.get().is_empty()>
                    <div class="tag-line" title=ctx_tr("Tag assegnato")>
                        {move || tag_chips(&tag.get()).into_iter().map(|chip| view! { <span class="tag-chip">{chip}</span> }).collect_view()}
                    </div>
                </Show>
                <Show when=move || !error.get().is_empty()>
                    <div class="muted" style="font-size:11px;white-space:normal">{move || error.get()}</div>
                </Show>
            </td>
            <td><span class="badge" class:ok=move || status.get() == "completed" class:warn=move || status.get() == "paused" class:err=move || status.get() == "error">{move || status_label.get()}</span></td>
            <td>
                <div class=move || match status.get().as_str() {
                    "completed" => "progress seed",
                    "paused" | "error" => "progress paused",
                    _ => "progress active",
                }><span style=move || format!("width:{:.0}%", progress.get().unwrap_or(0.0).clamp(0.0, 100.0))></span></div>
                <small class="muted">{move || progress.get().map(|value| format!("{value:.1}%")).unwrap_or_else(|| "—".into())}</small>
            </td>
            <td class="numeric">{move || format!("{}/s", size(&row.get(), "download_rate"))}</td>
            <td class="numeric">"—"</td>
            <td class="numeric" title=ctx_tr("Tempo stimato al completamento")>{move || eta_label(data, &row.get())}</td>
            <td class="numeric">"—"</td>
            <td class="numeric">"—"</td>
            <td>
                <div class="row-actions">
                    <Show when=move || is_http.get() && status.get() == "downloading">
                        <button class="btn sm" title=ctx_tr("Metti in pausa il download HTTP") on:click=move |_| run_post(data, &format!("/api/comics/downloads/{}/pause", id_pause.get_value()), None, "Download in pausa")>{ctx_tr("Pausa")}</button>
                    </Show>
                    <Show when=move || is_http.get() && status.get() == "paused">
                        <button class="btn sm" title=ctx_tr("Riprendi il download HTTP") on:click=move |_| run_post(data, &format!("/api/comics/downloads/{}/resume", id_resume.get_value()), None, "Download ripreso")>{ctx_tr("Riprendi")}</button>
                    </Show>
                    <button class="btn sm" title=ctx_tr("Dettagli del download") on:click=move |_| expanded.update(|open| *open = !*open)>{ctx_tr("Dettagli")}</button>
                    <button class="btn sm danger" title=ctx_tr("Rimuovi dalla lista; puoi scegliere se cancellare anche il file") on:click=move |_| remove_confirm.set(true)>{ctx_tr("Rimuovi")}</button>
                </div>
            </td>
        </tr>
        <Show when=move || expanded.get()>
            <tr>
                <td colspan="10">
                    <div class="modal-backdrop" on:click=move |_| expanded.set(false)>
                        <div class="modal torrent-modal" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                            <div class="modal-head">
                                <strong>{move || name.get()}</strong>
                                <div class="toolbar">
                                    <span class="mono muted">{id.clone()}</span>
                                    <button class="btn sm" on:click=move |_| expanded.set(false)>{ctx_tr("Chiudi")}</button>
                                </div>
                            </div>
                            <div class="modal-body">
                                <div class="grid-2">
                                    <StatLine label="Stato" value=status_label />
                                    <StatLine label="Metodo" value=method />
                                    <StatLine label="Progresso" value=Signal::derive(move || progress.get().map(|value| format!("{value:.1}%")).unwrap_or_else(|| "—".into())) />
                                    <StatLine label="Scaricato" value=downloaded />
                                    <StatLine label="Dimensione" value=total />
                                    <StatLine label="Velocità" value=speed />
                                    <StatLine label="ETA" value=Signal::derive(move || eta_label(data, &row.get())) />
                                    <StatLine label="Tag" value=tag_label />
                                    <StatLine label="File finale" value=destination />
                                    <StatLine label="File temporaneo" value=temporary />
                                    <StatLine label="Aggiornato" value=updated />
                                </div>
                                <div class="row span-full" style="margin-top:8px">
                                    <span class="muted">{ctx_tr("URL")}</span>
                                    <strong class="mono" style="word-break:break-all;font-size:11px">{move || url.get()}</strong>
                                </div>
                                <Show when=move || !error.get().is_empty()>
                                    <p class="muted" style="margin-top:8px">{move || format!("{}: {}", tr(data, "Errore"), error.get())}</p>
                                </Show>
                            </div>
                        </div>
                    </div>
                </td>
            </tr>
        </Show>
        <Show when=move || remove_confirm.get()>
            <tr>
                <td colspan="10">
                    <div class="modal-backdrop" on:click=move |_| remove_confirm.set(false)>
                        <div class="modal" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                            <div class="modal-head">
                                <strong>{ctx_tr("Rimuovi download")}</strong>
                                <button class="btn sm" on:click=move |_| remove_confirm.set(false)>{ctx_tr("Chiudi")}</button>
                            </div>
                            <div class="modal-body">
                                <p class="muted">{ctx_tr("Rimuovi solo la riga dalla lista oppure cancella anche il file scaricato (parziale o completo).")}</p>
                                <div class="toolbar">
                                    <button class="btn sm" on:click=move |_| { remove_confirm.set(false); remove(false); }>{ctx_tr("Rimuovi dalla lista")}</button>
                                    <Show when=move || is_http.get()>
                                        <button class="btn sm danger" on:click=move |_| { remove_confirm.set(false); remove(true); }>{ctx_tr("Rimuovi e cancella file")}</button>
                                    </Show>
                                </div>
                            </div>
                        </div>
                    </div>
                </td>
            </tr>
        </Show>
    }
}

#[component]
fn TorrentRow(hash: String, data: RwSignal<Data>, selected: RwSignal<Vec<String>>) -> impl IntoView {
    let hash_check = hash.clone();
    let hash_check_toggle = hash.clone();
    let hash_lookup = hash.clone();
    let hash_toggle = hash.clone();
    let hash_headers = hash.clone();
    let hash_pin = StoredValue::new(hash.clone());
    let hash_tag = StoredValue::new(hash.clone());
    let hash_spark = StoredValue::new(hash.clone());
    let hash_norename = StoredValue::new(hash.clone());
    let hash_trackers = StoredValue::new(hash.clone());
    let hash_files = StoredValue::new(hash.clone());
    let hash_super = StoredValue::new(hash.clone());
    let hash_web = StoredValue::new(hash.clone());
    let hash_export = StoredValue::new(hash.clone());
    let item = Signal::derive(move || {
        data.get()
            .torrents
            .iter()
            .find(|torrent| text(torrent, "hash", "").eq_ignore_ascii_case(&hash_lookup))
            .cloned()
            .unwrap_or(Value::Null)
    });
    let name = Signal::derive(move || text_tr(data, &item.get(), "name", "Metadata in attesa"));
    // Riga sotto il nome: perché è in download e da quale fonte è arrivato.
    let origin_line = Signal::derive(move || {
        let current = item.get();
        let mut parts = Vec::new();
        let reason = torrent_reason_label(data, &text(&current, "reason", ""));
        if !reason.is_empty() {
            parts.push(format!("{}: {reason}", tr(data, "Perché")));
        }
        let source = text(&current, "source", "");
        if !source.is_empty() {
            parts.push(format!("{}: {source}", tr(data, "Fonte")));
        }
        parts.join(" · ")
    });
    let state = Signal::derive(move || text(&item.get(), "state", "queued"));
    let state_label = Signal::derive(move || {
        let (label, _) = torrent_state_label(&state.get());
        tr(data, label)
    });
    let state_tone = Signal::derive(move || torrent_state_label(&state.get()).1);
    let tag_value = Signal::derive(move || {
        data.with(|current| torrent_tag_of(&current.torrent_tags, &hash_tag.get_value()))
    });
    // Seeding infinito: ratio o giorni a 0 nella configurazione del torrent.
    let seed_infinite = Signal::derive(move || {
        let current = item.get();
        current.get("seed_ratio").and_then(Value::as_f64) == Some(0.0)
            || current.get("seed_days").and_then(Value::as_i64) == Some(0)
    });
    let is_paused = Signal::derive(move || state.get().to_lowercase().contains("paus"));
    let progress = Signal::derive(move || item.get().get("progress").and_then(Value::as_f64).unwrap_or(0.0));
    let toggle_path = Signal::derive(move || {
        format!(
            "/api/torrents/{}/{}",
            hash_toggle,
            if is_paused.get() { "resume" } else { "pause" }
        )
    });
    let toggle_label = Signal::derive(move || if is_paused.get() { tr(data, "Riprendi") } else { tr(data, "Pausa") });
    let check_path = format!("/api/torrents/{hash}/recheck");
    let restart_path = StoredValue::new(format!("/api/torrents/{hash}/restart"));
    let announce_path = StoredValue::new(format!("/api/torrents/{hash}/reannounce"));
    let failed_path = StoredValue::new(format!("/api/torrents/{hash}/mark_failed"));
    let remove_path = format!("/api/torrents/{hash}");
    let remove_options_path = StoredValue::new(format!("/api/torrents/{hash}/remove"));
    let remove_mode = RwSignal::new("session".to_string());
    let remove_confirm = RwSignal::new(false);
    let detail_path = remove_path.clone();
    let peers_path = format!("/api/torrents/{hash}/peers");
    let limits_path = StoredValue::new(format!("/api/torrents/{hash}/limits"));
    let storage_path = StoredValue::new(format!("/api/torrents/{hash}/storage"));
    let expanded = RwSignal::new(false);
    let detail_tab = RwSignal::new("general".to_string());
    let peers = RwSignal::new(Vec::<Value>::new());
    let trackers = RwSignal::new(Vec::<Value>::new());
    let files = RwSignal::new(Vec::<Value>::new());
    let trackers_edit = RwSignal::new(String::new());
    let web_seeds = RwSignal::new(String::new());
    let super_seeding = RwSignal::new(false);
    let detail = RwSignal::new(Value::Null);
    let detail_magnet = RwSignal::new(String::new());
    let no_rename = RwSignal::new(false);
    let dl = RwSignal::new(String::new());
    let ul = RwSignal::new(String::new());
    let ratio = RwSignal::new(String::new());
    let days = RwSignal::new(String::new());
    let storage = RwSignal::new(String::new());
    let hash_for_paths = hash.clone();
    let toggle_expanded = move |_| {
        let next = !expanded.get();
        expanded.set(next);
        if next {
            let peers_path = peers_path.clone();
            let detail_path = detail_path.clone();
            let trackers_path = format!("/api/torrents/{hash_for_paths}/trackers");
            let files_path = format!("/api/torrents/{hash_for_paths}/files");
            let peers = peers;
            let detail = detail;
            spawn_local(async move {
                if let Ok(value) = get(&detail_path).await {
                    detail_magnet.set(text(&value, "magnet", ""));
                    no_rename.set(value.get("no_rename").and_then(Value::as_bool).unwrap_or(false));
                    let torrent = value.get("torrent").cloned().unwrap_or_else(|| value.clone());
                    let int_field = |key: &str| torrent.get(key).and_then(Value::as_i64);
                    dl.set(int_field("download_limit").filter(|value| *value >= 0).map(|value| (value / 1024).to_string()).unwrap_or_else(|| "-1".into()));
                    ul.set(int_field("upload_limit").filter(|value| *value >= 0).map(|value| (value / 1024).to_string()).unwrap_or_else(|| "-1".into()));
                    ratio.set(torrent.get("seed_ratio").and_then(Value::as_f64).map(|value| format!("{value}")).unwrap_or_else(|| "-1".into()));
                    days.set(int_field("seed_days").map(|value| value.to_string()).unwrap_or_else(|| "-1".into()));
                    detail.set(torrent);
                }
                if let Ok(value) = get(&peers_path).await {
                    peers.set(array(&value, "peers"));
                }
                if let Ok(value) = get(&trackers_path).await {
                    let items = array(&value, "trackers");
                    trackers_edit.set(
                        items
                            .iter()
                            .map(|tracker| {
                                format!(
                                    "{}|{}",
                                    tracker.get("tier").and_then(Value::as_i64).unwrap_or(0),
                                    tracker.get("url").and_then(Value::as_str).unwrap_or("")
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                    trackers.set(items);
                }
                if let Ok(value) = get(&files_path).await {
                    files.set(array(&value, "files"));
                }
            });
        }
    };
    view! {
        <tr>
            <td><input type="checkbox" title=ctx_tr("Seleziona il torrent") prop:checked=move || selected.get().contains(&hash_check) on:change=move |_| { let hash = hash_check_toggle.clone(); selected.update(|items| { if items.contains(&hash) { items.retain(|value| value != &hash); } else { items.push(hash); } }); } /></td>
            <td class="truncate" title=move || name.get()>
                <div class="torrent-name">
                    <span class="torrent-name-text">{move || name.get()}</span>
                    <Show when=move || item.get().get("archived").and_then(Value::as_bool).unwrap_or(false)><span class="badge ok" title=ctx_tr("File archiviati nella cartella NAS")>{ctx_tr("NAS")}</span></Show>
                </div>
                <Show when=move || !origin_line.get().is_empty()>
                    <div class="muted" style="font-size:11px;white-space:normal;overflow:hidden;text-overflow:ellipsis">{move || origin_line.get()}</div>
                </Show>
                <Show when=move || !tag_value.get().is_empty()>
                    <div class="tag-line" title=ctx_tr("Tag assegnato")>
                        {move || tag_chips(&tag_value.get()).into_iter().map(|chip| view! { <span class="tag-chip">{chip}</span> }).collect_view()}
                    </div>
                </Show>
            </td>
            <td>
                <Show
                    when=move || seed_infinite.get()
                    fallback=move || view! {
                        <span
                            class="badge"
                            class:ok=move || state_tone.get() == "ok"
                            class:warn=move || state_tone.get() == "warn"
                            class:err=move || state_tone.get() == "err"
                            title=move || state.get()
                        >{move || state_label.get()}</span>
                    }
                >
                    <span class="badge warn" title=move || format!("{} — {}: {}", tr(data, "Seeding infinito"), tr(data, "Stato"), state.get())>{ctx_tr("seed ∞")}</span>
                </Show>
            </td>
            <td>
                <div class=move || {
                    let value = state.get().to_lowercase();
                    if value.contains("seed") { "progress seed" }
                    else if value.contains("download") && !value.contains("metadata") { "progress active" }
                    else if value.contains("metadata") { "progress meta" }
                    else { "progress paused" }
                }><span style=move || format!("width:{:.0}%", progress.get().clamp(0.0, 100.0))></span></div>
                <small class="muted">{move || format!("{:.1}%", progress.get())}</small>
            </td>
            <td class="numeric">{move || format!("{}/s", size(&item.get(), "download_rate"))}</td>
            <td class="numeric">{move || format!("{}/s", size(&item.get(), "upload_rate"))}</td>
            <td class="numeric" title=ctx_tr("Tempo stimato al completamento")>{move || eta_label(data, &item.get())}</td>
            <td class="numeric" title=ctx_tr("Peer connessi / Seed")>{move || format!("{}: {} · {}: {}", tr(data, "Peer"), number(&item.get(), "num_peers"), tr(data, "Seed"), number(&item.get(), "num_seeds"))}</td>
            <td class="numeric" title=ctx_tr("Rapporto upload/download")>{move || ratio_label(&item.get())}</td>
            <td>
                <div class="row-actions">
                    <button class="btn sm" title=ctx_tr("Metti in pausa o riprendi il torrent") on:click=move |_| { let path = toggle_path.get_untracked(); run_post(data, &path, None, "Stato torrent aggiornato"); }>{move || toggle_label.get()}</button>
                    <button class="btn sm" title=ctx_tr("Verifica di nuovo i pezzi già scaricati") on:click=move |_| { let path = check_path.clone(); run_post(data, &path, None, "Recheck avviato"); }>{ctx_tr("Check")}</button>
                    <button class="btn sm" title=ctx_tr("Apri dettagli, azioni avanzate e limiti") on:click=toggle_expanded>{ctx_tr("Dettagli")}</button>
                    <button class="btn sm danger" title=ctx_tr("Rimuovi il torrent: chiede conferma e mostra le opzioni (mantieni o elimina i file)") on:click=move |_| remove_confirm.set(true)>{ctx_tr("Rimuovi")}</button>
                </div>
            </td>
        </tr>
        <Show when=move || expanded.get()>
            <tr>
                <td colspan="10">
                    <div class="modal-backdrop" on:click=move |_| expanded.set(false)>
                        <div class="modal torrent-modal" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                            <div class="modal-head">
                                <strong>{move || name.get()}</strong>
                                <div class="toolbar">
                                    <span class="mono muted">{hash_headers.clone()}</span>
                                    <button class="btn sm" on:click=move |_| expanded.set(false)>{ctx_tr("Chiudi")}</button>
                                </div>
                            </div>
                            <div class="tabs" style="padding:0 16px;margin:0">
                                {[("general","Generale"),("trackers","Tracker"),("files","Contenuto"),("peers","Peers"),("limits","Limiti"),("storage","Storage")].into_iter().map(|(id, label)| view! {
                                    <button class="tab" class:active=move || detail_tab.get() == id on:click=move |_| detail_tab.set(id.into())>{ctx_tr(label)}</button>
                                }).collect_view()}
                            </div>
                            <div class="modal-body">
                                <Show when=move || detail_tab.get() == "general">
                                    <div class="grid-2">
                                        <StatLine label="Stato" value=Signal::derive(move || text(&detail.get(), "state", "-")) />
                                        <StatLine label="Progresso" value=Signal::derive(move || format!("{:.1}%", detail.get().get("progress").and_then(Value::as_f64).unwrap_or(0.0))) />
                                        <StatLine label="Dimensione" value=Signal::derive(move || size(&detail.get(), "total_size")) />
                                        <StatLine label="ETA" value=Signal::derive(move || eta_label(data, &detail.get())) />
                                        <StatLine label="Scaricato" value=Signal::derive(move || size(&detail.get(), "all_time_download")) />
                                        <StatLine label="Caricato" value=Signal::derive(move || size(&detail.get(), "all_time_upload")) />
                                        <StatLine label="↓ / ↑" value=Signal::derive(move || format!("{}/s · {}/s", size(&detail.get(), "download_rate"), size(&detail.get(), "upload_rate"))) />
                                        <StatLine label="Peer / Seed" value=Signal::derive(move || format!("{} / {}", number(&detail.get(), "num_peers"), number(&detail.get(), "num_seeds"))) />
                                        <StatLine label="Limite seed" value=Signal::derive(move || {
                                            let ratio = detail.get().get("seed_ratio").and_then(Value::as_f64).unwrap_or(-1.0);
                                            let days = detail.get().get("seed_days").and_then(Value::as_i64).unwrap_or(-1);
                                            if ratio == 0.0 || days == 0 {
                                                tr(data, "∞ (infinito)")
                                            } else if ratio < 0.0 && days < 0 {
                                                tr(data, "globale")
                                            } else {
                                                tr_format(data, "ratio {ratio:.2} · {days} giorni", &[("{ratio:.2}", format!("{ratio:.2}")), ("{days}", days.to_string())])
                                            }
                                        }) />
                                        <StatLine label="Posizione coda" value=Signal::derive(move || number(&detail.get(), "queue_position")) />
                                         <StatLine label="Metadata" value=Signal::derive(move || if detail.get().get("has_metadata").and_then(Value::as_bool).unwrap_or(false) { tr(data, "presenti") } else { tr(data, "in attesa") }) />
                                         <StatLine label="Versione torrent" value=Signal::derive(move || torrent_version_label(data, &text(&detail.get(), "torrent_version", ""))) />
                                         <StatLine label="Auto-managed" value=Signal::derive(move || if detail.get().get("auto_managed").and_then(Value::as_bool).unwrap_or(false) { tr(data, "sì") } else { tr(data, "no") }) />
                                        <StatLine label="Percorso" value=Signal::derive(move || text(&detail.get(), "save_path", "-")) />
                                    </div>
                                    <div class="sparkline-wrap">
                                        <span class="muted">{ctx_tr("Velocità download (ultimi 40 campioni, ~2-3 min)")}</span>
                                        <svg class="sparkline" viewBox="0 0 200 40" preserveAspectRatio="none" aria-label="Velocità download">
                                            <polyline points=move || {
                                                let key = hash_spark.get_value();
                                                data.get().torrent_history.get(&key).map(|values| sparkline_points(values, 200.0, 40.0)).unwrap_or_default()
                                            } fill="none" />
                                        </svg>
                                    </div>
                                    <div class="toolbar" style="margin-top:12px">
                                        <Show when=move || !detail_magnet.get().is_empty()>
                                            <button class="btn sm" title=ctx_tr("Copia il link magnet negli appunti") on:click=move |_| copy_to_clipboard(&detail_magnet.get())>{ctx_tr("Copia magnet")}</button>
                                        </Show>
                                        <button class="btn sm" class:primary=move || no_rename.get() title=ctx_tr("Non rinominare i file di questo torrent (salta la rinomina TMDB al completamento)") on:click=move |_| {
                                            let next = !no_rename.get();
                                            no_rename.set(next);
                                            let path = format!("/api/torrents/{}/no_rename", hash_norename.get_value());
                                            run_post(data, &path, Some(json!({"value": next})), if next { "Rinomina disattivata per il torrent" } else { "Rinomina riattivata per il torrent" });
                                        }>{move || format!("{}{}", tr(data, "Non rinominare"), if no_rename.get() { " ✓" } else { "" })}</button>
                                        <button class="btn sm" title=ctx_tr("Forza l'annuncio a tutti i tracker") on:click=move |_| { let path = announce_path.get_value(); run_post(data, &path, None, "Reannounce richiesto"); }>{ctx_tr("Annuncia")}</button>
                                        <a class="btn sm" download=move || format!("{}.torrent", hash_export.get_value()) href=move || format!("/api/torrents/{}/export.torrent", hash_export.get_value()) title=ctx_tr("Scarica il file .torrent di questo torrent")>{ctx_tr("Esporta .torrent")}</a>
                                        <button class="btn sm" class:primary=move || super_seeding.get() title=ctx_tr("Attiva o disattiva il super seeding (initial seeding)") on:click=move |_| {
                                            let next = !super_seeding.get();
                                            super_seeding.set(next);
                                            let path = format!("/api/torrents/{}/super-seeding", hash_super.get_value());
                                            run_post(data, &path, Some(json!({"enabled": next})), if next { "Super seeding attivo" } else { "Super seeding disattivato" });
                                        }>{move || format!("{}{}", tr(data, "Super seeding"), if super_seeding.get() { " ✓" } else { "" })}</button>
                                        <button class="btn sm" title=ctx_tr("Pausa, riprende e richiede nuovi peer senza rimuovere dati o stato") on:click=move |_| { let path = restart_path.get_value(); run_post(data, &path, None, "Torrent riavviato"); }>{ctx_tr("Riavvia torrent")}</button>
                                        <button class="btn sm" title=ctx_tr("Fissa il torrent in cima alla coda") on:click=move |_| { let body = json!({"hash": hash_pin.get_value()}); run_post(data, "/api/torrents/pin", Some(body), "Torrent fissato in cima"); }>{ctx_tr("Pin")}</button>
                                        <button class="btn sm" title=ctx_tr("Assegna un tag al torrent") on:click=move |_| {
                                            let hash = hash_tag.get_value();
                                            spawn_local(async move {
                                                let Some(window) = web_sys::window() else { return };
                                                let Ok(Some(tag)) = window.prompt_with_message(&tr(data, "Tag del torrent (separati da virgola per più tag)")) else { return };
                                                let _ = send("POST", "/api/torrent-tags", Some(json!({"hash": hash, "tag": tag}))).await;
                                            });
                                        }>{ctx_tr("Tag")}</button>
                                        <button class="btn sm danger" title=ctx_tr("Segna come fallito: mette in blocklist, ripristina un eventuale upgrade e rimuove il torrent") on:click=move |_| { let path = failed_path.get_value(); run_post(data, &path, None, "Torrent segnato come fallito"); }>{ctx_tr("Segna come fallito")}</button>
                                    </div>
                                    <div class="field" style="margin-top:12px" title=ctx_tr("Cosa rimuovere quando premi Rimuovi")>
                                        <span>{ctx_tr("Rimozione")}</span>
                                        <div class="path-picker">
                                            <select prop:value=remove_mode on:change=move |event| remove_mode.set(event_target_value(&event))>
                                                <option value="session">{ctx_tr("Solo sessione")}</option>
                                                <option value="files">{ctx_tr("Sessione + file")}</option>
                                                <option value="block">{ctx_tr("Sessione + blocklist")}</option>
                                                <option value="block_files">{ctx_tr("File + blocklist")}</option>
                                            </select>
                                            <button class="btn sm danger" title=ctx_tr("Rimuove il torrent secondo l'opzione selezionata") on:click=move |_| {
                                                let mode = remove_mode.get();
                                                let delete_files = mode == "files" || mode == "block_files";
                                                let proceed = if delete_files {
                                                    web_sys::window()
                                                        .and_then(|window| window.confirm_with_message(&tr(data, "Eliminare i file dal disco? L'operazione non è reversibile.")).ok())
                                                        .unwrap_or(false)
                                                } else {
                                                    true
                                                };
                                                if !proceed { return; }
                                                let body = json!({"delete_files": delete_files, "blocklist": mode == "block" || mode == "block_files"});
                                                let path = remove_options_path.get_value();
                                                run_post(data, &path, Some(body), "Torrent rimosso");
                                            }>{ctx_tr("Rimuovi")}</button>
                                        </div>
                                    </div>
                                    <div class="field span-full" style="margin-top:12px" title=ctx_tr("Web seed HTTP/FTP: incolla uno o più URL separati da spazio per aggiungerli o rimuoverli")>
                                        <span>{ctx_tr("Web seed")}</span>
                                        <div class="path-picker">
                                            <input prop:value=web_seeds on:input=move |event| web_seeds.set(event_target_value(&event)) placeholder="https://…/file.mkv" />
                                            <button class="btn sm" on:click=move |_| { let path = format!("/api/torrents/{}/web-seeds", hash_web.get_value()); let urls: Vec<String> = web_seeds.get().split_whitespace().map(str::to_string).collect(); run_post(data, &path, Some(json!({"urls": urls, "remove": false})), "Web seed aggiunto"); }>{ctx_tr("Aggiungi")}</button>
                                            <button class="btn sm danger" on:click=move |_| { let path = format!("/api/torrents/{}/web-seeds", hash_web.get_value()); let urls: Vec<String> = web_seeds.get().split_whitespace().map(str::to_string).collect(); run_post(data, &path, Some(json!({"urls": urls, "remove": true})), "Web seed rimosso"); }>{ctx_tr("Rimuovi")}</button>
                                        </div>
                                    </div>
                                </Show>
                                <Show when=move || detail_tab.get() == "trackers">
                                    <div class="table-wrap">
                                        <table class="data-table">
                                            <thead><tr><th>{ctx_tr("Tracker")}</th><th>{ctx_tr("Tier")}</th></tr></thead>
                                            <tbody>
                                                {move || trackers.get().iter().cloned().map(|tracker| view! {
                                                    <tr><td class="mono truncate">{text(&tracker, "url", "-")}</td><td class="numeric">{number(&tracker, "tier")}</td></tr>
                                                }).collect_view()}
                                            </tbody>
                                        </table>
                                    </div>
                                    <Show when=move || trackers.get().is_empty()><Empty text="Nessun tracker disponibile (metadata non ancora risolti)." /></Show>
                                    <div class="field span-full" style="margin-top:12px" title=ctx_tr("Un tracker per riga; prefisso opzionale tier|url. Salva per sostituire l'intera lista.")>
                                        <span>{ctx_tr("Modifica tracker (tier|url per riga)")}</span>
                                        <textarea rows="4" prop:value=move || trackers_edit.get() on:input=move |event| trackers_edit.set(event_target_value(&event))></textarea>
                                    </div>
                                    <div class="toolbar">
                                        <button class="btn sm primary" on:click=move |_| {
                                            let entries = trackers_edit.get().lines().map(str::trim).filter(|line| !line.is_empty()).map(|line| {
                                                match line.split_once('|') {
                                                    Some((tier, url)) => json!({"tier": tier.trim().parse::<i32>().unwrap_or(0), "url": url.trim()}),
                                                    None => json!({"tier": 0, "url": line}),
                                                }
                                            }).collect::<Vec<_>>();
                                            let path = format!("/api/torrents/{}/trackers", hash_trackers.get_value());
                                            run_post(data, &path, Some(json!({"trackers": entries})), "Tracker aggiornati");
                                        }>{ctx_tr("Salva tracker")}</button>
                                    </div>
                                </Show>
                                <Show when=move || detail_tab.get() == "files">
                                    <div class="table-wrap">
                                        <table class="data-table">
                                            <thead><tr><th>{ctx_tr("File")}</th><th>{ctx_tr("Dimensione")}</th><th>{ctx_tr("Scaricato")}</th><th>{ctx_tr("Priorità")}</th></tr></thead>
                                            <tbody>
                                                {move || files.get().iter().enumerate().map(|(index, file)| {
                                                    let priority = file.get("priority").and_then(Value::as_i64).unwrap_or(4);
                                                    view! {
                                                        <tr>
                                                            <td class="truncate mono">{text(file, "path", "-")}</td>
                                                            <td class="numeric">{size(file, "size")}</td>
                                                            <td class="numeric">{size(file, "downloaded")}</td>
                                                            <td>
                                                                <select prop:value=priority.to_string() on:change=move |event| {
                                                                    let value = event_target_value(&event).parse::<i32>().unwrap_or(4);
                                                                    files.update(|items| { if let Some(item) = items.get_mut(index) { item["priority"] = json!(value); } });
                                                                    let priorities: Vec<i32> = files.get().iter().map(|item| item.get("priority").and_then(Value::as_i64).unwrap_or(4) as i32).collect();
                                                                    let path = format!("/api/torrents/{}/files/priority", hash_files.get_value());
                                                                    run_post(data, &path, Some(json!({"priorities": priorities})), "Priorità aggiornata");
                                                                }>
                                                                    <option value="0">{ctx_tr("Salta")}</option>
                                                                    <option value="1">{ctx_tr("Normale")}</option>
                                                                    <option value="6">{ctx_tr("Alta")}</option>
                                                                    <option value="7">{ctx_tr("Massima")}</option>
                                                                </select>
                                                            </td>
                                                        </tr>
                                                    }
                                                }).collect_view()}
                                            </tbody>
                                        </table>
                                    </div>
                                    <Show when=move || files.get().is_empty()><Empty text="Nessun file disponibile (metadata non ancora risolti)." /></Show>
                                </Show>
                                <Show when=move || detail_tab.get() == "peers">
                                    <div class="table-wrap">
                                        <table class="data-table">
                                            <thead><tr><th>{ctx_tr("Indirizzo")}</th><th>{ctx_tr("Client")}</th><th>{ctx_tr("↓")}</th><th>{ctx_tr("↑")}</th><th>{ctx_tr("Seed")}</th></tr></thead>
                                            <tbody>
                                                {move || peers.get().iter().cloned().map(|peer| view! {
                                                    <tr><td class="mono">{text(&peer, "address", "-")}</td><td class="muted">{text(&peer, "client", "?")}</td><td class="numeric">{size(&peer, "download_rate")}"/s"</td><td class="numeric">{size(&peer, "upload_rate")}"/s"</td><td>{if peer.get("seed").and_then(Value::as_bool).unwrap_or(false) { tr(data, "sì") } else { tr(data, "no") }}</td></tr>
                                                }).collect_view()}
                                            </tbody>
                                        </table>
                                    </div>
                                </Show>
                                <Show when=move || detail_tab.get() == "limits">
                                    <form class="form-grid" on:submit=move |event| { event.prevent_default(); let path = limits_path.get_value(); let dl_raw = dl.get().parse::<i64>().unwrap_or(-1); let dl_v = if dl_raw < 0 { -1 } else { dl_raw.saturating_mul(1024) }; let ul_raw = ul.get().parse::<i64>().unwrap_or(-1); let ul_v = if ul_raw < 0 { -1 } else { ul_raw.saturating_mul(1024) }; let r = ratio.get().parse::<f64>().unwrap_or(-1.0); let d = days.get().parse::<i64>().unwrap_or(-1); run_post(data, &path, Some(json!({"download_limit":dl_v,"upload_limit":ul_v,"seed_ratio":r,"seed_days":d})), "Limiti aggiornati"); }>
                                        <label class="field" title=ctx_tr("Download (KiB/s)")><span>{ctx_tr("Download (KiB/s)")}</span><input prop:value=dl on:input=move |event| dl.set(event_target_value(&event)) placeholder=ctx_tr("-1 = globale") /></label>
                                        <label class="field" title=ctx_tr("Upload (KiB/s)")><span>{ctx_tr("Upload (KiB/s)")}</span><input prop:value=ul on:input=move |event| ul.set(event_target_value(&event)) placeholder=ctx_tr("-1 = globale") /></label>
                                        <label class="field" title=ctx_tr("Ratio seed (-1 = segue la politica globale, 0 = infinito)")><span>{ctx_tr("Ratio seed")}</span><input prop:value=ratio on:input=move |event| ratio.set(event_target_value(&event)) placeholder=ctx_tr("-1 = globale · 0 = infinito") /></label>
                                        <label class="field" title=ctx_tr("Giorni di seed (-1 = segue la politica globale, 0 = infinito)")><span>{ctx_tr("Giorni seed")}</span><input prop:value=days on:input=move |event| days.set(event_target_value(&event)) placeholder=ctx_tr("-1 = globale · 0 = infinito") /></label>
                                        <label class="check span-full" title=ctx_tr("Il torrent resta in seeding senza limite di ratio né di tempo finché non lo rimuovi")><input type="checkbox" prop:checked=move || ratio.get().trim() == "0" || days.get().trim() == "0" on:change=move |event| { if event_target_checked(&event) { ratio.set("0".into()); days.set("0".into()); } else { ratio.set("-1".into()); days.set("-1".into()); } } /> <span>{ctx_tr("Seed infinito (ratio e tempo illimitati)")}</span></label>
                                        <div class="form-actions"><button class="btn primary">{ctx_tr("Salva limiti")}</button></div>
                                    </form>
                                </Show>
                                <Show when=move || detail_tab.get() == "storage">
                                    <form class="form" on:submit=move |event| { event.prevent_default(); let path = storage_path.get_value(); let value = storage.get(); run_post(data, &path, Some(json!({"path": value})), "Percorso aggiornato"); }>
                                        <label class="field span-full" title=ctx_tr("Nuovo percorso di storage")><span>{ctx_tr("Nuovo percorso di storage")}</span><input prop:value=storage on:input=move |event| storage.set(event_target_value(&event)) placeholder=ctx_tr("/mnt/nas/...") /></label>
                                        <div class="form-actions"><button class="btn primary">{ctx_tr("Sposta storage")}</button></div>
                                    </form>
                                </Show>
                            </div>
                        </div>
                    </div>
                </td>
            </tr>
        </Show>
        <Show when=move || remove_confirm.get()>
            <tr>
                <td colspan="10">
                    <div class="modal-backdrop" on:click=move |_| remove_confirm.set(false)>
                        <div class="modal remove-modal" style="width:min(720px,calc(100vw - 24px))" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                            <div class="modal-head">
                                <strong>{ctx_tr("Rimuovi torrent")}</strong>
                                <button class="btn sm" on:click=move |_| remove_confirm.set(false)>{ctx_tr("Chiudi")}</button>
                            </div>
                            <div class="modal-body">
                                <p class="remove-torrent-name">{move || name.get()}</p>
                                <div class="stack">
                                    {[
                                        ("session", false, false, "Solo torrent (mantieni i file)", "Il torrent esce dalla sessione; i file restano sul disco."),
                                        ("files", true, false, "Torrent + dati (elimina i file)", "Elimina i file dal disco. L'operazione non è reversibile."),
                                        ("block", false, true, "Torrent + blocklist (mantieni i file)", "Il torrent esce dalla sessione e la release va in blocklist."),
                                        ("block_files", true, true, "Torrent + file + blocklist", "Elimina i file e mette la release in blocklist."),
                                    ].into_iter().map(|(_mode, delete_files, blocklist, label, desc)| view! {
                                        <button class="btn remove-option" class:danger=move || delete_files || blocklist style="text-align:left" title=desc on:click=move |_| {
                                            let proceed = if delete_files {
                                                web_sys::window()
                                                    .and_then(|window| window.confirm_with_message(&tr(data, "Eliminare i file dal disco? L'operazione non è reversibile.")).ok())
                                                    .unwrap_or(false)
                                            } else {
                                                true
                                            };
                                            if !proceed { return; }
                                            let path = remove_options_path.get_value();
                                            run_post(data, &path, Some(json!({"delete_files": delete_files, "blocklist": blocklist})), "Torrent rimosso");
                                            remove_confirm.set(false);
                                        }>
                                            <strong>{ctx_tr(label)}</strong>
                                            <small>{ctx_tr(desc)}</small>
                                        </button>
                                    }).collect_view()}
                                </div>
                            </div>
                        </div>
                    </div>
                </td>
            </tr>
        </Show>
    }
}

/* ------------------------------------------------------------------ */
/* Library: series + movies                                            */
/* ------------------------------------------------------------------ */

#[component]
fn Library(data: RwSignal<Data>, mode: &'static str) -> impl IntoView {
    let title = if mode == "series" { "Serie TV" } else { "Film" };
    let name = RwSignal::new(String::new());
    let year = RwSignal::new(String::new());
    let quality = RwSignal::new(String::new());
    let language = RwSignal::new(text(&data.get().config, "default_language", "ita"));
    let seasons = RwSignal::new("1+".to_string());
    let aliases = RwSignal::new(String::new());
    let exclude = RwSignal::new(String::new());
    let archive_path = RwSignal::new(String::new());
    let selected = RwSignal::new(Option::<String>::None);
    let selected_movie = RwSignal::new(Option::<i64>::None);
    let list_filter = RwSignal::new(String::new());
    let selected_series = RwSignal::new(Vec::<String>::new());
    let bulk_language = RwSignal::new(text(&data.get().config, "default_language", "ita"));
    let tmdb_id = RwSignal::new(String::new());
    let tvdb_id = RwSignal::new(String::new());
    let tmdb_add_results = RwSignal::new(Vec::<Value>::new());
    let tmdb_loading = RwSignal::new(false);
    let tmdb_error = RwSignal::new(String::new());
    let tmdb_searched = RwSignal::new(false);
    let confirm_open = RwSignal::new(false);
    let add_series = {
        let data = data;
        move |_| {
            let value = name.get().trim().to_string();
            if value.is_empty() {
                return;
            }
            let alias_list: Vec<String> = aliases
                .get()
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect();
            let mut library = data.get().library;
            if let Some(items) = library.get_mut("series").and_then(Value::as_array_mut) {
                items.push(json!({"name": value, "seasons": seasons.get(), "quality": quality.get(), "language": language.get(), "archive_path": archive_path.get(), "timeframe": 0, "aliases": alias_list, "tmdb_id": tmdb_id.get(), "tvdb_id": tvdb_id.get(), "subtitle": "", "exclude": exclude.get(), "enabled": true, "ignored_seasons": [], "season_subfolders": false}));
            }
            save_library(data, library, "Serie aggiunta");
            name.set(String::new());
            aliases.set(String::new());
            exclude.set(String::new());
            archive_path.set(String::new());
            tmdb_id.set(String::new());
            tvdb_id.set(String::new());
            tmdb_add_results.set(Vec::new());
        }
    };
    let add_movie = {
        let data = data;
        move |_| {
            let value = name.get().trim().to_string();
            if value.is_empty() {
                return;
            }
            let mut library = data.get().library;
            if let Some(items) = library.get_mut("movies").and_then(Value::as_array_mut) {
                items.push(json!({"id": 0, "name": value, "year": year.get(), "quality": quality.get(), "language": language.get(), "enabled": true, "subtitle": "", "exclude": exclude.get(), "language_requirements": "", "subtitle_requirements": ""}));
            }
            save_library(data, library, "Film aggiunto");
            name.set(String::new());
            year.set(String::new());
            exclude.set(String::new());
        }
    };
    view! {
        <div class="view">
            <Show when=move || !(mode == "series" && selected.get().is_some())>
            <Panel title=title>
                <div class="search-row">
                    <input
                        prop:value=name
                        on:input=move |event| name.set(event_target_value(&event))
                        placeholder=move || if mode == "series" { tr(data, "Nome serie da cercare") } else { tr(data, "Nome film da cercare") }
                        title=ctx_tr("Titolo da cercare su TMDB")
                    />
                    <button class="btn primary" title=ctx_tr("Cerca il titolo su TMDB") on:click=move |_| {
                        let query = name.get();
                        if query.trim().is_empty() {
                            tmdb_error.set(tr(data, "Inserisci un titolo da cercare"));
                            return;
                        }
                        let kind = if mode == "series" { "series" } else { "movie" };
                        let results = tmdb_add_results;
                        let loading = tmdb_loading;
                        let error = tmdb_error;
                        let searched = tmdb_searched;
                        loading.set(true);
                        error.set(String::new());
                        searched.set(false);
                        results.set(Vec::new());
                        spawn_local(async move {
                            match send_timeout("POST", "/api/tmdb/search", Some(json!({"query": query, "kind": kind})), 30_000).await {
                                Ok(value) => results.set(array(&value, "items")),
                                Err(err) => {
                                    results.set(Vec::new());
                                    error.set(err);
                                }
                            }
                            loading.set(false);
                            searched.set(true);
                        });
                    }>{move || if tmdb_loading.get() { tr(data, "Cerco…") } else { tr(data, "Cerca su TMDB") }}</button>
                    <button class="btn" title=ctx_tr("Aggiungi manualmente senza usare TMDB") on:click=move |_| {
                        tmdb_add_results.set(Vec::new());
                        tmdb_error.set(String::new());
                        confirm_open.set(true);
                    }>{ctx_tr("Aggiungi manualmente")}</button>
                </div>
                <Show when=move || tmdb_loading.get()>
                    <div class="search-status" style="margin-top:10px"><span class="spinner"></span>{ctx_tr("Ricerca su TMDB in corso…")}</div>
                </Show>
                <Show when=move || !tmdb_loading.get() && !tmdb_error.get().is_empty()>
                    <div class="alert" style="margin-top:10px">{move || tmdb_error.get()}</div>
                </Show>
                <Show when=move || !tmdb_loading.get() && tmdb_error.get().is_empty() && tmdb_searched.get() && tmdb_add_results.get().is_empty()>
                    <div class="notice" style="margin-top:10px">{ctx_tr("Nessun risultato su TMDB. Controlla il titolo o aggiungi manualmente.")}</div>
                </Show>
                <Show when=move || !tmdb_add_results.get().is_empty()>
                    <div class="list" style="margin-top:10px">
                        {move || tmdb_add_results.get().iter().cloned().map(|item| {
                            let title = item.get("name").and_then(Value::as_str).or_else(|| item.get("title").and_then(Value::as_str)).unwrap_or("Titolo").to_string();
                            let overview = item.get("overview").and_then(Value::as_str).unwrap_or("").to_string();
                            let poster = item.get("poster").and_then(Value::as_str).map(str::to_string)
                                .or_else(|| item.get("poster_path").and_then(Value::as_str).map(|path| format!("https://image.tmdb.org/t/p/w92{path}")))
                                .unwrap_or_default();
                            let item_year = item.get("first_air_date").and_then(Value::as_str).or_else(|| item.get("release_date").and_then(Value::as_str)).unwrap_or("").chars().take(4).collect::<String>();
                            let external = text(&item, "external", "tmdb");
                            let is_tvdb = external == "tvdb";
                            let item_id = if is_tvdb {
                                item.get("tvdb_id").and_then(Value::as_i64).map(|value| value.to_string())
                                    .or_else(|| item.get("tvdb_id").and_then(Value::as_str).map(str::to_string))
                                    .unwrap_or_else(|| "-".into())
                            } else {
                                item.get("id").and_then(Value::as_i64).unwrap_or(0).to_string()
                            };
                            let source_label = if is_tvdb { "TVDB" } else { "TMDB" };
                            let choose_title = title.clone();
                            let choose_year = item_year.clone();
                            let choose_tmdb = if is_tvdb { String::new() } else { item_id.clone() };
                            let choose_tvdb = if is_tvdb { item_id.clone() } else { String::new() };
                            view! {
                                <div class="list-item">
                                    <div class="list-row">
                                        <div class=move || if poster.is_empty() { "list-poster placeholder" } else { "list-poster" }>
                                            {if poster.is_empty() { view! { ctx_tr("N/D") }.into_any() } else { view! { <img src=poster.clone() alt="" /> }.into_any() }}
                                        </div>
                                        <div>
                                            <strong>{title}</strong>
                                            <small>{format!("{} {} · {}", source_label, item_id, if item_year.is_empty() { tr(data, "anno n/d") } else { item_year.clone() })}</small>
                                            <small class="muted truncate">{overview}</small>
                                        </div>
                                    </div>
                                    <button class="btn sm primary" on:click=move |_| {
                                        name.set(choose_title.clone());
                                        year.set(choose_year.clone());
                                        tmdb_id.set(choose_tmdb.clone());
                                        tvdb_id.set(choose_tvdb.clone());
                                        confirm_open.set(true);
                                    }>{ctx_tr("Scegli")}</button>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </Show>
            </Panel>
            </Show>
            <Show when=move || confirm_open.get()>
                <div class="modal-backdrop" on:click=move |_| confirm_open.set(false)>
                    <div class="modal" style="width:min(560px,100%)" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                        <div class="modal-head">
                            <strong>{move || if mode == "series" { tr(data, "Conferma serie") } else { tr(data, "Conferma film") }}</strong>
                            <button class="btn sm" on:click=move |_| confirm_open.set(false)>{ctx_tr("Chiudi")}</button>
                        </div>
                        <div class="modal-body">
                            <form class="form-grid" on:submit=move |event| {
                                event.prevent_default();
                                if mode == "series" { add_series(()); } else { add_movie(()); }
                                confirm_open.set(false);
                            }>
                                <label class="field span-full" title=ctx_tr("Titolo")><span>{ctx_tr("Titolo")}</span><input prop:value=name on:input=move |event| name.set(event_target_value(&event)) placeholder=ctx_tr("Nome") /></label>
                                <Show when=move || mode == "movies">
                                    <label class="field" title=ctx_tr("Anno")><span>{ctx_tr("Anno")}</span><input prop:value=year on:input=move |event| year.set(event_target_value(&event)) placeholder=ctx_tr("2024") /></label>
                                </Show>
                        <label class="field" title=ctx_tr("Qualità richiesta")><span>{ctx_tr("Qualità richiesta")}</span><select prop:value=quality on:change=move |event| quality.set(event_target_value(&event))>{QUALITY_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}</select></label>
                                 <Show when=move || mode == "series">
                                     <label class="field" title=ctx_tr("Preset lingua o codici custom separati da virgola")><span>{ctx_tr("Lingue (preset o custom)")}</span><LanguagePresetField value=language /></label>
                                 </Show>
                                 <Show when=move || mode != "series">
                                     <label class="field" title=ctx_tr("Lingua")><span>{ctx_tr("Lingua")}</span><select prop:value=language on:change=move |event| language.set(event_target_value(&event))>{LANGUAGE_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}</select></label>
                                 </Show>
                                <label class="field span-full" title=ctx_tr("Parole che non devono comparire nel titolo della release (separate da virgola)")><span>{ctx_tr("Esclusioni (exclude)")}</span><input prop:value=exclude on:input=move |event| exclude.set(event_target_value(&event)) placeholder=ctx_tr("cam, ts, screener") /></label>
                                <Show when=move || mode == "series">
                                    <label class="field" title=ctx_tr("Quali stagioni monitorare, es. 1-3,5+")><span>{ctx_tr("Stagioni")}</span><input prop:value=seasons on:input=move |event| seasons.set(event_target_value(&event)) placeholder=ctx_tr("es. 1-3,5+") /></label>
                                    <label class="field span-full" title=ctx_tr("Nomi alternativi con cui riconoscere la serie")><span>{ctx_tr("Alias (separati da virgola)")}</span><input prop:value=aliases on:input=move |event| aliases.set(event_target_value(&event)) placeholder=ctx_tr("Nome alias, Altro alias") /></label>
                                    <div class="span-full"><PathPicker label="Percorso di salvataggio (NAS)" value=archive_path placeholder="/mnt/nas/Serie TV" /></div>
                                </Show>
                                <Show when=move || !tmdb_id.get().is_empty()>
                                    <label class="field span-full" title=ctx_tr("TMDB ID")><span>{ctx_tr("TMDB ID")}</span><input prop:value=tmdb_id readonly /></label>
                                </Show>
                                 <div class="form-actions">
                                    <button class="btn primary">{ctx_tr("Conferma")}</button>
                                    <button type="button" class="btn" on:click=move |_| confirm_open.set(false)>{ctx_tr("Annulla")}</button>
                                </div>
                            </form>
                        </div>
                    </div>
                </div>
            </Show>
            <Show when=move || mode == "series">
                <Show when=move || selected.get().is_none()>
                    <Panel title="Serie monitorate">
                    <div class="search-row" style="margin-bottom:10px">
                        <input prop:value=list_filter on:input=move |event| list_filter.set(event_target_value(&event)) placeholder=ctx_tr("Filtra serie monitorate…") title=ctx_tr("Filtra l'elenco delle serie già in libreria") />
                        <button type="button" class="btn" title=ctx_tr("Azzera il filtro") on:click=move |_| list_filter.set(String::new())>{ctx_tr("Pulisci")}</button>
                    </div>
                    <div class="toolbar" style="margin-bottom:10px">
                        <span class="muted">{move || tr_format(data, "{count} selezionate", &[("{count}", selected_series.get().len().to_string())])}</span>
                        <select prop:value=bulk_language title=ctx_tr("Lingua da applicare alle serie selezionate") on:change=move |event| bulk_language.set(event_target_value(&event))>
                            {LANGUAGE_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}
                        </select>
                        <button type="button" class="btn" title=ctx_tr("Imposta la lingua scelta su tutte le serie selezionate") on:click=move |_| {
                            let chosen = selected_series.get();
                            if chosen.is_empty() { return; }
                            let language = bulk_language.get();
                            let mut library = data.get().library;
                            if let Some(items) = library.get_mut("series").and_then(Value::as_array_mut) {
                                for item in items.iter_mut().filter(|item| chosen.contains(&text(item, "name", ""))) {
                                    item["language"] = Value::String(language.clone());
                                }
                            }
                            save_library(data, library, "Lingua aggiornata per le serie selezionate");
                            selected_series.set(Vec::new());
                        }>{ctx_tr("Imposta lingua")}</button>
                        <button type="button" class="btn danger" title=ctx_tr("Elimina dalla libreria tutte le serie selezionate") on:click=move |_| {
                            let chosen = selected_series.get();
                            if chosen.is_empty() { return; }
                            if !confirm_dialog(&tr_format(data, "Eliminare {count} serie selezionate? I file già archiviati non vengono toccati.", &[("{count}", chosen.len().to_string())])) { return; }
                            let mut library = data.get().library;
                            if let Some(items) = library.get_mut("series").and_then(Value::as_array_mut) {
                                items.retain(|item| !chosen.contains(&text(item, "name", "")));
                            }
                            save_library(data, library, "Serie selezionate eliminate");
                            selected_series.set(Vec::new());
                        }>{ctx_tr("Elimina selezionate")}</button>
                    </div>
                    <div class="table-wrap">
                        <table class="data-table lib-table series-table">
                            <thead><tr><th><input type="checkbox" title=ctx_tr("Seleziona tutte le serie monitorate") prop:checked=move || {
                                let visible = array(&data.get().library, "series");
                                !visible.is_empty() && visible.iter().all(|item| selected_series.get().contains(&text(item, "name", "")))
                            } on:change=move |_| {
                                let names = array(&data.get().library, "series").into_iter().map(|item| text(&item, "name", "")).collect::<Vec<_>>();
                                selected_series.update(|current| {
                                    if names.iter().all(|name| current.contains(name)) { current.clear(); }
                                    else { for name in names { if !current.contains(&name) { current.push(name); } } }
                                });
                            } /></th><th>{ctx_tr("Nome")}</th><th>{ctx_tr("Stagioni")}</th><th>{ctx_tr("Qualità")}</th><th>{ctx_tr("Lingua")}</th><th>{ctx_tr("Ep.")}</th><th>{ctx_tr("Complet.")}</th><th>{ctx_tr("Ultimo")}</th><th>{ctx_tr("Stato")}</th><th></th></tr></thead>
                            <tbody>
                                 {move || {
                                     let term = list_filter.get().trim().to_lowercase();
                                     let mut items = array(&data.get().library, "series");
                                     items.sort_by_cached_key(|item| text(item, "name", "").to_lowercase());
                                     items.into_iter().filter(|item| term.is_empty() || text(item, "name", "").to_lowercase().contains(&term)).map(|item| {
                                    let item_name = text_tr(data, &item, "name", "Serie");
                                    let editor_name = item_name.clone();
                                    let detail_name = item_name.clone();
                                    let open_name = item_name.clone();
                                    let delete_name = item_name.clone();
                                    let selected_name = item_name.clone();
                                    let selected_name_toggle = item_name.clone();
                                    let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                                    let total = item.get("episodes_total").and_then(Value::as_i64).unwrap_or(0);
                                    let downloaded = item.get("episodes_downloaded").and_then(Value::as_i64).unwrap_or(0);
                                    let completeness = if total > 0 { format!("{:.0}%", downloaded as f64 / total as f64 * 100.0) } else { "—".into() };
                                    let complete = total > 0 && downloaded >= total;
                                    let ended = matches!(
                                        text(&item, "tmdb_status", "").to_ascii_lowercase().as_str(),
                                        "ended" | "canceled" | "cancelled"
                                    );
                                    view! {
                                        <tr>
                                            <td><input type="checkbox" title=ctx_tr("Seleziona la serie per le azioni bulk") prop:checked=move || selected_series.get().contains(&selected_name) on:change=move |_| selected_series.update(|items| { if items.contains(&selected_name_toggle) { items.retain(|name| name != &selected_name_toggle); } else { items.push(selected_name_toggle.clone()); } }) /></td>
                                            <td><button class="btn ghost sm" on:click=move |_| selected.set(Some(open_name.clone()))>{item_name.clone()}</button></td>
                                            <td class="muted">{text(&item, "seasons", "-")}</td>
                                            <td class="muted">{text(&item, "quality", "-")}</td>
                                            <td class="muted">{text(&item, "language", "-")}</td>
                                            <td class="numeric" title=ctx_tr("Episodi scaricati / totali")>{format!("{}/{}", number(&item, "episodes_downloaded"), number(&item, "episodes_total"))}</td>
                                            <td class="numeric" title=ctx_tr("Percentuale di episodi scaricati")>{completeness}</td>
                                            <td class="muted" title=ctx_tr("Ultimo download")>{short_date(&text(&item, "last_downloaded_at", ""))}</td>
                                            <td>
                                                {if ended && complete {
                                                    view! { <span class="badge ok" title=ctx_tr("Serie terminata e completa: tutti gli episodi disponibili sono archiviati")>{ctx_tr("🏁 ✓✓")}</span> }.into_any()
                                                } else if ended {
                                                    view! { <span class="badge" title=ctx_tr("Serie terminata: mancano ancora episodi")>{ctx_tr("🏁 terminata")}</span> }.into_any()
                                                } else if complete {
                                                    view! { <span class="badge" title=ctx_tr("Al passo: tutti gli episodi pubblicati finora sono archiviati, ma la serie non è terminata")>{ctx_tr("✓ in pari")}</span> }.into_any()
                                                } else {
                                                     view! { <span class="badge" class:ok=enabled>{if enabled { tr(data, "attiva") } else { tr(data, "in pausa") }}</span> }.into_any()
                                                }}
                                                <Show when=move || (ended || complete) && !enabled>
                                                    <span class="badge" title=ctx_tr("Serie in pausa")>{ctx_tr("in pausa")}</span>
                                                </Show>
                                            </td>
                                            <td>
                                                <div class="row-actions">
                                                    <button class="btn sm primary" on:click=move |_| selected.set(Some(detail_name.clone()))>{ctx_tr("Dettagli")}</button>
                                                     <button class="btn sm" on:click=move |_| toggle_enabled(data, "series", editor_name.clone())>{if enabled { ctx_tr("Pausa") } else { ctx_tr("Attiva") }}</button>
                                                    <button class="btn sm danger" on:click=move |_| {
                                                        if confirm_dialog(&tr_format(data, "Eliminare la serie \"{name}\"? I file già archiviati non vengono toccati.", &[("{name}", delete_name.clone())])) {
                                                            remove_library(data, "series", delete_name.clone());
                                                        }
                                                    }>{ctx_tr("Elimina")}</button>
                                                </div>
                                            </td>
                                        </tr>
                                    }
                                }).collect_view()}}
                            </tbody>
                        </table>
                    </div>
                    <Show when=move || array(&data.get().library, "series").is_empty()><Empty text="Nessuna serie monitorata." /></Show>
                    </Panel>
                </Show>
                <Show when=move || selected.get().is_some()>
                    <div class="toolbar">
                        <button class="btn sm" on:click=move |_| selected.set(None)>{ctx_tr("← Torna all'elenco")}</button>
                    </div>
                    <SeriesPanel data selected />
                </Show>
            </Show>
            <Show when=move || mode == "movies">
                <MoviePanel data selected=selected_movie />
            </Show>
        </div>
    }
}

fn save_library(data: RwSignal<Data>, library: Value, success: &'static str) {
    spawn_local(async move {
        match send("POST", "/api/config/library", Some(library.clone())).await {
            Ok(_) => {
                // Rileggi dal server: il backend assegna gli id reali ai nuovi
                // film (id 0), necessari per aprire il "Dettaglio film".
                let updated = get("/api/config/library").await.ok();
                data.update(|current| {
                    current.library = updated.unwrap_or(library);
                    current.error.clear();
                    current.notice = success.into();
                });
            }
            Err(error) => data.update(|current| {
                current.notice.clear();
                current.error = error;
            }),
        }
    });
}

fn toggle_enabled(data: RwSignal<Data>, key: &str, name: String) {
    let mut library = data.get().library;
    if let Some(items) = library.get_mut(key).and_then(Value::as_array_mut) {
        if let Some(item) = items.iter_mut().find(|item| text(item, "name", "") == name) {
            let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(false);
            item["enabled"] = Value::Bool(!enabled);
        }
    }
    save_library(data, library, "Stato aggiornato");
}

fn remove_library(data: RwSignal<Data>, key: &str, name: String) {
    let mut library = data.get().library;
    if let Some(items) = library.get_mut(key).and_then(Value::as_array_mut) {
        items.retain(|item| text(item, "name", "") != name);
    }
    save_library(data, library, "Elemento rimosso");
}

fn save_series_fields(data: RwSignal<Data>, original: String, fields: Value) {
    let mut library = data.get().library;
    if let Some(items) = library.get_mut("series").and_then(Value::as_array_mut) {
        if let Some(item) = items.iter_mut().find(|item| text(item, "name", "") == original) {
            if let Some(map) = fields.as_object() {
                for (key, value) in map {
                    item[key.as_str()] = value.clone();
                }
            }
        }
    }
    save_library(data, library, "Serie aggiornata");
}

#[component]
fn SeriesPanel(data: RwSignal<Data>, selected: RwSignal<Option<String>>) -> impl IntoView {
    let detail = RwSignal::new(Value::Null);
    let series_info = RwSignal::new(Value::Null);
    let edit_open = RwSignal::new(false);
    let name = RwSignal::new(String::new());
    let archive_path = RwSignal::new(String::new());
    let timeframe = RwSignal::new(String::new());
    let seasons = RwSignal::new(String::new());
    let quality = RwSignal::new(String::new());
    let language = RwSignal::new(String::new());
    let aliases = RwSignal::new(String::new());
    let tmdb_id = RwSignal::new(String::new());
    let subtitle = RwSignal::new(String::new());
    let exclude = RwSignal::new(String::new());
    let enabled = RwSignal::new(true);
    let season_subfolders = RwSignal::new(false);
    let disable_upgrades = RwSignal::new(false);
    let rename_items = RwSignal::new(Vec::<Value>::new());
    let detail_tick = RwSignal::new(0u32);
    let scan_busy = RwSignal::new(false);
    let scan_message = RwSignal::new(String::new());
    let rename_busy = RwSignal::new(false);
    let rename_message = RwSignal::new(String::new());
    let rename_already_ok = RwSignal::new(0usize);
    let rename_open = RwSignal::new(false);
    let missing_busy = RwSignal::new(false);
    let missing_message = RwSignal::new(String::new());
    let missing_results = RwSignal::new(Vec::<Value>::new());
    let missing_searched = RwSignal::new(Vec::<Value>::new());
    let missing_filter = RwSignal::new(String::new());
    let baseline = RwSignal::new(String::new());
    let signature = move || {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            name.get(),
            seasons.get(),
            quality.get(),
            language.get(),
            archive_path.get(),
            timeframe.get(),
            aliases.get(),
            tmdb_id.get(),
            subtitle.get(),
            exclude.get(),
            enabled.get(),
            season_subfolders.get(),
            disable_upgrades.get()
        )
    };
    let dirty = Signal::derive(move || signature() != baseline.get());
    Effect::new(move |_| {
        let _ = detail_tick.get();
        if let Some(selected_name) = selected.get() {
            let info_name = selected_name.clone();
            spawn_local(async move {
                let path = format!("/api/series/{}", urlencoding::encode(&selected_name));
                if let Ok(value) = get(&path).await {
                    let series = value.get("series").cloned().unwrap_or_default();
                    name.set(text(&series, "name", &selected_name));
                    archive_path.set(text(&series, "archive_path", ""));
                    timeframe.set(raw(&series, "timeframe", "0"));
                    seasons.set(text(&series, "seasons", "1+"));
                    quality.set(text(&series, "quality", ""));
                    language.set(text(&series, "language", "ita"));
                    aliases.set(
                        series
                            .get("aliases")
                            .and_then(Value::as_array)
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default(),
                    );
                    tmdb_id.set(text(&series, "tmdb_id", ""));
                    subtitle.set(text(&series, "subtitle", ""));
                    exclude.set(text(&series, "exclude", ""));
                    enabled.set(series.get("enabled").and_then(Value::as_bool).unwrap_or(true));
                    season_subfolders.set(series.get("season_subfolders").and_then(Value::as_bool).unwrap_or(false));
                    disable_upgrades.set(series.get("disable_upgrades").and_then(Value::as_bool).unwrap_or(false));
                    detail.set(value);
                    baseline.set(signature());
                }
                if let Ok(info_value) = get(&format!(
                    "/api/series/{}/info",
                    urlencoding::encode(&info_name)
                ))
                .await
                {
                    series_info.set(info_value.get("info").cloned().unwrap_or(Value::Null));
                }
            });
        }
    });
    view! {
        <Panel title="Dettaglio serie">
            <Show when=move || selected.get().is_none()>
                <Empty text="Seleziona una serie per gestire stagioni ed episodi." />
            </Show>
            <Show when=move || selected.get().is_some()>
                <div class="stack">
                    <div class="series-hero">
                        <div class="series-poster">
                            {move || {
                                let poster = text(&series_info.get(), "poster", "");
                                if poster.is_empty() {
                                    view! { <div class="poster-placeholder">"N/D"</div> }.into_any()
                                } else {
                                    view! { <img src=poster alt="" /> }.into_any()
                                }
                            }}
                        </div>
                        <div class="series-hero-main">
                            <div class="series-hero-title">
                                <h2>{move || name.get()}</h2>
                                {move || {
                                    let info = series_info.get();
                                    let year = text(&info, "year", "");
                                    let network = text(&info, "network", "");
                                    let country = text(&info, "country", "");
                                    let vote = info.get("vote").and_then(Value::as_f64).map(|value| format!("★ {value:.1}")).unwrap_or_default();
                                     let seasons = info.get("seasons").and_then(Value::as_i64).map(|value| format!("{value} {}", tr(data, "stagioni"))).unwrap_or_default();
                                    let last = text(&info, "last_air_date", "");
                                    let last_hint = tr(data, "Ultima messa in onda");
                                    view! {
                                        {(!year.is_empty()).then(|| view! { <span class="badge">{year.clone()}</span> })}
                                        {(!network.is_empty()).then(|| view! { <span class="badge">{network.clone()}</span> })}
                                        {(!country.is_empty()).then(|| view! { <span class="badge">{country.clone()}</span> })}
                                        {(!vote.is_empty()).then(|| view! { <span class="badge" title=tr(data, "Voto TMDB")>{vote.clone()}</span> })}
                                        {(!seasons.is_empty()).then(|| view! { <span class="badge">{seasons.clone()}</span> })}
                                        {(!last.is_empty()).then(|| view! { <span class="badge" title=last_hint.clone()>{last.clone()}</span> })}
                                    }
                                }}
                                <span class="badge mono muted">{move || archive_path.get()}</span>
                                <span class="badge" class:ok=move || enabled.get()>{move || if enabled.get() { ctx_tr("attiva") } else { ctx_tr("in pausa") }}</span>
                                {move || {
                                    let ignored = detail
                                        .get()
                                        .get("series")
                                        .and_then(|series| series.get("ignored_seasons"))
                                        .and_then(Value::as_array)
                                        .cloned()
                                        .unwrap_or_default();
                                    let mut seasons: Vec<i64> = ignored.iter().filter_map(Value::as_i64).collect();
                                    seasons.sort_unstable();
                                    if seasons.is_empty() {
                                        return None;
                                    }
                                    let list = seasons.iter().map(|season| season.to_string()).collect::<Vec<_>>().join(", ");
                                     let label = format!("{}: {list}", tr(data, "Stagioni disattivate"));
                                    Some(view! {
                                        <span class="badge err" title=ctx_tr("Stagioni escluse dal monitoraggio (modificabili in Modifica serie)")>{label}</span>
                                    })
                                }}
                                {move || {
                                    let episodes = detail.get().get("episodes").and_then(Value::as_array).cloned().unwrap_or_default();
                                    let total = episodes.len();
                                    if total == 0 { return None; }
                                    let downloaded = episodes.iter().filter(|item| text(item, "status", "") == "downloaded").count();
                                    let complete = downloaded >= total;
                                    let ended = matches!(
                                        text(&series_info.get(), "status", "").to_ascii_lowercase().as_str(),
                                        "ended" | "canceled" | "cancelled"
                                    );
                                    let label = if !complete {
                                        tr(data, "In corso")
                                    } else if ended {
                                        tr(data, "Completa")
                                    } else {
                                        tr(data, "In pari")
                                    };
                                    Some(view! {
                                        <span class="badge" class:ok=complete && ended>{format!("{downloaded}/{total} · {label}")}</span>
                                    })
                                }}
                                {move || {
                                    let status = text(&series_info.get(), "status", "");
                                    if status.is_empty() { return None; }
                                     let label = if status.contains("Ended") || status.contains("Canceled") { tr(data, "Terminata") } else { tr(data, "In corso") };
                                    Some(view! { <span class="badge">{tr(data, &label)}</span> })
                                }}
                            </div>
                            <p class="series-plot">{move || {
                                let overview = text(&series_info.get(), "overview", "");
                                if overview.is_empty() { tr(data, "Nessuna trama disponibile.") } else { overview }
                            }}</p>
                            <Show when=move || !array(&series_info.get(), "cast").is_empty()>
                                <p class="series-cast">
                                    <span class="muted">{move || format!("{} · ", tr(data, "Cast"))}</span>
                                    {move || array(&series_info.get(), "cast").into_iter().map(|item| {
                                        let name = text(&item, "name", "");
                                        let url = text(&item, "url", "");
                                        if url.is_empty() {
                                            view! { <span class="cast-item">{name}</span> }.into_any()
                                        } else {
                                            view! { <a class="cast-item" href=url target="_blank" rel="noopener">{name}</a> }.into_any()
                                        }
                                    }).collect_view()}
                                </p>
                            </Show>
                            <Show when=move || !array(&series_info.get(), "genres").is_empty()>
                                <p class="series-genres"><span class="muted">{move || format!("{} · ", tr(data, "Generi"))}</span>{move || array(&series_info.get(), "genres").iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")}</p>
                            </Show>
                        </div>
                    </div>
                    <div class="toolbar">
                        <button class="btn sm" on:click=move |_| selected.set(None)>{ctx_tr("← Torna all'elenco")}</button>
                        <button class="btn sm" disabled=move || missing_busy.get() on:click=move |_| {
                            if let Some(name) = selected.get() {
                                let path = format!("/api/series/{}/search-missing", urlencoding::encode(&name));
                                let busy = missing_busy;
                                let message = missing_message;
                                let results = missing_results;
                                busy.set(true);
                                 message.set(tr(data, "Ricerca episodi mancanti in corso…"));
                                results.set(Vec::new());
                                missing_filter.set(String::new());
                                spawn_local(async move {
                                    match send("POST", &path, None).await {
                                        Ok(value) => {
                                            let searched = value
                                                .get("searched")
                                                .and_then(Value::as_i64)
                                                .unwrap_or(0);
                                            let found = array(&value, "results");
                                            let count = found.len();
                                            results.set(found);
                                            missing_searched.set(array(&value, "episodes"));
                                             message.set(if searched == 0 {
                                                 tr(data, "Nessun episodio mancante da cercare.")
                                             } else if count == 0 {
                                                 tr_format(data, "Cercati {searched} episodi mancanti: nessuna release compatibile trovata.", &[("{searched}", searched.to_string())])
                                             } else {
                                                 tr_format(data, "Cercati {searched} episodi mancanti: trovate {count} release compatibili.", &[("{searched}", searched.to_string()), ("{count}", count.to_string())])
                                             });
                                        }
                                         Err(error) => message.set(tr_format(data, "Ricerca mancanti non riuscita: {error}", &[("{error}", error)])),
                                    }
                                    busy.set(false);
                                });
                            }
                        }>{move || if missing_busy.get() { ctx_tr("Ricerca…").get() } else { ctx_tr("Cerca mancanti").get() }}</button>
                        <button class="btn sm" disabled=move || scan_busy.get() on:click=move |_| {
                            if let Some(name) = selected.get() {
                                let path = format!("/api/series/{}/scan-archive", urlencoding::encode(&name));
                                let busy = scan_busy;
                                let message = scan_message;
                                let tick = detail_tick;
                                busy.set(true);
                                 message.set(tr(data, "Scansione archivio in corso…"));
                                spawn_local(async move {
                                    match send("POST", &path, None).await {
                                        Ok(value) => {
                                            let updated = value.get("updated").and_then(Value::as_i64).unwrap_or(0);
                                            let found = value.get("found").and_then(Value::as_i64).unwrap_or(updated);
                                             message.set(tr_format(data, "Archivio scansionato: {found} file trovati, {updated} episodi aggiornati", &[("{found}", found.to_string()), ("{updated}", updated.to_string())]));
                                            tick.update(|value| *value += 1);
                                            trigger_refresh();
                                        }
                                         Err(error) => message.set(tr_format(data, "Errore scansione: {error}", &[("{error}", error)])),
                                    }
                                    busy.set(false);
                                });
                            }
                        }>{move || if scan_busy.get() { tr(data, "Scansione…") } else { tr(data, "Scansiona archivio") }}</button>
                        <button class="btn sm" on:click=move |_| {
                            if let Some(name) = selected.get() {
                                let path = format!("/api/series/{}/metadata", urlencoding::encode(&name));
                                run_post(data, &path, None, "Metadati TMDB aggiornati");
                            }
                        }>{ctx_tr("Aggiorna da TMDB")}</button>
                        <button class="btn sm" disabled=move || rename_busy.get() title=ctx_tr("Apri l'anteprima di rinomina (elenco Vecchio → Nuovo)") on:click=move |_| {
                            if let Some(name) = selected.get() {
                                let path = format!("/api/series/{}/rename-preview", urlencoding::encode(&name));
                                let items = rename_items;
                                let busy = rename_busy;
                                let msg = rename_message;
                                let already = rename_already_ok;
                                rename_open.set(true);
                                busy.set(true);
                                 msg.set(tr(data, "Anteprima rinomina in corso…"));
                                spawn_local(async move {
                                    match send("POST", &path, None).await {
                                        Ok(value) => {
                                            let list = array(&value, "items");
                                            let already_ok = value.get("already_ok_count").and_then(Value::as_u64).unwrap_or(0) as usize;
                                            let errors = list.iter().filter(|item| item.get("error").is_some()).count();
                                            already.set(already_ok);
                                            if list.is_empty() && already_ok > 0 {
                                                 msg.set(tr_format(data, "Tutti i file sono già corretti ({already_ok}).", &[("{already_ok}", already_ok.to_string())]));
                                            } else if errors > 0 {
                                                 msg.set(tr_format(data, "{pending} da rinominare, {already_ok} già corretti, {errors} errori", &[("{pending}", list.len().to_string()), ("{already_ok}", already_ok.to_string()), ("{errors}", errors.to_string())]));
                                            } else {
                                                 msg.set(tr_format(data, "{pending} da rinominare, {already_ok} già corretti", &[("{pending}", list.len().to_string()), ("{already_ok}", already_ok.to_string())]));
                                            }
                                            items.set(list);
                                        }
                                         Err(error) => msg.set(tr_format(data, "Anteprima non riuscita: {error}", &[("{error}", error)])),
                                    }
                                    busy.set(false);
                                });
                            }
                        }>{move || if rename_busy.get() { tr(data, "Attendi…") } else { tr(data, "Anteprima rinomina") }}</button>
                        <button class="btn sm" class:primary=move || edit_open.get() title=ctx_tr("Mostra o nascondi i campi di modifica della serie") on:click=move |_| edit_open.update(|value| *value = !*value)>{ctx_tr("Modifica serie")}</button>
                        {move || {
                            let url = text(&series_info.get(), "tvdb_url", "");
                            if url.is_empty() { return None; }
                            Some(view! { <a class="btn sm" href=url target="_blank" rel="noopener" title=tr(data, "Apri la serie su TheTVDB")>{tr(data, "TVDB")}</a> })
                        }}
                    </div>
                    <Show when=move || scan_busy.get()>
                        <div class="search-status" style="margin-top:8px"><span class="spinner"></span>{move || scan_message.get()}</div>
                    </Show>
                    <Show when=move || !scan_busy.get() && !scan_message.get().is_empty()>
                        <div class="notice" style="margin-top:8px">{move || scan_message.get()}</div>
                    </Show>
                    <Show when=move || !missing_message.get().is_empty()>
                        <div class="notice" style="margin-top:8px">{move || missing_message.get()}</div>
                    </Show>
                    <Show when=move || !missing_results.get().is_empty()>
                        <label class="field" style="margin-top:8px" title=ctx_tr("Filtra le release trovate per titolo, fonte o episodio; più parole restringono i risultati")>
                            <span>{ctx_tr("Filtra risultati")}</span>
                            <input prop:value=missing_filter on:input=move |event| missing_filter.set(event_target_value(&event)) placeholder=ctx_tr("Titolo, fonte o S09E11…") />
                        </label>
                    </Show>
                    <Show when=move || edit_open.get()>
                    <form class="form" on:submit=move |event| {
                        event.prevent_default();
                        if let Some(original) = selected.get() {
                            let alias_list: Vec<String> = aliases.get().split(',').map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned).collect();
                            let fields = json!({
                                "name": name.get(),
                                "seasons": seasons.get(),
                                "quality": quality.get(),
                                "language": language.get(),
                                "archive_path": archive_path.get(),
                                "timeframe": timeframe.get().parse::<i64>().unwrap_or(0),
                                "aliases": alias_list,
                                "tmdb_id": tmdb_id.get(),
                                "subtitle": subtitle.get(),
                                 "exclude": exclude.get(),
                                 "enabled": enabled.get(),
                                 "season_subfolders": season_subfolders.get(),
                                 "disable_upgrades": disable_upgrades.get(),
                                 // Il form salva l'intera libreria: conserva le
                                 // stagioni disattivate anche se sono state
                                 // cambiate poco prima con i pulsanti sotto.
                                 "ignored_seasons": detail.get().get("series").and_then(|series| series.get("ignored_seasons")).cloned().unwrap_or_else(|| json!([])),
                            });
                            save_series_fields(data, original, fields);
                            baseline.set(signature());
                        }
                    }>
                        <div class="form-grid">
                            <label class="field" title=ctx_tr("Nome della serie come viene cercata sulle fonti")><span>{ctx_tr("Titolo")}</span><input prop:value=name on:input=move |event| name.set(event_target_value(&event)) /></label>
                            <div class="field span-2" title=ctx_tr("Cartella sul NAS dove vengono archiviati gli episodi di questa serie")><span>{ctx_tr("Cartella archivio (NAS)")}</span>
                                <div class="path-picker">
                                    <input prop:value=archive_path on:input=move |event| archive_path.set(event_target_value(&event)) placeholder=ctx_tr("/home/user/SerieTVArchivio/Nome") />
                                    <BrowseButton value=archive_path />
                                </div>
                                <small class="hint">{move || if archive_path.get().is_empty() { tr(data, "Nessun percorso archivio impostato") } else { archive_path.get() }}</small>
                            </div>
                            <label class="field" title=ctx_tr("Stagioni monitorate, es. 1-3,5+ (1+ = tutte)")><span>{ctx_tr("Stagioni")}</span><input prop:value=seasons on:input=move |event| seasons.set(event_target_value(&event)) placeholder=ctx_tr("es. 1-3,5+") /></label>
                            <label class="field" title=ctx_tr("Archivia gli episodi in una cartella Stagione 01, Stagione 02, … dentro la cartella archivio della serie")><span>{ctx_tr("Sottocartelle per stagione")}</span><input type="checkbox" prop:checked=move || season_subfolders.get() on:change=move |_| season_subfolders.update(|value| *value = !*value) /></label>
                             <label class="field" title=ctx_tr("Qualità minima/desiderata delle release")><span>{ctx_tr("Qualità richiesta")}</span><select prop:value=quality on:change=move |event| quality.set(event_target_value(&event))>{QUALITY_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}</select></label>
                            <label class="field" title=ctx_tr("Se disattivo, una release migliore non sostituirà mai il file già archiviato per questa serie")><span>{ctx_tr("Consenti aggiornamenti")}</span><input type="checkbox" prop:checked=move || !disable_upgrades.get() on:change=move |_| disable_upgrades.update(|value| *value = !*value) /></label>
                            <label class="field" title=ctx_tr("Scegli un preset oppure inserisci codici custom separati da virgole; tutte le lingue indicate sono richieste")><span>{ctx_tr("Lingue (preset o custom)")}</span><LanguagePresetField value=language /></label>
                            <label class="field span-2" title=ctx_tr("Nomi alternativi con cui riconoscere la serie (separati da virgola)")><span>{ctx_tr("Alias / nomi alternativi (separati da virgola)")}</span><input prop:value=aliases on:input=move |event| aliases.set(event_target_value(&event)) /></label>
                            <label class="field" title=ctx_tr("ID TMDB della serie (per titoli episodi e poster)")><span>{ctx_tr("TMDB ID")}</span><input prop:value=tmdb_id on:input=move |event| tmdb_id.set(event_target_value(&event)) /></label>
                            <label class="field" title=ctx_tr("Attesa (ore) prima di scaricare: 0 = nessuna attesa")><span>{ctx_tr("Timeframe (ore, 0 = nessuno)")}</span><input prop:value=timeframe on:input=move |event| timeframe.set(event_target_value(&event)) placeholder=ctx_tr("0") /></label>
                            <label class="field" title=ctx_tr("Scegli un preset o inserisci sottotitoli custom separati da virgole")><span>{ctx_tr("Sottotitoli (preset o custom)")}</span><input list="series-subtitle-presets" prop:value=subtitle on:input=move |event| subtitle.set(event_target_value(&event)) placeholder=ctx_tr("nessuno, ita, eng oppure ita,eng") /><datalist id="series-subtitle-presets"><option value="">{ctx_tr("Nessuno")}</option><option value="ita">{ctx_tr("Italiano")}</option><option value="eng">{ctx_tr("Inglese")}</option><option value="ita,eng">{ctx_tr("Italiano + Inglese")}</option></datalist></label>
                            <label class="field" title=ctx_tr("Parole da escludere dai titoli (separate da virgola)")><span>{ctx_tr("Esclusioni")}</span><input prop:value=exclude on:input=move |event| exclude.set(event_target_value(&event)) /></label>
                            <div class="field" title=ctx_tr("Attiva o metti in pausa il monitoraggio della serie")><span>{ctx_tr("Stato")}</span>
                                <div class="path-picker">
                                    <select prop:value=move || if enabled.get() { "true" } else { "false" } on:change=move |event| enabled.set(event_target_value(&event) == "true")><option value="true">{ctx_tr("Attiva")}</option><option value="false">{ctx_tr("In pausa")}</option></select>
                                    <button class="btn primary" class:dirty=move || dirty.get() title=ctx_tr("Salva tutte le modifiche della serie")>{ctx_tr("Salva serie")}</button>
                                </div>
                            </div>
                        </div>
                    </form>
                    <div class="field">
                        <span>{ctx_tr("Stagioni (clicca per attivare/disattivare)")}</span>
                        <div class="toolbar">
                            {move || {
                                let ignored_seasons = detail.get().get("series").and_then(|series| series.get("ignored_seasons")).and_then(Value::as_array).cloned().unwrap_or_default();
                                let ignored: Vec<i64> = ignored_seasons.iter().filter_map(Value::as_i64).collect();
                                let max = detail.get().get("episodes").and_then(Value::as_array).map(|items| items.iter().filter_map(|item| item.get("season").and_then(Value::as_i64)).max().unwrap_or(0)).unwrap_or(0);
                                (1..=max.max(1)).map(|season| {
                                    let is_ignored = ignored.contains(&season);
                                    let class = if is_ignored { "btn sm danger" } else { "btn sm primary" };
                                            let selected_name = selected.get().unwrap_or_default();
                                            let current_detail = detail;
                                            view! {
                                                <button class=class on:click=move |_| {
                                            let path = format!("/api/series/{}/toggle-season", urlencoding::encode(&selected_name));
                                            let body = json!({"season": season, "enabled": is_ignored});
                                            let tick = detail_tick;
                                                    spawn_local(async move {
                                                        match send("POST", &path, Some(body)).await {
                                                            Ok(value) => {
                                                                // Aggiorna subito il dettaglio locale: un
                                                                // successivo "Salva serie" non può più
                                                                // sovrascrivere l'ultimo toggle con una
                                                                // copia precedente della libreria.
                                                                if let Some(ignored) = value.get("ignored_seasons").cloned() {
                                                                    current_detail.update(|detail| {
                                                                        if let Some(series) = detail.get_mut("series") {
                                                                            series["ignored_seasons"] = ignored;
                                                                        }
                                                                    });
                                                                }
                                                                tick.update(|value| *value += 1);
                                                        trigger_refresh();
                                                    }
                                                    Err(error) => data.update(|current| current.error = error),
                                                }
                                            });
                                        }>{format!("S{season}")}</button>
                                    }
                                }).collect_view()
                            }}
                        </div>
                    </div>
                    </Show>
                    <EpisodeTable data detail series_info selected missing_results missing_searched missing_filter />
                    <Show when=move || rename_open.get()>
                        <div class="modal-backdrop" on:click=move |_| rename_open.set(false)>
                            <div class="modal" style="width:min(1500px,96vw)" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                                <div class="modal-head">
                                    <strong>{ctx_tr("Anteprima rinomina TMDB")}</strong>
                                    <button type="button" class="btn sm" on:click=move |_| rename_open.set(false)>{ctx_tr("Chiudi")}</button>
                                </div>
                                <div class="modal-body">
                                    <p class="muted">{ctx_tr("I file presenti nella cartella verranno rinominati. L'operazione è irreversibile.")}</p>
                                    <Show when=move || rename_busy.get()>
                                        <div class="search-status"><span class="spinner"></span>{move || rename_message.get()}</div>
                                    </Show>
                                    <Show when=move || !rename_busy.get() && !rename_message.get().is_empty()>
                                        <div class="notice" style="margin-bottom:10px">{move || rename_message.get()}</div>
                                    </Show>
                                    <div class="table-wrap">
                                        <table class="data-table rename-table">
                                            <thead><tr><th>{ctx_tr("Ep.")}</th><th>{ctx_tr("Vecchio nome")}</th><th>{ctx_tr("Nuovo nome / errore")}</th></tr></thead>
                                            <tbody>
                                                {move || rename_items.get().iter().cloned().map(|item| {
                                                    let from = text(&item, "from", "-");
                                                    let to = text(&item, "to", "");
                                                     let error = text(&item, "error", "-");
                                                     let discarded = item.get("discarded").and_then(Value::as_bool).unwrap_or(false);
                                                     let to_full = if to.is_empty() { error.clone() } else { to.clone() };
                                                     let to_label = if discarded { tr(data, "Duplicato → cestino") } else if to.is_empty() { error } else { file_name(&to) };
                                                    view! {
                                                        <tr>
                                                            <td class="mono">{format!("S{:02}E{:02}", item.get("season").and_then(Value::as_i64).unwrap_or(0), item.get("episode").and_then(Value::as_i64).unwrap_or(0))}</td>
                                                            <td class="truncate mono muted" title=from.clone()>{file_name(&from)}</td>
                                                            <td class="truncate mono" title=to_full>{to_label}</td>
                                                        </tr>
                                                    }
                                                }).collect_view()}
                                            </tbody>
                                        </table>
                                    </div>
                                    <Show when=move || !rename_busy.get() && rename_items.get().is_empty()>
                                        <Empty text="Nessun file da rinominare." />
                                    </Show>
                                </div>
                                <div class="form-actions" style="padding:12px 16px;border-top:1px solid var(--line)">
                                    <button type="button" class="btn" on:click=move |_| rename_open.set(false)>{ctx_tr("Annulla")}</button>
                                    <button type="button" class="btn primary" disabled=move || rename_busy.get() || (rename_items.get().is_empty() && rename_already_ok.get() == 0) on:click=move |_| {
                                        if let Some(name) = selected.get() {
                                            let path = format!("/api/series/{}/rename-execute", urlencoding::encode(&name));
                                            let already = rename_already_ok.get();
                                            let pending = rename_items.get().len();
                                            // Chiede se rinominare solo i file da
                                            // sistemare o anche quelli già corretti.
                                            let force = if already > 0 {
                                                web_sys::window()
                                                    .and_then(|window| window.confirm_with_message(&tr_format(data, "{pending} file da rinominare, {already} già corretti. Rinominare anche quelli già corretti? OK = tutti, Annulla = solo i {pending}.", &[("{pending}", pending.to_string()), ("{already}", already.to_string())])).ok())
                                                    .unwrap_or(false)
                                            } else {
                                                false
                                            };
                                            run_series_rename(data, path, rename_items, rename_busy, rename_message, rename_already_ok, force);
                                        }
                                    }>{ctx_tr("Esegui rinomina")}</button>
                                    <button type="button" class="btn" disabled=move || rename_busy.get() || (rename_items.get().is_empty() && rename_already_ok.get() == 0) title=ctx_tr("Rielabora il nome esatto di tutti i file, anche di quelli già corretti") on:click=move |_| {
                                        if let Some(name) = selected.get() {
                                            let already = rename_already_ok.get();
                                            let pending = rename_items.get().len();
                                            let total = already + pending;
                                            let confirmed = web_sys::window()
                                                .and_then(|window| window.confirm_with_message(&tr_format(data, "Forzare la rinomina di {total} file? Anche i {already} già corretti verranno rielaborati. L'operazione è irreversibile.", &[("{total}", total.to_string()), ("{already}", already.to_string())])).ok())
                                                .unwrap_or(false);
                                            if !confirmed {
                                                return;
                                            }
                                            let path = format!("/api/series/{}/rename-execute", urlencoding::encode(&name));
                                            run_series_rename(data, path, rename_items, rename_busy, rename_message, rename_already_ok, true);
                                        }
                                    }>{ctx_tr("Forza rinomina")}</button>
                                </div>
                            </div>
                        </div>
                    </Show>
                </div>
            </Show>
        </Panel>
    }
}

#[component]
fn EpisodeTable(
    data: RwSignal<Data>,
    detail: RwSignal<Value>,
    series_info: RwSignal<Value>,
    selected: RwSignal<Option<String>>,
    missing_results: RwSignal<Vec<Value>>,
    missing_searched: RwSignal<Vec<Value>>,
    missing_filter: RwSignal<String>,
) -> impl IntoView {
    let search_results = RwSignal::new(Vec::<Value>::new());
    let search_label = RwSignal::new(String::new());
    let search_target = RwSignal::new(None::<(i64, i64)>);
    // Stagioni collassate di default: si espandono una alla volta.
    let expanded_seasons = RwSignal::new(Vec::<i64>::new());
    view! {
        <div class="table-wrap">
            <table class="data-table lib-table episode-table">
                <thead><tr><th>{ctx_tr("Episodio")}</th><th>{ctx_tr("Titolo")}</th><th>{ctx_tr("Stato")}</th><th>{ctx_tr("Aggiunto al client")}</th><th>{ctx_tr("Score")}</th><th>{ctx_tr("Azioni")}</th></tr></thead>
                <tbody>
                    {move || {
                        let series = selected.get().unwrap_or_default();
                        let episodes = array(&detail.get(), "episodes");
                        let mut season_list: Vec<i64> = episodes.iter().filter_map(|item| item.get("season").and_then(Value::as_i64)).collect();
                        season_list.sort_unstable();
                        season_list.dedup();
                        let expanded = expanded_seasons.get();
                        let metadata = array(&detail.get(), "metadata");
                        let info = series_info.get();
                        let show_status = text(&info, "status", "").to_ascii_lowercase();
                        let show_ended = matches!(show_status.as_str(), "ended" | "canceled" | "cancelled");
                        let last_aired_season = info
                            .get("last_episode")
                            .and_then(|episode| episode.get("season_number"))
                            .and_then(Value::as_i64)
                            .unwrap_or(0);
                        let next_aired_season = info
                            .get("next_episode")
                            .and_then(|episode| episode.get("season_number"))
                            .and_then(Value::as_i64)
                            .unwrap_or(0);
                        let missing_items = missing_results.get();
                        let missing_targets = missing_searched.get();
                        let missing_filter_text = missing_filter.get();
                        let manual_target = search_target.get();
                        let manual_items = search_results.get();
                        let manual_label = search_label.get();
                        let mut rows = Vec::new();
                        for season_group in season_list {
                            let season_items = episodes.iter().filter(|item| item.get("season").and_then(Value::as_i64) == Some(season_group)).cloned().collect::<Vec<_>>();
                            let owned = season_items.iter().filter(|item| {
                                text(item, "status", "") == "downloaded" || !text(item, "archive_path", "").is_empty()
                            }).count();
                            let metadata_total = metadata.iter().find(|entry| entry.get("season").and_then(Value::as_i64) == Some(season_group)).and_then(|entry| entry.get("count").and_then(Value::as_i64)).unwrap_or(0);
                            let max_episode = season_items.iter().filter_map(|item| item.get("episode").and_then(Value::as_i64)).max().unwrap_or(0);
                            let total = if metadata_total > 0 { metadata_total } else { max_episode };
                            let season_ended = show_ended || last_aired_season > season_group || next_aired_season > season_group;
                            let season_complete = total > 0 && owned as i64 >= total;
                            let is_expanded = expanded.contains(&season_group);
                            let toggle_season = season_group;
                            rows.push(view! {
                                <tr class="season-row">
                                    <td colspan="6">
                                        <button type="button" class="btn ghost sm" on:click=move |_| expanded_seasons.update(|items| {
                                            if items.contains(&toggle_season) { items.retain(|value| *value != toggle_season); } else { items.push(toggle_season); }
                                        })>
                                            {if is_expanded { "▾" } else { "▸" }} {format!("{} {}", tr(data, "Stagione"), season_group)}
                                            <span class="muted">{format!("· {owned}/{total}")}</span>
                                            {if season_ended {
                                                 let (label, title) = if season_complete {
                                                     ("🏁 ✓✓", tr(data, "Stagione terminata e tutti gli episodi sono presenti"))
                                                 } else {
                                                     ("🏁", tr(data, "Stagione terminata: mancano ancora episodi"))
                                                };
                                                view! { <span class="badge" class:ok=season_complete title=title>{label}</span> }.into_any()
                                            } else { view! {}.into_any() }}
                                        </button>
                                    </td>
                                </tr>
                            }.into_any());
                            if !is_expanded {
                                continue;
                            }
                            for item in season_items {
                            let season = item.get("season").and_then(Value::as_i64).unwrap_or(0);
                            let episode = item.get("episode").and_then(Value::as_i64).unwrap_or(0);
                            let ignored = item.get("ignored").and_then(Value::as_bool).unwrap_or(false);
                            let status = text(&item, "status", "missing");
                            let series_name = series.clone();
                            let series_force = series.clone();
                            let series_re = series.clone();
                            let series_del = series.clone();
                            let series_search = series.clone();
                            let missing_for_episode = missing_items
                                .iter()
                                .filter(|result| {
                                    result.get("season").and_then(Value::as_i64) == Some(season)
                                        && result.get("episode").and_then(Value::as_i64) == Some(episode)
                                        && release_matches_result_filter(result, &missing_filter_text, season, episode)
                                })
                                .cloned()
                                .collect::<Vec<_>>();
                             let missing_empty_label = if missing_filter_text.trim().is_empty() {
                                 tr(data, "Nessuna release compatibile trovata.")
                             } else {
                                 tr(data, "Nessun risultato corrisponde al filtro.")
                            };
                            let missing_searched_for_episode = missing_targets.iter().any(|target| {
                                target.get("season").and_then(Value::as_i64) == Some(season)
                                    && target.get("episode").and_then(Value::as_i64) == Some(episode)
                            });
                            let manual_searched_for_episode = manual_target == Some((season, episode));
                            let manual_for_episode = if manual_searched_for_episode {
                                manual_items.clone()
                            } else {
                                Vec::new()
                            };
                             let ignore_label = if ignored { tr(data, "Riattiva") } else { tr(data, "Ignora") };
                            let on_nas = !text(&item, "archive_path", "").is_empty();
                            // "downloaded" + "NAS" era ridondante: basta "NAS".
                            let downloaded_on_nas = status == "downloaded" && on_nas;
                             let status_label = if downloaded_on_nas {
                                 tr(data, "NAS")
                             } else {
                                 match status.as_str() {
                                     "downloaded" => tr(data, "Scaricato"),
                                     "missing" => tr(data, "Mancante"),
                                     "ignored" => tr(data, "Ignorato"),
                                     value => value.to_string(),
                                 }
                             };
                            let status_for_nas = status.clone();
                            let magnet = text(&item, "magnet_link", "");
                             let client_state = if status == "missing" || status == "downloaded" { tr(data, "No") } else { tr(data, "Sì") };
                            // Nel dettaglio mostra il nome rinominato in libreria; il
                            // titolo originale del file scaricato resta nel tooltip.
                            let original_title = text(&item, "title", "-");
                            let air_date = text(&item, "air_date", "");
                            let air_date_display = air_date.clone();
                            let renamed_title = text(&item, "renamed_title", "");
                            let (title_display, title_tooltip) = if renamed_title.is_empty() {
                                (original_title.clone(), String::new())
                            } else {
                                (renamed_title.clone(), original_title.clone())
                            };
                            rows.push(view! {
                                <tr>
                                    <td class="mono">{format!("S{season:02}E{episode:02}")}</td>
                                    <td class="truncate" title=title_tooltip>
                                        <div>{title_display}</div>
                                        <Show when=move || !air_date.is_empty()>
                                             <small class="muted">{format!("{}: {air_date_display}", tr(data, "In onda / prevista"))}</small>
                                        </Show>
                                    </td>
                                    <td>
                                        <span class="badge" class:ok=downloaded_on_nas || status=="downloaded">{status_label.clone()}</span>
                                        <Show when=move || on_nas && status_for_nas != "downloaded">
                                            <span class="badge ok" title=ctx_tr("File presente nella cartella NAS")>{ctx_tr("NAS")}</span>
                                        </Show>
                                        <Show when=move || ignored><span class="badge warn">{ctx_tr("ignorato")}</span></Show>
                                    </td>
                                     <td><span class="badge" class:ok=status!="missing" && status!="downloaded">{client_state}</span></td>
                                    <td class="numeric">{number(&item, "quality_score")}</td>
                                    <td>
                                        <div class="toolbar">
                                            <button class="btn sm" title=ctx_tr("Cerca manualmente questa puntata su feed, indexer, motori e archivio") on:click=move |_| {
                                                let path = format!("/api/episodes/{}/{}/{}/search", urlencoding::encode(&series_search), season, episode);
                                                let results = search_results;
                                                let label = search_label;
                                                let target = search_target;
                                                target.set(Some((season, episode)));
                                                spawn_local(async move {
                                                    match send("POST", &path, None).await {
                                                        Ok(value) => {
                                                            let count = array(&value, "results").len();
                                                            let feeds = number(&value, "feed_matches");
                                                            results.set(array(&value, "results"));
                                                             label.set(format!("S{season:02}E{episode:02}: {count} {} ({} {})", tr(data, "risultati"), feeds, tr(data, "dai feed")));
                                                        }
                                                        Err(error) => data.update(|current| current.error = error),
                                                    }
                                                });
                                            } >{ctx_tr("Cerca")}</button>
                                            <button class="btn sm" title=ctx_tr("Copia il magnet associato all'episodio") disabled=magnet.is_empty() on:click=move |_| copy_to_clipboard(&magnet)>{ctx_tr("Copia magnet")}</button>
                                            <button class="btn sm" on:click=move |_| {
                                                let path = format!("/api/episodes/{}/{}/{}/ignore", urlencoding::encode(&series_name), season, episode);
                                                run_post(data, &path, Some(json!({"ignored": !ignored, "reason": "ui"})), "Episodio aggiornato");
                                            }>{ignore_label}</button>
                                            <button class="btn sm" on:click=move |_| {
                                                let path = format!("/api/episodes/{}/{}/{}/force", urlencoding::encode(&series_force), season, episode);
                                                run_post(data, &path, None, "Episodio forzato");
                                            }>{ctx_tr("Forza")}</button>
                                            <button class="btn sm" on:click=move |_| {
                                                let path = format!("/api/episodes/{}/{}/{}/redownload", urlencoding::encode(&series_re), season, episode);
                                                run_post(data, &path, None, "Episodio in coda");
                                            }>{ctx_tr("Riscarica")}</button>
                                            <button class="btn sm danger" on:click=move |_| {
                                                let path = format!("/api/episodes/{}/{}/{}", urlencoding::encode(&series_del), season, episode);
                                                run_delete(data, &path, "Episodio eliminato");
                                            }>{ctx_tr("Elimina")}</button>
                                        </div>
                                    </td>
                                </tr>
                            }.into_any());
                            if missing_searched_for_episode {
                                rows.push(view! {
                                    <tr class="episode-search-row">
                                        <td colspan="6">
                                             <strong>{format!("{} · S{season:02}E{episode:02}", tr(data, "Cerca mancanti"))}</strong>
                                            {if missing_for_episode.is_empty() {
                                                view! { <p class="muted">{missing_empty_label}</p> }.into_any()
                                            } else {
                                                view! {
                                                    <div class="episode-search-list">
                                                        {missing_for_episode.into_iter().map(|item| {
                                                            let release = item.get("release").cloned().unwrap_or_default();
                                                            let queued = release.clone();
                                                            let origin = text(&item, "origin", "");
                                                            view! {
                                                                <div class="episode-search-item">
                                                                    <span class="truncate">{text(&release, "title", "Release")}</span>
                                                                    <span class="muted">{text(&release, "source", "-")}{if origin.is_empty() { String::new() } else { format!(" · {origin}") }}</span>
                                                                    <button class="btn sm primary" on:click=move |_| run_post(data, "/api/search/add", Some(json!({"release": queued.clone()})), "Release accodata")>{ctx_tr("Accoda")}</button>
                                                                </div>
                                                            }
                                                        }).collect_view()}
                                                    </div>
                                                }.into_any()
                                            }}
                                        </td>
                                    </tr>
                                }.into_any());
                            }
                            if manual_searched_for_episode {
                                rows.push(view! {
                                    <tr class="episode-search-row">
                                        <td colspan="6">
                                            <strong>{manual_label.clone()}</strong>
                                            {if manual_for_episode.is_empty() {
                                                 view! { <p class="muted">{ctx_tr("Nessuna release compatibile trovata.")}</p> }.into_any()
                                            } else {
                                                view! {
                                                    <div class="episode-search-list">
                                                        {manual_for_episode.into_iter().map(|item| {
                                                            let release = item.get("release").cloned().unwrap_or_default();
                                                            let queued = release.clone();
                                                            let origin = text(&item, "origin", "");
                                                            view! {
                                                                <div class="episode-search-item">
                                                                    <span class="truncate">{text(&release, "title", "Release")}</span>
                                                                    <span class="muted">{text(&release, "source", "-")}{if origin.is_empty() { String::new() } else { format!(" · {origin}") }}</span>
                                                                    <button class="btn sm primary" on:click=move |_| run_post(data, "/api/search/add", Some(json!({"release": queued.clone()})), "Release accodata")>{ctx_tr("Accoda")}</button>
                                                                </div>
                                                            }
                                                        }).collect_view()}
                                                    </div>
                                                }.into_any()
                                            }}
                                        </td>
                                    </tr>
                                }.into_any());
                            }
                            }
                        }
                        rows.collect_view()
                    }}
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn MoviePanel(data: RwSignal<Data>, selected: RwSignal<Option<i64>>) -> impl IntoView {
    let tab = RwSignal::new("config".to_string());
    let detail = RwSignal::new(Value::Null);
    let best_matches = RwSignal::new(Vec::<Value>::new());
    let best_filter = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let year = RwSignal::new(String::new());
    let quality = RwSignal::new(String::new());
    let language = RwSignal::new(String::new());
    let subtitle = RwSignal::new(String::new());
    let exclude = RwSignal::new(String::new());
    let disable_upgrades = RwSignal::new(false);
    let list_filter = RwSignal::new(String::new());
    let list_sort = RwSignal::new("name".to_string());
    let selected_movies = RwSignal::new(Vec::<String>::new());
    let bulk_movie_language = RwSignal::new(text(&data.get().config, "default_language", "ita"));
    let lang_a = RwSignal::new(String::new());
    let lang_b = RwSignal::new(String::new());
    let lang_c = RwSignal::new(String::new());
    let lang_a_req = RwSignal::new(true);
    let lang_b_req = RwSignal::new(true);
    let lang_c_req = RwSignal::new(true);
    let subtitle_reqs = RwSignal::new(String::new());
    // ID esterni inseriti a mano: se popolati vengono usati per i metadati.
    let movie_tmdb_id = RwSignal::new(String::new());
    let movie_tvdb_id = RwSignal::new(String::new());
    let metadata_open = RwSignal::new(false);
    let metadata_query = RwSignal::new(String::new());
    let metadata_source = RwSignal::new("tmdb".to_string());
    let metadata_results = RwSignal::new(Vec::<Value>::new());
    let metadata_result_source = RwSignal::new(String::new());
    let metadata_loading = RwSignal::new(false);
    let metadata_error = RwSignal::new(String::new());
    Effect::new(move |_| {
        if let Some(id) = selected.get() {
            spawn_local(async move {
                if let Ok(value) = get(&format!("/api/movies/{id}")).await {
                    let movie = value.get("movie").cloned().unwrap_or_default();
                    name.set(text(&movie, "name", ""));
                    year.set(text(&movie, "year", ""));
                    quality.set(text(&movie, "quality", ""));
                    language.set(text(&movie, "language", ""));
                    subtitle.set(text(&movie, "subtitle", ""));
                    exclude.set(text(&movie, "exclude", ""));
                    disable_upgrades.set(movie.get("disable_upgrades").and_then(Value::as_bool).unwrap_or(false));
                    movie_tmdb_id.set(text(&movie, "tmdb_id", ""));
                    movie_tvdb_id.set(text(&movie, "tvdb_id", ""));
                    let entries = parse_language_entries(&text(&movie, "language_requirements", ""));
                    let pick = |index: usize| {
                        entries
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| (String::new(), false))
                    };
                    let (first, first_req) = pick(0);
                    let (second, second_req) = pick(1);
                    let (third, third_req) = pick(2);
                    lang_a.set(first);
                    lang_a_req.set(first_req);
                    lang_b.set(second);
                    lang_b_req.set(second_req);
                    lang_c.set(third);
                    lang_c_req.set(third_req);
                    subtitle_reqs.set(display_requirements(&text(&movie, "subtitle_requirements", "")));
                    best_matches.set(array(&value, "matches"));
                    detail.set(value);
                }
            });
        }
    });
    view! {
        <div class="view">
            <div class="tabs">
                <button class="tab" class:active=move || tab.get() == "config" on:click=move |_| tab.set("config".into())>{ctx_tr("Monitorati")}</button>
                <button class="tab" class:active=move || tab.get() == "downloaded" on:click=move |_| tab.set("downloaded".into())>{ctx_tr("Scaricati")}</button>
            </div>
            <Show when=move || tab.get() == "config">
                <Show when=move || selected.get().is_none()>
                    <Panel title="Film monitorati">
                        <div class="search-row" style="margin-bottom:10px">
                            <input prop:value=list_filter on:input=move |event| list_filter.set(event_target_value(&event)) placeholder=ctx_tr("Filtra film monitorati…") title=ctx_tr("Filtra l'elenco dei film già in libreria") />
                            <button type="button" class="btn" title=ctx_tr("Azzera il filtro") on:click=move |_| list_filter.set(String::new())>{ctx_tr("Pulisci")}</button>
                        </div>
                        <div class="toolbar" style="margin-bottom:10px">
                            <span class="muted">{move || tr_format(data, "{count} selezionati", &[("{count}", selected_movies.get().len().to_string())])}</span>
                            <select prop:value=bulk_movie_language title=ctx_tr("Lingua da applicare ai film selezionati") on:change=move |event| bulk_movie_language.set(event_target_value(&event))>
                                {LANGUAGE_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}
                            </select>
                            <button type="button" class="btn" title=ctx_tr("Imposta la lingua scelta su tutti i film selezionati") on:click=move |_| {
                                let chosen = selected_movies.get();
                                if chosen.is_empty() { return; }
                                let language = bulk_movie_language.get();
                                let mut library = data.get().library;
                                if let Some(items) = library.get_mut("movies").and_then(Value::as_array_mut) {
                                    for item in items.iter_mut().filter(|item| chosen.contains(&text(item, "name", ""))) {
                                        item["language"] = Value::String(language.clone());
                                    }
                                }
                                save_library(data, library, "Lingua aggiornata per i film selezionati");
                                selected_movies.set(Vec::new());
                            }>{ctx_tr("Imposta lingua")}</button>
                            <button type="button" class="btn danger" title=ctx_tr("Elimina dalla libreria tutti i film selezionati") on:click=move |_| {
                                let chosen = selected_movies.get();
                                if chosen.is_empty() { return; }
                                if !confirm_dialog(&tr_format(data, "Eliminare {count} film selezionati? I file già archiviati non vengono toccati.", &[("{count}", chosen.len().to_string())])) { return; }
                                let mut library = data.get().library;
                                if let Some(items) = library.get_mut("movies").and_then(Value::as_array_mut) {
                                    items.retain(|item| !chosen.contains(&text(item, "name", "")));
                                }
                                save_library(data, library, "Film selezionati eliminati");
                                selected_movies.set(Vec::new());
                            }>{ctx_tr("Elimina selezionati")}</button>
                        </div>
                        <div class="table-wrap">
                            <table class="data-table lib-table movie-table">
                                <thead><tr>
                                    <th><input type="checkbox" title=ctx_tr("Seleziona tutti i film monitorati") prop:checked=move || {
                                        let visible = array(&data.get().library, "movies");
                                        !visible.is_empty() && visible.iter().all(|item| selected_movies.get().contains(&text(item, "name", "")))
                                    } on:change=move |_| {
                                        let names = array(&data.get().library, "movies").into_iter().map(|item| text(&item, "name", "")).collect::<Vec<_>>();
                                        selected_movies.update(|current| {
                                            if names.iter().all(|name| current.contains(name)) { current.clear(); }
                                            else { for name in names { if !current.contains(&name) { current.push(name); } } }
                                        });
                                    } /></th>
                                    <th><button type="button" class="th-sort" title=ctx_tr("Ordina per titolo") on:click=move |_| list_sort.set("name".into())>{ctx_tr("Nome")}</button></th>
                                    <th><button type="button" class="th-sort" title=ctx_tr("Ordina per anno") on:click=move |_| list_sort.set("year".into())>{ctx_tr("Anno")}</button></th>
                                    <th><button type="button" class="th-sort" title=ctx_tr("Ordina per qualità") on:click=move |_| list_sort.set("quality".into())>{ctx_tr("Qualità")}</button></th>
                                    <th><button type="button" class="th-sort" title=ctx_tr("Ordina per lingua") on:click=move |_| list_sort.set("language".into())>{ctx_tr("Lingua")}</button></th>
                                    <th>{ctx_tr("Stato")}</th><th></th>
                                </tr></thead>
                                <tbody>
                                    {move || {
                                        let term = list_filter.get().trim().to_lowercase();
                                        let key = list_sort.get();
                                        let mut movies = array(&data.get().library, "movies");
                                        movies.sort_by_key(|item| text(item, &key, "").to_lowercase());
                                        movies.into_iter().filter(|item| term.is_empty() || text(item, "name", "").to_lowercase().contains(&term)).map(|item| {
                                        let id = item.get("id").and_then(Value::as_i64).unwrap_or(0);
                                        let item_name = text_tr(data, &item, "name", "Film");
                                        let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                                        let toggle_name = item_name.clone();
                                        let delete_name = item_name.clone();
                                        let select_name = item_name.clone();
                                        let select_name_toggle = item_name.clone();
                                        view! {
                                            <tr>
                                                <td><input type="checkbox" title=ctx_tr("Seleziona il film per le azioni bulk") prop:checked=move || selected_movies.get().contains(&select_name) on:change=move |_| selected_movies.update(|items| { if items.contains(&select_name_toggle) { items.retain(|name| name != &select_name_toggle); } else { items.push(select_name_toggle.clone()); } }) /></td>
                                                <td><button class="btn ghost sm" on:click=move |_| selected.set(Some(id))>{item_name.clone()}</button></td>
                                                <td class="muted">{text(&item, "year", "-")}</td>
                                                <td class="muted">{text(&item, "quality", "-")}</td>
                                                <td class="muted">{text(&item, "language", "-")}</td>
                                                 <td><span class="badge" class:ok=enabled>{if enabled { tr(data, "attivo") } else { tr(data, "pausa") }}</span></td>
                                                <td>
                                                    <div class="toolbar">
                                                         <button class="btn sm" on:click=move |_| toggle_enabled(data, "movies", toggle_name.clone())>{if enabled { ctx_tr("Pausa") } else { ctx_tr("Attiva") }}</button>
                                                        <button class="btn sm danger" on:click=move |_| {
                                                            if confirm_dialog(&tr_format(data, "Eliminare il film \"{name}\"? I file già archiviati non vengono toccati.", &[("{name}", delete_name.clone())])) {
                                                                remove_library(data, "movies", delete_name.clone());
                                                            }
                                                        }>{ctx_tr("Elimina")}</button>
                                                    </div>
                                                </td>
                                            </tr>
                                        }
                                    }).collect_view()}}
                                </tbody>
                            </table>
                        </div>
                    </Panel>
                </Show>
                <Show when=move || selected.get().is_some()>
                    <Panel title="Dettaglio film">
                        <div class="toolbar" style="margin-bottom:12px">
                            <button class="btn" title=ctx_tr("Torna all'elenco dei film monitorati") on:click=move |_| selected.set(None)>{ctx_tr("← Torna all'elenco")}</button>
                            {move || {
                                let movie = detail.get().get("movie").cloned().unwrap_or_default();
                                let metadata = detail.get().get("metadata").cloned().unwrap_or_default();
                                let name = text(&movie, "name", "");
                                let year = text(&movie, "year", "");
                                let query = if year.is_empty() { name.clone() } else { format!("{name} {year}") };
                                let tmdb_url = metadata
                                    .get("id")
                                    .and_then(Value::as_i64)
                                    .map(|id| format!("https://www.themoviedb.org/movie/{id}"))
                                    .unwrap_or_else(|| format!("https://www.themoviedb.org/search?query={}", urlencoding::encode(&query)));
                                // I film monitorati non hanno un ID TVDB persistito: il
                                // link apre la ricerca TVDB già compilata come fallback.
                                let tvdb_url = format!("https://thetvdb.com/search?query={}", urlencoding::encode(&query));
                                view! {
                                    <a class="btn sm" href=tmdb_url target="_blank" rel="noopener" title=ctx_tr("Apri i dettagli del film su TMDB")>{ctx_tr("TMDB")}</a>
                                    <a class="btn sm" href=tvdb_url target="_blank" rel="noopener" title=ctx_tr("Cerca il film su TheTVDB")>{ctx_tr("TVDB")}</a>
                                }
                            }}
                        </div>
                        <div class="detail-head">
                            {move || match detail.get().get("metadata").and_then(|item| item.get("poster_path")).and_then(Value::as_str) {
                                Some(path) if !path.is_empty() => {
                                    let poster_url = if path.starts_with("http://") || path.starts_with("https://") { path.to_string() } else { format!("https://image.tmdb.org/t/p/w185{path}") };
                                    view! { <img class="detail-poster" src=poster_url alt="Locandina film" loading="lazy" /> }.into_any()
                                }
                                None => view! { <div class="detail-poster placeholder">{ctx_tr("N/D")}</div> }.into_any(),
                                _ => view! { <div class="detail-poster placeholder">{ctx_tr("N/D")}</div> }.into_any(),
                            }}
                            <div class="detail-body">
                                <strong>{move || text_tr(data, &detail.get().get("metadata").cloned().unwrap_or_default(), "title", "Dettaglio film")}</strong>
                                <span class="muted">{move || text(&detail.get().get("movie").cloned().unwrap_or_default(), "year", "")}</span>
                                <p>{move || text_tr(data, &detail.get().get("metadata").cloned().unwrap_or_default(), "overview", "Nessuna trama disponibile.")}</p>
                                <div class="cast-chips">
                                    {move || array(&detail.get(), "cast").into_iter().take(10).map(|member| {
                                        let actor = text(&member, "name", "");
                                        let role = text(&member, "character", "");
                                        view! { <span class="cast-chip" title=role>{actor}</span> }
                                    }).collect_view()}
                                </div>
                            </div>
                        </div>
                            <form class="form" on:submit=move |event| {
                                event.prevent_default();
                                if let Some(id) = selected.get() {
                                    let requirements = serialize_language_entries(&[
                                        (lang_a.get(), lang_a_req.get()),
                                        (lang_b.get(), lang_b_req.get()),
                                        (lang_c.get(), lang_c_req.get()),
                                    ]);
                                    let body = json!({"name": name.get(), "year": year.get(), "quality": quality.get(), "language": language.get(), "subtitle": subtitle.get(), "exclude": exclude.get(), "language_requirements": requirements, "subtitle_requirements": subtitle_reqs.get(), "tmdb_id": movie_tmdb_id.get(), "tvdb_id": movie_tvdb_id.get(), "disable_upgrades": disable_upgrades.get()});
                                    let path = format!("/api/movies/{id}");
                                    run_post(data, &path, Some(body), "Film aggiornato");
                                }
                            }>
                                <div class="form-grid">
                                     <label class="field" title=ctx_tr("Titolo")><span>{ctx_tr("Titolo")}</span><input prop:value=name on:input=move |event| name.set(event_target_value(&event)) placeholder=ctx_tr("Nome film") /></label>
                                     <label class="field" title=ctx_tr("Anno")><span>{ctx_tr("Anno")}</span><input prop:value=year on:input=move |event| year.set(event_target_value(&event)) placeholder=ctx_tr("2024") /></label>
                                     <div class="movie-metadata-action">
                                         <button type="button" class="btn" title=ctx_tr("Cerca e scegli i metadati del film: non modifica scaricamenti, qualità o lingue") on:click=move |_| {
                                             let query = if year.get().trim().is_empty() { name.get() } else { format!("{} {}", name.get(), year.get()) };
                                             metadata_query.set(query);
                                             metadata_results.set(Vec::new());
                                             metadata_error.set(String::new());
                                             metadata_open.set(true);
                                         }>{ctx_tr("Aggiorna da TMDB/TVDB")}</button>
                                     </div>
                                     <label class="field" title=ctx_tr("Qualità richiesta")><span>{ctx_tr("Qualità richiesta")}</span><select prop:value=quality on:change=move |event| quality.set(event_target_value(&event))>{QUALITY_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}</select></label>
                                <label class="field" title=ctx_tr("Se disattivo, una release migliore non sostituirà mai il file già archiviato per questo film")><span>{ctx_tr("Consenti aggiornamenti")}</span><input type="checkbox" prop:checked=move || !disable_upgrades.get() on:change=move |_| disable_upgrades.update(|value| *value = !*value) /></label>
                                 <label class="field" title=ctx_tr("Lingua")><span>{ctx_tr("Lingua")}</span><select prop:value=language on:change=move |event| language.set(event_target_value(&event))>{LANGUAGE_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}</select></label>
                                <div class="grid-2" style="grid-column: 1 / -1">
                                    <label class="field" title=ctx_tr("ID numerico TMDB del film. Se impostato viene usato per i metadati (titolo, trama, locandina, cast) al posto della ricerca per nome.")><span>{ctx_tr("TMDB ID")}</span><input prop:value=movie_tmdb_id on:input=move |event| movie_tmdb_id.set(event_target_value(&event)) placeholder=ctx_tr("es. 27205") /></label>
                                    <label class="field" title=ctx_tr("ID TheTVDB del film (usato dalla ricerca metadati TVDB). Per il dettaglio film i metadati vengono da TMDB.")><span>{ctx_tr("TVDB ID")}</span><input prop:value=movie_tvdb_id on:input=move |event| movie_tvdb_id.set(event_target_value(&event)) placeholder=ctx_tr("es. 1234") /></label>
                                </div>
                                <label class="field span-full" title=ctx_tr("Parole che non devono comparire nel titolo della release (separate da virgola)")><span>{ctx_tr("Parole vietate (exclude)")}</span><input prop:value=exclude on:input=move |event| exclude.set(event_target_value(&event)) placeholder=ctx_tr("cam, ts, screener") /></label>
                                    <div class="grid-2" style="grid-column: 1 / -1">
                                        <label class="field" title=ctx_tr("Facoltativo. Lingue di sottotitoli preferite (es. ita, eng): non bloccano il download, ma a parità di qualità viene preferita una release che le contiene (piccolo bonus).")><span>{ctx_tr("Sottotitoli (facoltativi)")}</span><input prop:value=subtitle on:input=move |event| subtitle.set(event_target_value(&event)) placeholder=ctx_tr("es. ita, eng") /></label>
                                        <label class="field" title=ctx_tr("Obbligatorio. Lingue di sottotitoli richieste (es. ita,eng): la release viene scartata se non contiene questi sottotitoli.")><span>{ctx_tr("Requisiti sottotitoli (obbligatori)")}</span><input prop:value=subtitle_reqs on:input=move |event| subtitle_reqs.set(event_target_value(&event)) placeholder=ctx_tr("ita,eng") /></label>
                                    </div>
                                    <div class="field span-full" title=ctx_tr("Lingue che devono essere presenti nella release; con 'obbligatoria' la release senza quella lingua viene scartata")>
                                        <span>{ctx_tr("Lingue richieste (fino a 3)")}</span>
                                        <div class="language-rows-compact">
                                            {language_row(lang_a, lang_a_req)}
                                            {language_row(lang_b, lang_b_req)}
                                            {language_row(lang_c, lang_c_req)}
                                        </div>
                                        <small class="hint">{move || {
                                            let entries = vec![
                                                (lang_a.get(), lang_a_req.get()),
                                                (lang_b.get(), lang_b_req.get()),
                                                (lang_c.get(), lang_c_req.get()),
                                            ];
                                            let active: Vec<String> = entries.iter().filter(|(language, _)| !language.is_empty())
                                                 .map(|(language, required)| if *required { format!("{} ({})", language, tr(data, "obbligatoria")) } else { format!("{} ({})", language, tr(data, "opzionale")) })
                                                .collect();
                                             if active.is_empty() { tr(data, "Nessuna lingua aggiuntiva richiesta.") } else { format!("{}: {}", tr(data, "Anteprima"), active.join(", ")) }
                                        }}</small>
                                    </div>
                                </div>
                                     <div class="form-actions">
                                     <button class="btn primary">{ctx_tr("Salva")}</button>
                                    <button type="button" class="btn" on:click=move |_| {
                                        if let Some(id) = selected.get() {
                                            run_post(data, &format!("/api/movies/{id}/redownload"), None, "Film rimesso in coda");
                                        }
                                    }>{ctx_tr("Riscarica")}</button>
                                    <button type="button" class="btn" title=ctx_tr("Cerca ora le release per questo film in archivio, feed, indexer e motori web") on:click=move |_| {
                                        if let Some(id) = selected.get() {
                                            spawn_local(async move {
                                                match send("POST", &format!("/api/movies/{id}/search"), None).await {
                                                    Ok(value) => best_matches.set(array(&value, "results")),
                                                    Err(error) => data.update(|current| current.error = error),
                                                }
                                            });
                                        }
                                    }>{ctx_tr("Cerca subito")}</button>
                                </div>
                            </form>
                            <Show when=move || metadata_open.get()>
                                <div class="modal-backdrop" on:click=move |_| metadata_open.set(false)>
                                    <div class="modal" style="width:min(820px,96vw)" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                                        <div class="modal-head">
                                            <strong>{ctx_tr("Aggiorna metadati film")}</strong>
                                            <button type="button" class="btn sm" on:click=move |_| metadata_open.set(false)>{ctx_tr("Chiudi")}</button>
                                        </div>
                                        <div class="modal-body">
                                            <p class="muted">{ctx_tr("Scegli il film corretto. Verranno aggiornati solo titolo, anno, trama, locandina e ID del provider; scaricamenti e impostazioni restano invariati.")}</p>
                                            <form class="search-row" on:submit=move |event| {
                                                event.prevent_default();
                                                let Some(id) = selected.get() else { return; };
                                                let query = metadata_query.get();
                                                let source = metadata_source.get();
                                                if query.trim().is_empty() { return; }
                                                let loading = metadata_loading;
                                                let error = metadata_error;
                                                let results = metadata_results;
                                                let result_source = metadata_result_source;
                                                loading.set(true);
                                                error.set(String::new());
                                                spawn_local(async move {
                                                    let path = format!("/api/movies/{id}/metadata/search");
                                                    match send("POST", &path, Some(json!({"query":query,"source":source}))).await {
                                                        Ok(value) => {
                                                            result_source.set(text(&value, "source", "tmdb"));
                                                            results.set(array(&value, "items"));
                                                        }
                                                        Err(message) => error.set(message),
                                                    }
                                                    loading.set(false);
                                                });
                                            }>
                                                <select prop:value=metadata_source on:change=move |event| metadata_source.set(event_target_value(&event)) title=ctx_tr("Provider di metadati")>
                                                    <option value="tmdb">"TMDB"</option>
                                                    <option value="tvdb">"TVDB"</option>
                                                </select>
                                                <input prop:value=metadata_query on:input=move |event| metadata_query.set(event_target_value(&event)) placeholder=ctx_tr("Cerca titolo film…") />
                                                 <button type="submit" class="btn primary" disabled=move || metadata_loading.get()>{move || if metadata_loading.get() { tr(data, "Ricerca…") } else { tr(data, "Cerca") }}</button>
                                            </form>
                                            <Show when=move || !metadata_error.get().is_empty()>
                                                <div class="notice err" style="margin-top:10px">{move || metadata_error.get()}</div>
                                            </Show>
                                            <div class="list" style="margin-top:12px">
                                                {move || {
                                                    let source = metadata_result_source.get();
                                                    metadata_results.get().into_iter().map(|item| {
                                                        let candidate_id = item.get("id").and_then(Value::as_i64).map(|value| value.to_string()).unwrap_or_else(|| text(&item, "id", ""));
                                                        let fallback_title = text_tr(data, &item, "name", "Film");
                                                        let title = text(&item, "title", &fallback_title);
                                                        let fallback_date = text(&item, "first_air_date", "");
                                                        let date = text(&item, "release_date", &fallback_date);
                                                        let overview = text(&item, "overview", "");
                                                        let movie_id = selected.get().unwrap_or_default();
                                                        let candidate_for_disabled = candidate_id.clone();
                                                        let source_for_choice = source.clone();
                                                        view! {
                                                            <div class="list-item">
                                                                <div><strong>{title}</strong><small>{date}</small><small class="truncate">{overview}</small></div>
                                                                <button type="button" class="btn sm primary" disabled=move || candidate_for_disabled.is_empty() on:click=move |_| apply_movie_metadata_choice(movie_id, candidate_id.clone(), source_for_choice.clone(), name, year, detail, movie_tmdb_id, movie_tvdb_id, metadata_open, metadata_loading, metadata_error)>{ctx_tr("Usa questi dati")}</button>
                                                            </div>
                                                        }
                                                    }).collect_view()
                                                }}
                                            </div>
                                        </div>
                                    </div>
                                </div>
                            </Show>
                            <Show when=move || !best_matches.get().is_empty()>
                                <div class="table-wrap" style="margin-top:12px">
                                    <h4 style="margin:0 0 8px">{ctx_tr("Migliori trovati")}</h4>
                                    <div class="toolbar" style="margin-bottom:8px">
                                        <input style="flex:1" prop:value=best_filter on:input=move |event| best_filter.set(event_target_value(&event)) placeholder=ctx_tr("Filtra per titolo/anno… (-parola per escludere)") title=ctx_tr("Tutti i termini devono comparire nel titolo; quelli con - davanti devono mancare. Un anno a 4 cifre filtra anche per anno.") />
                                        <span class="muted">{move || format!("{}/{}", best_matches.get().iter().filter(|release| matches_release_filter(release, &best_filter.get())).count(), best_matches.get().len())}</span>
                                    </div>
                                    <table class="data-table">
                                        <thead><tr><th>{ctx_tr("Release")}</th><th>{ctx_tr("Fonte")}</th><th></th></tr></thead>
                                        <tbody>{move || best_matches.get().iter().filter(|release| matches_release_filter(release, &best_filter.get())).cloned().map(|release| {
                                            let queued = release.clone();
                                            view! { <tr><td class="truncate">{text(&release, "title", "Release")}</td><td class="muted">{text(&release, "source", "-")}</td><td><button class="btn sm primary" on:click=move |_| { let release = queued.clone(); run_post(data, "/api/search/add", Some(json!({"release": release})), "Release accodata"); }>{ctx_tr("Accoda")}</button></td></tr> }
                                        }).collect_view()}</tbody>
                                    </table>
                                </div>
                            </Show>
                            <div class="table-wrap" style="margin-top:12px">
                                <table class="data-table">
                                    <thead><tr><th>{ctx_tr("Release")}</th><th>{ctx_tr("Scaricato")}</th><th>{ctx_tr("Dimensione")}</th></tr></thead>
                                    <tbody>
                                        {move || array(&detail.get(), "history").into_iter().map(|item| view! {
                                            <tr><td class="truncate">{text(&item, "title", "-")}</td><td class="muted">{text(&item, "downloaded_at", "-")}</td><td class="numeric">{size(&item, "size_bytes")}</td></tr>
                                        }).collect_view()}
                                    </tbody>
                                </table>
                            </div>
                    </Panel>
                </Show>
            </Show>
            <Show when=move || tab.get() == "downloaded">
                <Panel title="Film scaricati">
                    <div class="table-wrap">
                        <table class="data-table">
                            <thead><tr><th>{ctx_tr("Film")}</th><th>{ctx_tr("Anno")}</th><th>{ctx_tr("Scaricato")}</th><th>{ctx_tr("Dimensione")}</th></tr></thead>
                            <tbody>
                                {move || data.get().movie_history.iter().cloned().map(|item| view! {
                                    <tr><td class="truncate">{text(&item, "name", "-")}</td><td class="muted">{text(&item, "year", "-")}</td><td class="muted">{text(&item, "downloaded_at", "-")}</td><td class="numeric">{size(&item, "size_bytes")}</td></tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                    <Show when=move || data.get().movie_history.is_empty()><Empty text="Nessun film scaricato." /></Show>
                </Panel>
            </Show>
        </div>
    }
}

/* ------------------------------------------------------------------ */
/* Discovery / search                                                  */
/* ------------------------------------------------------------------ */

#[component]
fn TmdbResults(
    data: RwSignal<Data>,
    items: RwSignal<Vec<Value>>,
    kind: RwSignal<String>,
    pending: RwSignal<Option<Value>>,
    page: RwSignal<String>,
) -> impl IntoView {
    view! {
        <div class="tmdb-grid">
            {move || items.get().iter().cloned().map(|item| {
                let tmdb_id = item.get("id").and_then(Value::as_i64).unwrap_or(0).to_string();
                let item_name = item.get("name").and_then(Value::as_str).or_else(|| item.get("title").and_then(Value::as_str)).unwrap_or("Titolo").to_string();
                let year = item.get("first_air_date").and_then(Value::as_str).or_else(|| item.get("release_date").and_then(Value::as_str)).unwrap_or("").chars().take(4).collect::<String>();
                let overview = item.get("overview").and_then(Value::as_str).unwrap_or("").to_string();
                let poster = item.get("poster_path").and_then(Value::as_str).map(|path| format!("https://image.tmdb.org/t/p/w154{path}"));
                let kind_value = kind.get();
                let is_movie = kind_value == "movie";
                let library_page = if is_movie { "movies" } else { "series" };
                let add_name = item_name.clone();
                let title = item_name.clone();
                let library = data.get().library.clone();
                let normalize = |value: &str| value.to_lowercase().chars().filter(|character| character.is_alphanumeric()).collect::<String>();
                let name_norm = normalize(&item_name);
                let in_list = if kind_value == "movie" {
                    array(&library, "movies").iter().any(|movie| {
                        normalize(&text(movie, "name", "")) == name_norm
                            && (year.is_empty() || text(movie, "year", "") == year)
                    })
                } else {
                    array(&library, "series").iter().any(|series| {
                        (!tmdb_id.is_empty() && text(series, "tmdb_id", "") == tmdb_id)
                            || normalize(&text(series, "name", "")) == name_norm
                    })
                };
                let tmdb_url = if kind_value == "movie" {
                    format!("https://www.themoviedb.org/movie/{tmdb_id}")
                } else {
                    format!("https://www.themoviedb.org/tv/{tmdb_id}")
                };
                view! {
                    <article class="tmdb-card" class:in-list=in_list>
                        <a class="tmdb-poster" href=tmdb_url.clone() target="_blank" rel="noopener" title=ctx_tr("Apri su TMDB")>
                            {match poster {
                                Some(url) => view! { <img src=url alt=title.clone() loading="lazy" /> }.into_any(),
                                None => view! { <span>{ctx_tr("N/D")}</span> }.into_any(),
                            }}
                        </a>
                        <div class="tmdb-body">
                            <a href=tmdb_url.clone() target="_blank" rel="noopener" title=ctx_tr("Apri su TMDB")><strong>{item_name.clone()}</strong></a>
                            <small class="muted">{format!("TMDB {tmdb_id} · {year}")}</small>
                            <p class="muted">{overview.chars().take(170).collect::<String>()}</p>
                            <Show when=move || in_list>
                                <span class="badge ok">{ctx_tr("Già in lista")}</span>
                            </Show>
                            <Show when=move || in_list>
                                 <button class="btn sm" on:click=move |_| page.set(library_page.to_string())>{if is_movie { ctx_tr("Apri in Film") } else { ctx_tr("Apri in Serie TV") }}</button>
                            </Show>
                            <button class="btn sm primary" disabled=move || in_list on:click=move |_| {
                                pending.set(Some(json!({"kind": kind_value, "name": add_name.clone(), "year": year.clone(), "tmdb_id": tmdb_id.clone()})));
                             }>{move || if in_list { tr(data, "In lista") } else { tr(data, "Aggiungi alla libreria") }}</button>
                        </div>
                    </article>
                }
            }).collect_view()}
        </div>
    }
}

fn load_discover(
    data: RwSignal<Data>,
    items: RwSignal<Vec<Value>>,
    kind: RwSignal<String>,
    window: RwSignal<String>,
    mode: &'static str,
) {
    let body = json!({"kind": kind.get(), "window": window.get(), "mode": mode});
    spawn_local(async move {
        match send("POST", "/api/tmdb/discover", Some(body)).await {
            Ok(value) => items.set(array(&value, "items")),
            Err(error) => data.update(|current| current.error = error),
        }
    });
}

#[component]
fn Discovery(data: RwSignal<Data>, page: RwSignal<String>) -> impl IntoView {
    let tmdb_query = RwSignal::new(String::new());
    let tmdb_kind = RwSignal::new("series".to_string());
    // Tab di pagina di Esplora: Calendario (default, primo) | Serie TV | Film.
    let explore_tab = RwSignal::new("calendar".to_string());
    let tmdb_results = RwSignal::new(Vec::<Value>::new());
    let trending = RwSignal::new(Vec::<Value>::new());
    let trend_kind = RwSignal::new("series".to_string());
    let trend_window = RwSignal::new("week".to_string());
    let query = RwSignal::new(String::new());
    let pending = RwSignal::new(Option::<Value>::None);
    let add_quality = RwSignal::new(String::new());
    let add_language = RwSignal::new(text(&data.get().config, "default_language", "ita"));
    let add_seasons = RwSignal::new("1+".to_string());
    let add_archive_path = RwSignal::new(String::new());
    let add_exclude = RwSignal::new(String::new());
    let add_message = RwSignal::new(String::new());
    load_discover(data, trending, trend_kind, trend_window, "trending");
    view! {
        <div class="view">
            <div class="tabs" style="margin-bottom:10px">
                <button class="tab" class:active=move || explore_tab.get() == "calendar" on:click=move |_| explore_tab.set("calendar".into())>{ctx_tr("Calendario")}</button>
                <button class="tab" class:active=move || explore_tab.get() == "series" on:click=move |_| { explore_tab.set("series".into()); tmdb_kind.set("series".into()); trend_kind.set("series".into()); load_discover(data, trending, trend_kind, trend_window, "trending"); }>{ctx_tr("Serie TV")}</button>
                <button class="tab" class:active=move || explore_tab.get() == "movie" on:click=move |_| { explore_tab.set("movie".into()); tmdb_kind.set("movie".into()); trend_kind.set("movie".into()); load_discover(data, trending, trend_kind, trend_window, "trending"); }>{ctx_tr("Film")}</button>
            </div>
            <Show when=move || explore_tab.get() == "calendar"><CalendarView data /></Show>
            <Show when=move || explore_tab.get() != "calendar">
            <Show when=move || !add_message.get().is_empty()>
                <div class="notice">{move || add_message.get()}</div>
            </Show>
            <Panel title="Di tendenza su TMDB">
                <div class="toolbar">
                    <button class="btn" title=ctx_tr("Le uscite più popolari della settimana") on:click=move |_| { trend_window.set("week".into()); load_discover(data, trending, trend_kind, trend_window, "trending"); }>{ctx_tr("Tendenza settimana")}</button>
                    <button class="btn" title=ctx_tr("Le uscite più popolari di oggi") on:click=move |_| { trend_window.set("day".into()); load_discover(data, trending, trend_kind, trend_window, "trending"); }>{ctx_tr("Tendenza oggi")}</button>
                    <button class="btn" title=ctx_tr("I titoli più popolari su TMDB") on:click=move |_| load_discover(data, trending, trend_kind, trend_window, "popular")>{ctx_tr("Popolari")}</button>
                    <button class="btn" title=ctx_tr("I titoli TMDB con la valutazione più alta") on:click=move |_| load_discover(data, trending, trend_kind, trend_window, "top_rated")>{ctx_tr("Più votati")}</button>
                    <button class="btn" title=ctx_tr("Film al cinema o serie attualmente in onda") on:click=move |_| load_discover(data, trending, trend_kind, trend_window, "now_playing")>{ctx_tr("In programmazione")}</button>
                    <button class="btn" title=ctx_tr("Prossimi film o serie in arrivo") on:click=move |_| load_discover(data, trending, trend_kind, trend_window, "upcoming")>{ctx_tr("Prossime uscite")}</button>
                </div>
                <div style="margin-top:12px"><TmdbResults data items=trending kind=trend_kind pending page /></div>
            </Panel>
            <Panel title="Esplora TMDB (ricerca)">
                <form class="form" on:submit=move |event| {
                    event.prevent_default();
                    let body = json!({"query": tmdb_query.get(), "kind": tmdb_kind.get()});
                    spawn_local(async move {
                        match send("POST", "/api/tmdb/search", Some(body)).await {
                            Ok(value) => tmdb_results.set(array(&value, "items")),
                            Err(error) => data.update(|current| current.error = error),
                        }
                    });
                }>
                    <div class="form-grid">
                        <label class="field span-2" title=ctx_tr("Titolo da cercare su TMDB")><span>{ctx_tr("Titolo da cercare su TMDB")}</span><input prop:value=tmdb_query on:input=move |event| tmdb_query.set(event_target_value(&event)) placeholder=ctx_tr("Nome serie o film") /></label>
                        <label class="field" title=ctx_tr("Tipo")><span>{ctx_tr("Tipo")}</span><select prop:value=tmdb_kind on:change=move |event| tmdb_kind.set(event_target_value(&event))>
                            <option value="series">{ctx_tr("Serie TV")}</option>
                            <option value="movie">{ctx_tr("Film")}</option>
                        </select></label>
                    </div>
                    <div class="form-actions"><button class="btn primary">{ctx_tr("Cerca su TMDB")}</button></div>
                </form>
                <div class="table-wrap" style="margin-top:12px">
                    <table class="data-table">
                        <thead><tr><th>{ctx_tr("Titolo")}</th><th>{ctx_tr("TMDB")}</th><th>{ctx_tr("Anno")}</th><th></th></tr></thead>
                        <tbody>
                            {move || tmdb_results.get().iter().cloned().map(|item| {
                                let tmdb_id = item.get("id").and_then(Value::as_i64).unwrap_or(0).to_string();
                                let item_name = item.get("name").and_then(Value::as_str).or_else(|| item.get("title").and_then(Value::as_str)).unwrap_or("Titolo").to_string();
                                let year = item.get("first_air_date").and_then(Value::as_str).or_else(|| item.get("release_date").and_then(Value::as_str)).unwrap_or("").chars().take(4).collect::<String>();
                                let kind = tmdb_kind.get();
                                let add_name = item_name.clone();
                                view! {
                                    <tr>
                                        <td>{item_name.clone()}</td>
                                        <td class="mono muted">{tmdb_id.clone()}</td>
                                        <td class="muted">{year.clone()}</td>
                                        <td><button class="btn sm primary" on:click=move |_| {
                                            pending.set(Some(json!({"kind": kind, "name": add_name.clone(), "year": year.clone(), "tmdb_id": tmdb_id.clone()})));
                                        }>{ctx_tr("Aggiungi")}</button></td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
            </Panel>
            <Panel title="Ricerca release">
                <form class="form" on:submit=move |event| {
                    event.prevent_default();
                    let value = query.get();
                    spawn_local(async move {
                        match send("POST", "/api/search", Some(json!({"query": value}))).await {
                            Ok(result) => data.update(|current| { current.search = array(&result, "results"); current.error.clear(); }),
                            Err(error) => data.update(|current| current.error = error),
                        }
                    });
                }>
                    <label class="field span-full" title=ctx_tr("Cerca release")><span>{ctx_tr("Cerca release")}</span><input prop:value=query on:input=move |event| query.set(event_target_value(&event)) placeholder=ctx_tr("Serie, film o S01E03") /></label>
                    <div class="form-actions"><button class="btn primary">{ctx_tr("Cerca")}</button></div>
                </form>
                <div class="table-wrap" style="margin-top:12px">
                    <table class="data-table">
                        <thead><tr><th>{ctx_tr("Release")}</th><th>{ctx_tr("Sorgente")}</th><th>{ctx_tr("Risoluzione")}</th><th>{ctx_tr("Codec")}</th><th></th></tr></thead>
                        <tbody>
                            {move || data.get().search.iter().cloned().map(|item| {
                                let release = item.clone();
                                let resolution = text(&item.get("quality").cloned().unwrap_or_default(), "resolution", "-");
                                let codec = text(&item.get("quality").cloned().unwrap_or_default(), "codec", "-");
                                view! {
                                    <tr>
                                        <td class="truncate">{text(&item, "title", "Release")}</td>
                                        <td class="muted">{text(&item, "source", "-")}</td>
                                        <td class="muted">{resolution}</td>
                                        <td class="muted">{codec}</td>
                                        <td><button class="btn sm primary" on:click=move |_| { let release = release.clone(); run_post(data, "/api/search/add", Some(json!({"release": release})), "Release accodata"); }>{ctx_tr("Accoda")}</button></td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
            </Panel>
            <Show when=move || pending.get().is_some()>
                <div class="modal-backdrop" on:click=move |_| pending.set(None)>
                    <div class="modal" style="width:min(560px,100%)" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                        <div class="modal-head">
                            <strong>{move || pending.get().map(|item| text_tr(data, &item, "name", "Conferma aggiunta")).unwrap_or_else(|| tr(data, "Conferma aggiunta"))}</strong>
                            <button type="button" class="btn sm" on:click=move |_| pending.set(None)>{ctx_tr("Chiudi")}</button>
                        </div>
                        <div class="modal-body">
                            <form class="form-grid" on:submit=move |event| {
                                event.prevent_default();
                                let Some(item) = pending.get() else { return };
                                let kind = text(&item, "kind", "series");
                                let name = text(&item, "name", "");
                                let year = text(&item, "year", "");
                                let tmdb_id = text(&item, "tmdb_id", "");
                                let quality = add_quality.get();
                                let language = add_language.get();
                                let seasons = add_seasons.get();
                                let archive_path = add_archive_path.get();
                                let exclude = add_exclude.get();
                                let message = add_message;
                                let name_label = name.clone();
                                spawn_local(async move {
                                    let body = json!({
                                        "kind": kind,
                                        "name": name,
                                        "year": year,
                                        "tmdb_id": tmdb_id,
                                        "quality": quality,
                                        "language": language,
                                        "seasons": seasons,
                                        "archive_path": archive_path,
                                        "exclude": exclude,
                                    });
                                    match send("POST", "/api/tmdb/add", Some(body)).await {
                                        Ok(_) => {
                                             message.set(tr_format(data, "Aggiunto: {name}", &[("{name}", name_label)]));
                                            pending.set(None);
                                            trigger_refresh();
                                        }
                                        Err(error) => message.set(error),
                                    }
                                });
                            }>
                                <label class="field span-full" title=ctx_tr("Titolo")><span>{ctx_tr("Titolo")}</span><input prop:value=move || pending.get().map(|item| text(&item, "name", "")).unwrap_or_default() readonly /></label>
                                 <label class="field" title=ctx_tr("Qualità richiesta")><span>{ctx_tr("Qualità richiesta")}</span><select prop:value=add_quality on:change=move |event| add_quality.set(event_target_value(&event))>{QUALITY_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}</select></label>
                                 <label class="field" title=ctx_tr("Lingua")><span>{ctx_tr("Lingua")}</span><select prop:value=add_language on:change=move |event| add_language.set(event_target_value(&event))>{LANGUAGE_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}</select></label>
                                <Show when=move || pending.get().map(|item| text(&item, "kind", "series") == "series").unwrap_or(true)>
                                    <label class="field" title=ctx_tr("Quali stagioni monitorare, es. 1-3,5+")><span>{ctx_tr("Stagioni")}</span><input prop:value=add_seasons on:input=move |event| add_seasons.set(event_target_value(&event)) placeholder=ctx_tr("es. 1-3,5+") /></label>
                                    <div class="span-full"><PathPicker label="Percorso di salvataggio (NAS)" value=add_archive_path placeholder="/mnt/nas/Serie TV" /></div>
                                </Show>
                                <label class="field span-full" title=ctx_tr("Parole da escludere dai titoli")><span>{ctx_tr("Parole vietate (exclude)")}</span><input prop:value=add_exclude on:input=move |event| add_exclude.set(event_target_value(&event)) placeholder=ctx_tr("cam, ts, screener") /></label>
                                <div class="form-actions">
                                    <button class="btn primary">{ctx_tr("Conferma")}</button>
                                    <button type="button" class="btn" on:click=move |_| pending.set(None)>{ctx_tr("Annulla")}</button>
                                </div>
                            </form>
                        </div>
                    </div>
                </div>
            </Show>
            </Show>
        </div>
    }
}

/* ------------------------------------------------------------------ */
/* Archive                                                             */
/* ------------------------------------------------------------------ */

#[component]
fn ArchiveView(data: RwSignal<Data>) -> impl IntoView {
    // La cronologia dei feed è consultabile solo quando l'utente la apre:
    // non carichiamo centinaia di gruppi ad ogni ingresso in Archivio.
    let tab = RwSignal::new("archive".to_string());
    let query = RwSignal::new(String::new());
    let selected = RwSignal::new(Vec::<i64>::new());
    let manual_magnet = RwSignal::new(String::new());
    view! {
        <div class="view">
            <div class="tabs">
                <button class="tab" class:active=move || tab.get() == "archive" on:click=move |_| tab.set("archive".into())>{ctx_tr("Archivio torrent")}</button>
                <button class="tab" class:active=move || tab.get() == "seen" on:click=move |_| tab.set("seen".into())>{ctx_tr("Visti dal feed")}</button>
            </div>
            <Show when=move || tab.get() == "archive">
            <Panel title="Archivio torrent">
                <form class="toolbar" on:submit=move |event| {
                    event.prevent_default();
                    selected.set(Vec::new());
                    let term = query.get();
                    spawn_local(async move {
                        match get(&format!("/api/archive?q={}&page=1&limit=100", urlencoding::encode(&term))).await {
                            Ok(value) => data.update(|current| {
                                current.archive = array(&value, "items");
                                current.archive_page = value.get("page").and_then(Value::as_u64).unwrap_or(1) as usize;
                                current.archive_pages = value.get("pages").and_then(Value::as_u64).unwrap_or(1) as usize;
                                current.archive_total = value.get("total").and_then(Value::as_i64).unwrap_or(0);
                            }),
                            Err(error) => data.update(|current| current.error = error),
                        }
                    });
                }>
                    <input prop:value=query title=ctx_tr("Cerca nell'archivio per parole nel titolo: quelle scritte devono essere presenti, quelle con - davanti devono mancare (es. -cam)") on:input=move |event| query.set(event_target_value(&event)) placeholder=ctx_tr("Cerca parole nel titolo… (-parola esclude)") />
                    <button class="btn primary">{ctx_tr("Cerca")}</button>
                    <button type="button" class="btn" on:click=move |_| {
                        let chosen = selected.get();
                        let items = data.get().archive.iter().filter(|item| chosen.contains(&item.get("id").and_then(Value::as_i64).unwrap_or_default())).map(|item| json!({"title": text(item, "title", ""), "magnet": text(item, "magnet", ""), "source": text(item, "source", "archive")})).collect::<Vec<_>>();
                        run_post(data, "/api/archive/batch-download", Some(json!({"items": items})), "Release accodate");
                    }>{ctx_tr("Accoda selezionate")}</button>
                    <button type="button" class="btn danger" title=ctx_tr("Elimina dall'archivio le release selezionate") on:click=move |_| {
                        let chosen = selected.get();
                        if chosen.is_empty() { return; }
                        run_post(data, "/api/archive/delete", Some(json!({"ids": chosen})), "Release eliminate");
                        selected.set(Vec::new());
                    }>{ctx_tr("Elimina selezionate")}</button>
                    <button type="button" class="btn" title=ctx_tr("Copia negli appunti i magnet delle release selezionate") on:click=move |_| {
                        let chosen = selected.get();
                        let magnets: Vec<String> = data.get().archive.iter().filter(|item| chosen.contains(&item.get("id").and_then(Value::as_i64).unwrap_or_default())).map(|item| text(item, "magnet", "")).filter(|magnet| !magnet.is_empty()).collect();
                        if !magnets.is_empty() { copy_to_clipboard(&magnets.join("\n")); }
                    }>{ctx_tr("Copia magnet")}</button>
                </form>
                <form class="toolbar" style="margin-top:8px" on:submit=move |event| {
                    event.prevent_default();
                    let magnet = manual_magnet.get();
                    if magnet.trim().is_empty() { return; }
                    run_post(data, "/api/send-magnet", Some(json!({"magnet": magnet})), "Magnet accodato");
                    manual_magnet.set(String::new());
                }>
                    <input style="flex:1" prop:value=manual_magnet on:input=move |event| manual_magnet.set(event_target_value(&event)) placeholder=ctx_tr("Magnet o URL .torrent da aggiungere alla sessione") title=ctx_tr("Accoda un magnet o un URL .torrent senza uscire dall'archivio") />
                    <button class="btn">{ctx_tr("Aggiungi magnet")}</button>
                </form>
                <div class="toolbar" style="margin-top:8px">
                    <span class="muted">{move || tr_format(data, "{count} elementi", &[("{count}", data.get().archive_total.to_string())])}</span>
                    <button class="btn sm" disabled=move || { data.get().archive_page <= 1 } on:click=move |_| archive_page(data, query.get(), data.get().archive_page.saturating_sub(1))>{ctx_tr("Precedente")}</button>
                    <span>{move || format!("Pagina {} / {}", data.get().archive_page, data.get().archive_pages)}</span>
                    <button class="btn sm" disabled=move || { data.get().archive_page >= data.get().archive_pages } on:click=move |_| archive_page(data, query.get(), data.get().archive_page.saturating_add(1))>{ctx_tr("Successiva")}</button>
                </div>
                <div class="table-wrap" style="margin-top:10px">
                    <table class="data-table">
                        <thead><tr><th><input type="checkbox" title=ctx_tr("Seleziona tutti") prop:checked=move || {
                            let ids: Vec<i64> = data.get().archive.iter().filter_map(|item| item.get("id").and_then(Value::as_i64)).collect();
                            !ids.is_empty() && ids.iter().all(|id| selected.get().contains(id))
                        } on:change=move |_| {
                            let ids: Vec<i64> = data.get().archive.iter().filter_map(|item| item.get("id").and_then(Value::as_i64)).collect();
                            selected.update(|current| {
                                if ids.iter().all(|id| current.contains(id)) {
                                    current.clear();
                                } else {
                                    for id in ids {
                                        if !current.contains(&id) { current.push(id); }
                                    }
                                }
                            });
                        } /></th><th>{ctx_tr("Titolo")}</th><th>{ctx_tr("Fonte")}</th><th>{ctx_tr("Score")}</th><th></th></tr></thead>
                        <tbody>
                            {move || data.get().archive.iter().cloned().map(|item| {
                                let id = item.get("id").and_then(Value::as_i64).unwrap_or_default();
                                let title = text(&item, "title", "Release");
                                let add_title = title.clone();
                                let add_magnet = text(&item, "magnet", "");
                                let add_source = text(&item, "source", "archive");
                                let tmdb_search_url = format!("https://www.themoviedb.org/search?query={}", urlencoding::encode(&title));
                                view! {
                                    <tr>
                                        <td><input type="checkbox" prop:checked=move || selected.get().contains(&id) on:change=move |_| selected.update(|items| { if items.contains(&id) { items.retain(|value| *value != id); } else { items.push(id); } }) /></td>
                                        <td class="truncate">{title}</td>
                                        <td class="muted">{text(&item, "source", "archive")}</td>
                                        <td class="numeric">{number(&item, "quality_score")}</td>
                                        <td>
                                                <div class="toolbar">
                                                    <button class="btn sm" on:click=move |_| { run_post(data, "/api/archive/add", Some(json!({"title": add_title.clone(), "magnet": add_magnet.clone(), "source": add_source.clone()})), "Release accodata"); }>{ctx_tr("Accoda")}</button>
                                                    <a class="btn sm" href=tmdb_search_url target="_blank" rel="noopener" title=ctx_tr("Cerca il titolo della release su TMDB")>{ctx_tr("TMDB")}</a>
                                                    <button class="btn sm danger" on:click=move |_| { run_post(data, "/api/archive/delete", Some(json!({"ids":[id]})), "Release eliminata"); }>{ctx_tr("Elimina")}</button>
                                            </div>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
            </Panel>
            </Show>
            <Show when=move || tab.get() == "seen">
                <FeedSeenPanel data />
            </Show>
        </div>
    }
}

/// "Visti dai feed": tutte le release che passano dalle sorgenti, raggruppate
/// per titolo (film o serie), non solo quelle in libreria. Porting di
/// `movie_feed_seen` / `series_feed_seen` del legacy.
#[component]
fn FeedSeenPanel(data: RwSignal<Data>) -> impl IntoView {
    // Nessuna richiesta finché non viene scelto esplicitamente Film o Serie.
    let kind = RwSignal::new(String::new());
    let groups = RwSignal::new(Vec::<Value>::new());
    let total = RwSignal::new(0_i64);
    let expanded = RwSignal::new(String::new());
    let entries = RwSignal::new(Vec::<Value>::new());
    let entries_loading = RwSignal::new(false);
    let query = RwSignal::new(String::new());
    // Filtro unificato: mostra solo i gruppi che corrispondono a un titolo
    // monitorato (eredita la funzione del vecchio pannello "Dal feed").
    let monitored_only = RwSignal::new(false);
    view! {
        <Panel title="Visti dal feed">
            <div class="toolbar">
                <button class="btn" class:primary=move || kind.get()=="movie" on:click=move |_| {
                    kind.set("movie".into());
                    expanded.set(String::new());
                    entries.set(Vec::new());
                    entries_loading.set(false);
                    load_seen_groups("movie".into(), query.get(), groups, total, data);
                }>{ctx_tr("Film visti")}</button>
                <button class="btn" class:primary=move || kind.get()=="series" on:click=move |_| {
                    kind.set("series".into());
                    expanded.set(String::new());
                    entries.set(Vec::new());
                    entries_loading.set(false);
                    load_seen_groups("series".into(), query.get(), groups, total, data);
                }>{ctx_tr("Serie viste")}</button>
                <Show when=move || !kind.get().is_empty()>
                <form on:submit=move |event| {
                    event.prevent_default();
                    expanded.set(String::new());
                    entries.set(Vec::new());
                    load_seen_groups(kind.get(), query.get(), groups, total, data);
                }>
                    <input prop:value=query on:input=move |event| query.set(event_target_value(&event)) placeholder=ctx_tr("Cerca per titolo… (* e ? come wildcard)") title=ctx_tr("Filtra i gruppi visti per parole nel titolo") />
                </form>
                    <button class="btn sm" on:click=move |_| load_seen_groups(kind.get(), query.get(), groups, total, data)>{ctx_tr("Aggiorna")}</button>
                    <label class="check" title=ctx_tr("Mostra solo i titoli che stai monitorando (serie/film in libreria)")><input type="checkbox" prop:checked=move || monitored_only.get() on:change=move |event| monitored_only.set(event_target_checked(&event)) /> <span>{ctx_tr("Solo monitorati")}</span></label>
                    <span class="muted">{move || format!("{} gruppi", total.get())}</span>
                </Show>
            </div>
            <Show when=move || kind.get().is_empty()>
                <Empty text="Scegli Film visti o Serie viste per consultare lo storico dei feed." />
            </Show>
            <Show when=move || !kind.get().is_empty() && groups.get().is_empty()>
                <Empty text="Nessuna release registrata dai feed: verranno raccolte al prossimo ciclo." />
            </Show>
            <Show when=move || !kind.get().is_empty()>
            <div class="table-wrap" style="margin-top:10px">
                <table class="data-table">
                    <thead><tr><th>{ctx_tr("Titolo")}</th><th>{ctx_tr("Anno/Stagione")}</th><th>{ctx_tr("N.")}</th><th>{ctx_tr("Migliore")}</th><th>{ctx_tr("Score")}</th><th>{ctx_tr("Ultimo")}</th><th></th></tr></thead>
                    <tbody>{move || {
                        let monitored: Vec<String> = array(&data.get().library, "series")
                            .into_iter()
                            .chain(array(&data.get().library, "movies"))
                            .map(|item| text(&item, "name", "").to_lowercase())
                            .filter(|name| !name.is_empty())
                            .collect();
                        let only = monitored_only.get();
                        groups.get().into_iter().filter(|group| {
                            if !only { return true; }
                            let group_name = text(group, "group_name", "").to_lowercase();
                            !group_name.is_empty()
                                && monitored.iter().any(|name| {
                                    name == &group_name
                                        || group_name.contains(name.as_str())
                                        || name.contains(&group_name)
                                })
                        }).map(|group| {
                        let key = text(&group, "group_key", "");
                        let name = text(&group, "group_name", "N/D");
                        let year = number(&group, "year");
                        let season = number(&group, "season");
                        let count = number(&group, "count");
                        let best = text(&group, "best_resolution", "unknown");
                        let score = number(&group, "best_score");
                        let latest = text(&group, "latest_found", "");
                        let is_series = kind.get() == "series";
                        let kind_for_click = if is_series { "series" } else { "movie" }.to_string();
                        let key_for_click = key.clone();
                        view! {
                            <tr>
                                <td class="truncate">{name}</td>
                                <td class="numeric">{if is_series { format!("S{season}") } else { year }}</td>
                                <td class="numeric">{count}</td>
                                <td>{best}</td>
                                <td class="numeric">{score}</td>
                                <td class="muted truncate">{latest}</td>
                                <td><button class="btn sm" on:click=move |_| {
                                    expanded.set(key_for_click.clone());
                                    entries.set(Vec::new());
                                    entries_loading.set(true);
                                    load_seen_entries(kind_for_click.clone(), key_for_click.clone(), entries, entries_loading, data);
                                }>{ctx_tr("Mostra")}</button></td>
                            </tr>
                        }
                    }).collect_view()}}</tbody>
                </table>
            </div>
            <Show when=move || !expanded.get().is_empty()>
                <Show when=move || entries_loading.get()>
                    <div class="search-status" style="margin-top:10px"><span class="spinner"></span>{ctx_tr("Caricamento release dai feed…")}</div>
                </Show>
                <Show when=move || !entries_loading.get() && entries.get().is_empty()>
                    <Empty text="Nessuna release trovata per questo gruppo." />
                </Show>
                <div class="table-wrap" style="margin-top:10px">
                    <table class="data-table">
                        <thead><tr><th>{ctx_tr("Release")}</th><th>{ctx_tr("Fonte")}</th><th>{ctx_tr("Qualità")}</th><th>{ctx_tr("Score")}</th><th>{ctx_tr("Vista")}</th><th></th></tr></thead>
                        <tbody>{move || entries.get().into_iter().map(|item| {
                            let title = text(&item, "title", "Release");
                            let magnet = text(&item, "magnet", "");
                            let source = text(&item, "source", "feed");
                            let quality = format!("{} {}", text(&item, "resolution", ""), text(&item, "codec", "")).trim().to_string();
                            let score = number(&item, "quality_score");
                            let found = text(&item, "found_at", "");
                            let add_title = title.clone();
                            let add_magnet = magnet.clone();
                            let add_source = source.clone();
                            view! {
                                <tr>
                                    <td class="truncate">{title}</td>
                                    <td class="muted">{source}</td>
                                    <td>{quality}</td>
                                    <td class="numeric">{score}</td>
                                    <td class="muted truncate">{found}</td>
                                    <td><button class="btn sm primary" on:click=move |_| run_post(data, "/api/archive/add", Some(json!({"title": add_title.clone(), "magnet": add_magnet.clone(), "source": add_source.clone()})), "Release accodata")>{ctx_tr("Accoda")}</button></td>
                                </tr>
                            }
                        }).collect_view()}</tbody>
                    </table>
                </div>
            </Show>
            </Show>
        </Panel>
    }
}

fn load_seen_groups(
    kind: String,
    query: String,
    groups: RwSignal<Vec<Value>>,
    total: RwSignal<i64>,
    data: RwSignal<Data>,
) {
    spawn_local(async move {
        let resource = if kind == "series" { "series" } else { "movies" };
        match get(&format!(
            "/api/{resource}/seen/grouped?limit=100&q={}",
            urlencoding::encode(&query)
        ))
        .await
        {
            Ok(value) => {
                groups.set(array(&value, "groups"));
                total.set(value.get("total").and_then(Value::as_i64).unwrap_or(0));
            }
            Err(error) => data.update(|current| current.error = error),
        }
    });
}

fn load_seen_entries(
    kind: String,
    key: String,
    entries: RwSignal<Vec<Value>>,
    loading: RwSignal<bool>,
    data: RwSignal<Data>,
) {
    spawn_local(async move {
        let resource = if kind == "series" { "series" } else { "movies" };
        match get(&format!("/api/{resource}/seen?key={}", urlencoding::encode(&key))).await {
            Ok(value) => entries.set(array(&value, "items")),
            Err(error) => data.update(|current| current.error = error),
        }
        loading.set(false);
    });
}

fn archive_page(data: RwSignal<Data>, query: String, page: usize) {
    spawn_local(async move {
        match get(&format!("/api/archive?q={}&page={page}&limit=100", urlencoding::encode(&query))).await {
            Ok(value) => data.update(|current| {
                current.archive = array(&value, "items");
                current.archive_page = value.get("page").and_then(Value::as_u64).unwrap_or(1) as usize;
                current.archive_pages = value.get("pages").and_then(Value::as_u64).unwrap_or(1) as usize;
                current.archive_total = value.get("total").and_then(Value::as_i64).unwrap_or(0);
            }),
            Err(error) => data.update(|current| current.error = error),
        }
    });
}

/// Cambia pagina nello Storico download (10 elementi per pagina).
fn history_goto(data: RwSignal<Data>, page: usize) {
    let page = page.max(1);
    let query = data.get_untracked().history_filter;
    spawn_local(async move {
        match get(&format!("/api/torrents/history?page={page}&limit=10&q={}", urlencoding::encode(&query))).await {
            Ok(value) => data.update(|current| {
                current.history = array(&value, "items");
                current.history_page = value.get("page").and_then(Value::as_u64).unwrap_or(1) as usize;
                current.history_pages = value.get("pages").and_then(Value::as_u64).unwrap_or(1) as usize;
                current.history_total = value.get("total").and_then(Value::as_i64).unwrap_or(0);
            }),
            Err(error) => data.update(|current| current.error = error),
        }
    });
}

/// Applica una scelta della modale metadata senza passare dal form dei filtri:
/// il backend aggiorna esclusivamente i campi editoriali e conserva download,
/// qualità, lingue e stato del film.
fn apply_movie_metadata_choice(
    movie_id: i64,
    external_id: String,
    source: String,
    name: RwSignal<String>,
    year: RwSignal<String>,
    detail: RwSignal<Value>,
    tmdb_id: RwSignal<String>,
    tvdb_id: RwSignal<String>,
    metadata_open: RwSignal<bool>,
    metadata_loading: RwSignal<bool>,
    metadata_error: RwSignal<String>,
) {
    metadata_loading.set(true);
    metadata_error.set(String::new());
    spawn_local(async move {
        let path = format!("/api/movies/{movie_id}/metadata");
        match send("POST", &path, Some(json!({"id":external_id,"source":source}))).await {
            Ok(value) => {
                let movie = value.get("movie").cloned().unwrap_or_default();
                name.set(text(&movie, "name", ""));
                year.set(text(&movie, "year", ""));
                tmdb_id.set(text(&movie, "tmdb_id", ""));
                tvdb_id.set(text(&movie, "tvdb_id", ""));
                metadata_open.set(false);
                trigger_refresh();
                if let Ok(value) = get(&format!("/api/movies/{movie_id}")).await {
                    detail.set(value);
                }
            }
            Err(message) => metadata_error.set(message),
        }
        metadata_loading.set(false);
    });
}

/// Filtro "regex facilitato" per le release: i termini separati da spazi devono
/// comparire nel titolo, quelli prefissati da `-` devono essere assenti. Un anno
/// a 4 cifre confronta anche il campo `year` della release.
fn matches_release_filter(release: &Value, filter: &str) -> bool {
    let title = text(release, "title", "").to_lowercase();
    let year = release.get("year").and_then(Value::as_i64);
    for raw in filter.split_whitespace() {
        if raw.is_empty() {
            continue;
        }
        if let Some(negated) = raw.strip_prefix('-') {
            if !negated.is_empty() && title.contains(&negated.to_lowercase()) {
                return false;
            }
            continue;
        }
        let wanted = raw.to_lowercase();
        let is_year = raw.len() == 4 && raw.chars().all(|character| character.is_ascii_digit());
        let hit = title.contains(&wanted) || (is_year && year == raw.parse::<i64>().ok());
        if !hit {
            return false;
        }
    }
    true
}

/// Cartella finale da mostrare nella colonna "Cartella libreria / NAS": per un
/// file multimediale è la cartella che lo contiene, altrimenti l'ultimo segmento.
fn archive_folder_label(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return String::new();
    }
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or("");
    let lower = base.to_ascii_lowercase();
    let is_media = [".mkv", ".mp4", ".avi", ".m4v", ".ts", ".mov", ".wmv", ".flv"]
        .iter()
        .any(|extension| lower.ends_with(extension));
    if is_media {
        let parent = trimmed
            .rsplit_once(['/', '\\'])
            .map(|(directory, _)| directory)
            .unwrap_or("");
        parent
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(parent)
            .to_string()
    } else {
        base.to_string()
    }
}

/* ------------------------------------------------------------------ */
/* Comics                                                              */
/* ------------------------------------------------------------------ */

fn download_comic_post(
    data: RwSignal<Data>,
    post_url: String,
    title: String,
    save_path: String,
) {
    spawn_local(async move {
        let links = match send(
            "POST",
            "/api/comics/links",
            Some(json!({"url": post_url.clone()})),
        )
        .await
        {
            Ok(value) => value
                .get("links")
                .cloned()
                .unwrap_or_else(|| value.clone()),
            Err(error) => {
                flash(data, Err(error), "Download avviato");
                return;
            }
        };
        let url = links
            .get("download_now")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find_map(Value::as_str))
            .or_else(|| {
                links
                    .get("direct")
                    .and_then(Value::as_array)
                    .and_then(|items| items.iter().find_map(Value::as_str))
            })
            .map(str::to_owned);
        let Some(url) = url else {
            flash_text(data, "err", "Download Now non trovato nella pagina GetComics.".into());
            return;
        };
        let result = send(
            "POST",
            "/api/comics/download",
            Some(json!({
                "url": url,
                "method": "direct",
                "title": title,
                "post_url": post_url,
                "save_path": save_path,
            })),
        )
        .await;
        flash(data, result, "Download avviato");
        trigger_refresh();
    });
}

#[component]
fn ComicsView(data: RwSignal<Data>) -> impl IntoView {
    let save_path = RwSignal::new(String::new());
    let post_url = RwSignal::new(String::new());
    let weekly_date = RwSignal::new(String::new());
    let weekly_from_date = RwSignal::new(data.get().comics_weekly_from_date.clone());
    let links = RwSignal::new(Vec::<Value>::new());
    let explore_query = RwSignal::new(String::new());
    let explore_items = RwSignal::new(Vec::<Value>::new());
    let selected_comic = RwSignal::new(Value::Null);
    let edit_comic = RwSignal::new(Value::Null);
    let edit_from_date = RwSignal::new(String::new());
    let edit_save_path = RwSignal::new(String::new());
    view! {
        <div class="view">
            <Panel title="Aggiungi fumetto">
                <form class="toolbar comics-search-form" on:submit=move |event| {
                    event.prevent_default();
                    let query = explore_query.get();
                    selected_comic.set(Value::Null);
                    spawn_local(async move {
                        match send("POST", "/api/comics/explore", Some(json!({"query": query}))).await {
                            Ok(value) => {
                                explore_items.set(array(&value, "items"));
                            }
                            Err(error) => data.update(|current| current.error = error),
                        }
                    });
                }>
                    <label class="field" style="flex:1" title=ctx_tr("Scrivi il nome del fumetto, poi scegli il risultato esatto trovato su GetComics")><span>{ctx_tr("Titolo")}</span><input prop:value=explore_query on:input=move |event| explore_query.set(event_target_value(&event)) placeholder=ctx_tr("Nome fumetto, es. Poison Ivy #41") /></label>
                    <button class="btn primary">{ctx_tr("Trova")}</button>
                </form>
                <p class="hint">{ctx_tr("Scrivi il titolo, premi Trova e seleziona esattamente il fumetto desiderato. Rextto salverà automaticamente il post GetComics e i suoi metadati.")}</p>
                <div class="table-wrap" style="margin-top:10px">
                    <table class="data-table">
                        <thead><tr><th>{ctx_tr("Risultato GetComics")}</th><th>{ctx_tr("Data")}</th><th></th></tr></thead>
                         <tbody>{move || explore_items.get().iter().cloned().map(|item| {
                                let item_title = text_tr(data, &item, "title", "Fumetto");
                             let item_url = text(&item, "url", "");
                              let item_date = text(&item, "date", "");
                              let item_for_select = item.clone();
                              let download_url = item_url.clone();
                              let download_title = item_title.clone();
                              view! { <tr>
                                  <td class="truncate"><a href=item_url target="_blank" rel="noopener">{item_title.clone()}</a></td>
                                  <td class="muted">{if item_date.is_empty() { "-".to_string() } else { item_date }}</td>
                                   <td><div class="toolbar">
                                       <button class="btn sm" on:click=move |_| {
                                           download_comic_post(data, download_url.clone(), download_title.clone(), save_path.get());
                                       }>{ctx_tr("Scarica")}</button>
                                       <button class="btn sm primary" on:click=move |_| {
                                           selected_comic.set(item_for_select.clone());
                                       } >{ctx_tr("Seleziona")}</button>
                                   </div></td>
                               </tr> }
                         }).collect_view()}</tbody>
                    </table>
                </div>
                <Show when=move || selected_comic.get().is_object()>
                    <div class="mode-banner active" style="margin-top:12px">
                        <strong>{ctx_tr("Fumetto selezionato")}</strong>
                        <span>{move || text(&selected_comic.get(), "title", "")}</span>
                    </div>
                    <div class="form-grid" style="margin-top:10px">
                        <PathPicker label="Percorso archivio" value=save_path placeholder="/mnt/nas/Comics" />
                    </div>
                    <div class="form-actions">
                        <button class="btn primary" on:click=move |_| {
                            let comic = selected_comic.get();
                            let mut tag_url = text(&comic, "tag_url", "");
                            let post_url = text(&comic, "url", "");
                            if tag_url.is_empty() { tag_url = post_url.clone(); }
                            let body = json!({
                                    "title": text_tr(data, &comic, "title", "Fumetto"),
                                "tag_url": tag_url,
                                "post_url": post_url,
                                "cover_url": text(&comic, "cover_url", ""),
                                "publisher": text(&comic, "publisher", ""),
                                "description": text(&comic, "description", ""),
                                "from_date": text(&comic, "date", ""),
                                "save_path": save_path.get(),
                            });
                            run_post(data, "/api/comics", Some(body), "Fumetto aggiunto");
                            selected_comic.set(Value::Null);
                        }>{ctx_tr("Aggiungi fumetto selezionato")}</button>
                        <button type="button" class="btn" on:click=move |_| selected_comic.set(Value::Null)>{ctx_tr("Annulla selezione")}</button>
                        <button type="button" class="btn" on:click=move |_| { run_post(data, "/api/comics/cycle", None, "Ciclo fumetti avviato"); }>{ctx_tr("Esegui ciclo")}</button>
                    </div>
                </Show>
            </Panel>
            <Panel title="Monitorati">
                <div class="table-wrap">
                    <table class="data-table">
                        <thead><tr><th>{ctx_tr("Titolo")}</th><th>{ctx_tr("Tag")}</th><th>{ctx_tr("Stato")}</th><th>{ctx_tr("Ultimo controllo")}</th><th></th></tr></thead>
                        <tbody>
                            {move || data.get().comics.iter().cloned().map(|item| {
                                let id = item.get("id").and_then(Value::as_i64).unwrap_or(0);
                                 let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                                 let edit_item = item.clone();
                                 let source_url = {
                                     let post = text(&item, "post_url", "");
                                     if post.is_empty() { text(&item, "tag_url", "") } else { post }
                                 };
                                 view! {
                                     <tr>
                                         <td title=text(&item, "title", "Comics")>{
                                             let latest = text(&item, "latest_downloaded_title", "");
                                             if latest.is_empty() { text(&item, "title", "Comics") } else { latest }
                                         }</td>
                                         <td class="truncate muted" title=source_url.clone()>
                                             <a href=source_url.clone() target="_blank" rel="noopener" style="color:inherit;text-decoration:underline dotted">{friendly_slug(&source_url)}</a>
                                         </td>
                                         <td><span class="badge" class:ok=enabled>{if enabled { tr(data, "attivo") } else { tr(data, "pausa") }}</span></td>
                                        <td class="muted">{text(&item, "last_checked", "-")}</td>
                                        <td>
                                            <div class="toolbar">
                                                <button class="btn sm" on:click=move |_| {
                                                    edit_from_date.set(text(&edit_item, "from_date", ""));
                                                    edit_save_path.set(text(&edit_item, "save_path", ""));
                                                    edit_comic.set(edit_item.clone());
                                                } >{ctx_tr("Modifica")}</button>
                                                <button class="btn sm" on:click=move |_| run_post(data, &format!("/api/comics/{id}/enabled"), Some(json!({"enabled": !enabled})), "Stato aggiornato")>{if enabled { ctx_tr("Pausa") } else { ctx_tr("Attiva") }}</button>
                                                 <button class="btn sm danger" on:click=move |_| remove_monitored_comic(data, id)>{ctx_tr("Elimina")}</button>
                                            </div>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
            </Panel>
            <Show when=move || edit_comic.get().is_object()>
                <div class="modal-backdrop" on:click=move |_| edit_comic.set(Value::Null)>
                    <div class="modal" on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()>
                        <div class="modal-head"><strong>{ctx_tr("Modifica fumetto monitorato")}</strong><button class="btn sm" on:click=move |_| edit_comic.set(Value::Null)>{ctx_tr("Chiudi")}</button></div>
                        <div class="modal-body">
                            <form class="form" on:submit=move |event| {
                                event.prevent_default();
                                let comic = edit_comic.get();
                                run_post(data, "/api/comics", Some(json!({
                                    "title": text_tr(data, &comic, "title", "Fumetto"),
                                    "tag_url": text(&comic, "tag_url", ""),
                                    "from_date": edit_from_date.get(),
                                    "save_path": edit_save_path.get(),
                                })), "Fumetto aggiornato");
                                edit_comic.set(Value::Null);
                            }>
                                <label class="field" title=ctx_tr("Titolo del fumetto monitorato")><span>{ctx_tr("Titolo")}</span><input prop:value=move || text(&edit_comic.get(), "title", "") readonly /></label>
                                <label class="field" title=ctx_tr("Data di inizio del monitoraggio (YYYY-MM-DD)")><span>{ctx_tr("Data inizio (YYYY-MM-DD)")}</span><input prop:value=edit_from_date on:input=move |event| edit_from_date.set(event_target_value(&event)) placeholder=ctx_tr("YYYY-MM-DD") /></label>
                                <div class="span-full"><PathPicker label="Cartella destinazione" value=edit_save_path placeholder="/mnt/nas/Comics" /></div>
                                <div class="form-actions"><button class="btn primary">{ctx_tr("Salva")}</button><button type="button" class="btn" on:click=move |_| edit_comic.set(Value::Null)>{ctx_tr("Annulla")}</button></div>
                            </form>
                        </div>
                    </div>
                </div>
            </Show>
            <div class="grid-2">
                <Panel title="Estrai link da un post">
                    <form class="form" on:submit=move |event| {
                        event.prevent_default();
                        let url = post_url.get();
                        spawn_local(async move {
                            match send("POST", "/api/comics/links", Some(json!({"url": url}))).await {
                                Ok(value) => links.set(vec![value]),
                                Err(error) => data.update(|current| current.error = error),
                            }
                        });
                    }>
                        <label class="field span-full" title=ctx_tr("URL post GetComics")><span>{ctx_tr("URL post GetComics")}</span><input prop:value=post_url on:input=move |event| post_url.set(event_target_value(&event)) placeholder=ctx_tr("https://getcomics.org/…") /></label>
                        <div class="form-actions"><button class="btn">{ctx_tr("Estrai link")}</button></div>
                    </form>
                </Panel>
                <Panel title="Weekly pack — monitoraggio">
                    <div class="mode-banner" class:active=move || data.get().comics_weekly_enabled>
                        <strong>{move || if data.get().comics_weekly_enabled { tr(data, "Weekly pack monitorato") } else { tr(data, "Weekly pack NON monitorato") }}</strong>
                        <span>{move || if data.get().comics_weekly_enabled { tr(data, "Rextto scarica automaticamente il weekly pack appena esce.") } else { tr(data, "Clicca «Monitora weekly pack» per farlo scaricare automaticamente ogni settimana.") }}</span>
                    </div>
                    <label class="field" style="margin-top:10px" title=ctx_tr("Non scaricare weekly pack con data precedente a quella indicata")>
                        <span>{ctx_tr("Scarica weekly pack a partire dal")}</span>
                        <input type="date" prop:value=weekly_from_date on:input=move |event| weekly_from_date.set(event_target_value(&event)) />
                    </label>
                    <div class="form-actions" style="margin-top:10px">
                        <button class="btn" title=ctx_tr("Salva la data di inizio dei weekly pack") on:click=move |_| {
                            run_post(data, "/api/comics/weekly/settings", Some(json!({"enabled": data.get().comics_weekly_enabled, "from_date": weekly_from_date.get()})), "Data weekly pack salvata");
                        }>{ctx_tr("Salva data")}</button>
                        <button class="btn primary" title=ctx_tr("Attiva o disattiva il download automatico del weekly pack") on:click=move |_| {
                            let enabled = !data.get().comics_weekly_enabled;
                            run_post(data, "/api/comics/weekly/settings", Some(json!({"enabled": enabled, "from_date": weekly_from_date.get()})), if enabled { "Weekly pack ora monitorato" } else { "Weekly pack non monitorato" });
                        }>{move || if data.get().comics_weekly_enabled { tr(data, "Disattiva monitoraggio") } else { tr(data, "Monitora weekly pack") }}</button>
                    </div>
                    <p class="hint">{ctx_tr("Il monitoraggio è globale: quando attivo, il weekly pack viene cercato e scaricato automaticamente. Per scaricare un solo pack specifico usa la ricerca qui sotto.")}</p>
                    <form class="form" on:submit=move |event| {
                        event.prevent_default();
                        let date = weekly_date.get();
                        spawn_local(async move {
                            match send("POST", "/api/comics/weekly/links", Some(json!({"date": date}))).await {
                                Ok(value) => links.set(vec![value]),
                                Err(error) => data.update(|current| current.error = error),
                            }
                        });
                    }>
                        <label class="field span-full" title=ctx_tr("Cerca i link di un weekly pack di una data specifica")><span>{ctx_tr("Cerca un weekly pack specifico (data YYYY-MM-DD)")}</span><input prop:value=weekly_date on:input=move |event| weekly_date.set(event_target_value(&event)) placeholder=ctx_tr("YYYY-MM-DD") /></label>
                        <div class="form-actions"><button class="btn">{ctx_tr("Cerca weekly")}</button></div>
                    </form>
                </Panel>
            </div>
            <Show when=move || !links.get().is_empty()>
                <Panel title="Link trovati">
                    <div class="stack">
                        {move || links.get().iter().cloned().map(|value| {
                            let source = value.get("links").cloned().unwrap_or_else(|| value.clone());
                            let download_now_urls = source
                                .get("download_now")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_default();
                            let groups = [("download_now", "Download Now"), ("direct", "HTTP"), ("mega", "Mega"), ("torrents", "Torrent"), ("magnets", "Magnet")];
                            view! {
                                <div class="stack">
                                    {groups.into_iter().map(|(key, label)| {
                                        let urls = source.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
                                        let urls = if key == "direct" {
                                            urls.into_iter()
                                                .filter(|url| !download_now_urls.iter().any(|download| download == url))
                                                .collect::<Vec<_>>()
                                        } else {
                                            urls
                                        };
                                        view! {
                                            <div>
                                                <strong>{label}</strong>
                                                {urls.into_iter().enumerate().map(|(index, url)| {
                                                        let method = match key {
                                                            "download_now" | "direct" => "direct",
                                                            "torrents" => "torrent",
                                                            "magnets" => "magnet",
                                                            "mega" => "mega",
                                                            _ => "direct",
                                                        };
                                                    let url_value = url.as_str().unwrap_or_default().to_string();
                                                    let title_value = format!("Comic {label} {index}");
                                                    view! {
                                                        <div class="list-item">
                                                            <small class="mono truncate">{url_value.clone()}</small>
                                                            <button class="btn sm primary" on:click=move |_| run_post(data, "/api/comics/download", Some(json!({"url": url_value.clone(), "method": method, "title": title_value.clone(), "post_url": "", "save_path": ""})), "Download avviato")>{ctx_tr("Scarica")}</button>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }
                                    }).collect_view()}
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </Panel>
            </Show>
            <Panel title="Download e storico">
                <div class="table-wrap">
                    <table class="data-table">
                        <thead><tr><th>{ctx_tr("Titolo")}</th><th>{ctx_tr("Metodo")}</th><th>{ctx_tr("Stato")}</th><th>{ctx_tr("Progresso")}</th></tr></thead>
                        <tbody>
                            {move || data.get().comic_downloads.iter().cloned().map(|item| view! {
                                <tr><td class="truncate">{text(&item, "title", "-")}</td><td class="muted">{text(&item, "method", "-")}</td><td><span class="badge">{text(&item, "status", "-")}</span></td><td class="numeric">{format!("{:.0}%", item.get("progress").and_then(Value::as_f64).unwrap_or(0.0))}</td></tr>
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
                <div class="table-wrap" style="margin-top:10px">
                    <table class="data-table">
                        <thead><tr><th>{ctx_tr("Storico")}</th><th>{ctx_tr("Stato")}</th><th></th></tr></thead>
                        <tbody>
                            {move || data.get().comics_history.iter().cloned().map(|item| {
                                let title = text_tr(data, &item, "title", "Fumetto");
                                let post_url = text(&item, "post_url", "");
                                let magnet = text(&item, "magnet", "");
                                let torrent_url = text(&item, "torrent_url", "");
                                let (url, method) = if !magnet.is_empty() { (magnet, "magnet") } else { (torrent_url, "torrent") };
                                let enabled = !url.is_empty();
                                view! { <tr>
                                    <td class="truncate">{title.clone()}</td><td class="muted">{text(&item, "sent_at", "-")}</td>
                                    <td><div class="toolbar">
                                        {let resend_post_url = post_url.clone(); view! { <button class="btn sm" disabled=!enabled on:click=move |_| run_post(data, "/api/comics/download", Some(json!({"url":url.clone(),"method":method,"title":title.clone(),"post_url":resend_post_url.clone(),"save_path":""})), "Download reinviato")>{ctx_tr("Reinvia")}</button> }}
                                        <button class="btn sm danger" on:click=move |_| run_post(data, "/api/comics/history/delete", Some(json!({"url":post_url.clone()})), "Voce storico eliminata")>{ctx_tr("Elimina")}</button>
                                    </div></td>
                                </tr> }
                            }).collect_view()}
                            {move || data.get().comics_weekly.iter().cloned().map(|item| {
                                let date = text(&item, "pack_date", "-");
                                let magnet = text(&item, "magnet", "");
                                let torrent_url = text(&item, "torrent_url", "");
                                let (url, method) = if !magnet.is_empty() { (magnet, "magnet") } else { (torrent_url, "torrent") };
                                let enabled = !url.is_empty();
                                let title = format!("Weekly {date}");
                                 let sent = !text(&item, "sent_at", "").is_empty();
                                  let status = if sent { tr(data, "inviato") } else if enabled { tr(data, "trovato") } else { tr(data, "in attesa del link") };
                                view! { <tr>
                                     <td class="truncate">{title.clone()}</td><td><span class="badge" class:ok=sent>{status}</span></td>
                                    <td><button class="btn sm" disabled=!enabled title=ctx_tr("Scarica di nuovo questo weekly pack") on:click=move |_| run_post(data, "/api/comics/download", Some(json!({"url":url.clone(),"method":method,"title":title.clone(),"post_url":"","save_path":""})), "Weekly pack forzato")>{ctx_tr("Forza")}</button></td>
                                </tr> }
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
            </Panel>
        </div>
    }
}

/* ------------------------------------------------------------------ */
/* Settings                                                            */
/* ------------------------------------------------------------------ */

const QUALITY_OPTIONS: &[(&str, &str)] = &[
    ("", "Qualsiasi"),
    ("720p", "720p"),
    ("720p+", "720p+"),
    ("1080p", "1080p"),
    ("1080p+", "1080p+"),
    ("2160p", "4K (2160p)"),
    ("2160p+", "4K+ (2160p+)"),
];
const LANGUAGE_OPTIONS: &[(&str, &str)] = &[
    ("ita", "Italiano"),
    ("eng", "Inglese"),
    ("multi", "Multi lingua"),
    ("any", "Qualsiasi"),
];
const SERIES_LANGUAGE_OPTIONS: &[(&str, &str)] = &[
    ("", "Seleziona una lingua"),
    ("ita", "Italiano"),
    ("eng", "Inglese"),
    ("ita,eng", "Italiano + Inglese"),
    ("multi", "Multi lingua"),
    ("any", "Qualsiasi"),
];
const BOOL_OPTIONS: &[(&str, &str)] = &[("true", "Sì"), ("false", "No")];
const RENAME_FORMAT_OPTIONS: &[(&str, &str)] = &[
    ("base", "Base"),
    ("standard", "Standard"),
    ("full", "Completo"),
    ("custom", "Custom"),
];
const CLEANUP_ACTION_OPTIONS: &[(&str, &str)] = &[
    ("move", "Sposta nel trash"),
    ("delete", "Elimina"),
];
const ENCRYPTION_OPTIONS: &[(&str, &str)] = &[
    ("0", "Disabilitata"),
    ("1", "Abilitata"),
    ("2", "Forzata"),
];
const ENGINE_OPTIONS: &[(&str, &str)] = &[
    ("bitsearch", "BitSearch"),
    ("tpb", "The Pirate Bay"),
    ("1337x", "1337x"),
    ("bt4g", "BT4G"),
    ("knaben", "Knaben"),
    ("nyaa", "Nyaa"),
    ("eztv", "EZTV"),
    ("btdig", "BTDig"),
    ("limetorrents", "LimeTorrents"),
    ("torrentz2", "Torrentz2"),
    ("torrentscsv", "TorrentsCSV"),
];
const RENAME_TOKENS: &[(&str, &str)] = &[
    ("{Serie}", "Serie"),
    ("{Stagione}", "Stagione"),
    ("{Episodio}", "Episodio"),
    ("{Titolo}", "Titolo"),
    ("{Risoluzione}", "Risoluzione"),
    ("{VideoCodec}", "Video codec"),
    ("{Audio}", "Audio"),
    ("{AudioCodec}", "Audio codec"),
    ("{Canali}", "Canali"),
    ("{HDR}", "HDR"),
    ("{Lingue}", "Lingue"),
];

#[component]
fn RenameEditor(data: RwSignal<Data>) -> impl IntoView {
    let format = RwSignal::new("base".to_string());
    let template = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    Effect::new(move |_| {
        format.set(raw(&data.get().config, "rename_format", "base"));
        template.set(raw(&data.get().config, "rename_template", ""));
    });
    let preview = Signal::derive(move || {
        let series = "Nome Serie";
        let title = "Titolo Episodio";
        let resolution = "1080p";
        let codec = "x265";
        let audio = "DDP5.1";
        let hdr = "HDR10";
        let language = "ita";
        let channels = "5.1";
        match format.get().as_str() {
            "standard" => format!("{series} - S01E02 - {title} [{resolution}][{codec}]"),
            "full" | "completo" => {
                format!("{series} - S01E02 - {title} [{resolution}][{audio}][{hdr}][{codec}][{language}]")
            }
            "custom" => template
                .get()
                .replace("{Serie}", series)
                .replace("{Stagione}", "S01")
                .replace("{Episodio}", "E02")
                .replace("{Titolo}", title)
                .replace("{Risoluzione}", resolution)
                .replace("{VideoCodec}", codec)
                .replace("{AudioCodec}", audio)
                .replace("{Audio}", audio)
                .replace("{Canali}", channels)
                .replace("{HDR}", hdr)
                .replace("{Lingue}", language),
            _ => format!("{series} - S01E02 - {title}"),
        }
    });
    let save = move |_| {
        let format_value = format.get();
        let template_value = template.get();
        let message = message;
        spawn_local(async move {
            let first = send(
                "POST",
                "/api/config/settings",
                Some(json!({"key": "rename_format", "value": format_value})),
            )
            .await;
            if let Err(error) = first {
                message.set(error);
                return;
            }
            match send(
                "POST",
                "/api/config/settings",
                Some(json!({"key": "rename_template", "value": template_value})),
            )
            .await
            {
                 Ok(_) => message.set(tr(data, "Rinomina salvata")),
                Err(error) => message.set(error),
            }
        });
    };
    view! {
        <div class="settings-row" id="setting-rename-format" title=ctx_tr("Schema usato per comporre il nome dei file: base, standard, completo o personalizzato.")>
            <label>{ctx_tr("Formato")}</label>
            <select prop:value=format title=ctx_tr("Scegli lo schema di rinomina") on:change=move |event| format.set(event_target_value(&event))>
                {RENAME_FORMAT_OPTIONS.iter().map(|(value, label)| view! { <option value=*value>{ctx_tr(label)}</option> }).collect_view()}
            </select>
            <small class="muted"></small>
        </div>
        <div class="settings-row" id="setting-rename-template" title=ctx_tr("Template usato quando il formato è 'personalizzato': combina i segnaposto come {Serie} o {Risoluzione}.")>
            <label>{ctx_tr("Template custom")}</label>
            <input prop:value=template title=ctx_tr("Template personalizzato del nome file") on:input=move |event| template.set(event_target_value(&event)) placeholder=ctx_tr("{Serie} - {Stagione}{Episodio} - {Titolo}") />
            <div class="form-actions"><button class="btn sm primary" title=ctx_tr("Salva il formato e il template di rinomina") on:click=save>{ctx_tr("Salva")}</button><small class="muted">{message}</small></div>
        </div>
        <div class="field" style="margin-top:10px">
            <span>{ctx_tr("Pezzi del nome (aggiungi al template)")}</span>
            <div class="toolbar">
                {RENAME_TOKENS.iter().map(|(token, label)| {
                    let token = *token;
                     let token_title = tr_format(data, "Aggiungi il segnaposto {token} al template", &[("{token}", token.to_string())]);
                    view! {
                         <button class="btn sm" title=token_title on:click=move |_| template.update(|value| value.push_str(token))>{ctx_tr(label)}</button>
                    }
                }).collect_view()}
            </div>
        </div>
        <div class="field" style="margin-top:12px">
            <span>{ctx_tr("Anteprima")}</span>
            <code style="display:block;padding:10px 12px;border-radius:8px;background:var(--code-bg);color:var(--code-text)">{move || format!("{}.mkv", preview.get())}</code>
        </div>
    }
}

#[component]
fn NasPathsEditor() -> impl IntoView {
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let rules = RwSignal::new(Vec::<Value>::new());
    let message = RwSignal::new(String::new());
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(value) = get("/api/tag-dir-rules").await {
                rules.set(array(&value, "items"));
            }
        });
    });
    view! {
        <div class="stack" style="margin-top:10px">
            <div class="table-wrap">
                <table class="data-table">
                    <thead><tr><th>{ctx_tr("Tag")}</th><th>{ctx_tr("Cartella temporanea")}</th><th>{ctx_tr("Cartella finale (NAS)")}</th><th></th></tr></thead>
                    <tbody>
                        {move || rules.get().iter().enumerate().map(|(index, rule)| {
                            let tag = raw(rule, "tag", "");
                            let temp = raw(rule, "temp_dir", "");
                            let final_dir = raw(rule, "final_dir", "");
                            view! {
                                <tr>
                                    <td><input prop:value=tag title=ctx_tr("Tag/categoria della release (es. Film, Serie TV). Deve corrispondere al tag usato per archiviare.") on:input=move |event| { let value = event_target_value(&event); rules.update(|items| { if let Some(item) = items.get_mut(index) { item["tag"] = Value::String(value); } }); } /></td>
                                    <td><input prop:value=temp title=ctx_tr("Cartella in cui scaricare temporaneamente i file di questa categoria prima dello spostamento finale.") on:input=move |event| { let value = event_target_value(&event); rules.update(|items| { if let Some(item) = items.get_mut(index) { item["temp_dir"] = Value::String(value); } }); } /></td>
                                    <td><input prop:value=final_dir title=ctx_tr("Cartella NAS in cui archiviare definitivamente i file di questa categoria.") on:input=move |event| { let value = event_target_value(&event); rules.update(|items| { if let Some(item) = items.get_mut(index) { item["final_dir"] = Value::String(value); } }); } /></td>
                                    <td><button class="btn sm danger" title=ctx_tr("Rimuovi questa regola di percorso") on:click=move |_| rules.update(|items| { if index < items.len() { items.remove(index); } })>{ctx_tr("X")}</button></td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
            <div class="toolbar">
                <button class="btn sm" title=ctx_tr("Aggiungi una nuova regola tag → cartelle") on:click=move |_| rules.update(|items| items.push(json!({"tag": "", "temp_dir": "", "final_dir": ""})))>{ctx_tr("Aggiungi regola")}</button>
                 <button class="btn sm primary" title=ctx_tr("Salva tutte le regole di percorso") on:click=move |_| { let payload = Value::Array(rules.get()); let message = message; spawn_local(async move { match send("POST", "/api/tag-dir-rules", Some(payload)).await { Ok(_) => message.set(tr(data, "Percorsi salvati")), Err(error) => message.set(tr(data, &error)) } }); }>{ctx_tr("Salva percorsi")}</button>
                <small class="muted">{message}</small>
            </div>
        </div>
    }
}

/// Comma-separated list helpers for the small automation editors
/// (watched folders, event hooks).
fn csv_list(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn set_csv_list(item: &mut Value, key: &str, text: &str) {
    let list = text
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Value::String(value.to_string()))
        .collect::<Vec<_>>();
    item[key] = Value::Array(list);
}

#[component]
fn WatchedFoldersPanel() -> impl IntoView {
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let folders = RwSignal::new(Vec::<Value>::new());
    let message = RwSignal::new(String::new());
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(value) = get("/api/watched-folders").await {
                folders.set(array(&value, "items"));
            }
        });
    });
    view! {
        <Panel title="Cartelle osservate">
            <p class="hint">{ctx_tr("Rextto controlla queste cartelle ogni 15 secondi e aggiunge i file .torrent e .magnet che vi vengono copiati. Dopo un'aggiunta riuscita il file viene rimosso (o rinominato in .imported se \"Rimuovi dopo\" è disattivo).")}</p>
            <div class="stack">
                {move || folders.get().iter().enumerate().map(|(index, folder)| {
                    let enabled = flag(folder, "enabled", true);
                    let recursive = flag(folder, "recursive", false);
                    let delete_after = flag(folder, "delete_after", true);
                    view! {
                        <section class="setting-group">
                            <h4>{raw(folder, "path", "Cartella")}</h4>
                            <div class="setting-group-body">
                                <div class="field" title=ctx_tr("Cartella locale che Rextto controlla periodicamente per nuovi file .torrent e .magnet.")><span>{ctx_tr("Percorso")}</span>
                                    <input prop:value=raw(folder, "path", "") on:input=move |event| { let value = event_target_value(&event); folders.update(|items| { if let Some(item) = items.get_mut(index) { item["path"] = Value::String(value); } }); } />
                                </div>
                                <label class="check-inline" title=ctx_tr("Controlla questa cartella ogni 15 secondi. Se disattivata, il percorso resta salvato ma non viene importato nulla.")><input type="checkbox" prop:checked=enabled on:change=move |event| { let value = event_target_checked(&event); folders.update(|items| { if let Some(item) = items.get_mut(index) { item["enabled"] = Value::Bool(value); } }); } />{ctx_tr("Attiva")}</label>
                                <label class="check-inline" title=ctx_tr("Cerca anche nelle sottocartelle, non solo nella directory indicata.")><input type="checkbox" prop:checked=recursive on:change=move |event| { let value = event_target_checked(&event); folders.update(|items| { if let Some(item) = items.get_mut(index) { item["recursive"] = Value::Bool(value); } }); } />{ctx_tr("Ricorsiva")}</label>
                                <label class="check-inline" title=ctx_tr("Dopo l'importazione riuscita elimina il file sorgente. Se disattivata, il file viene rinominato con estensione .imported per evitare nuovi tentativi.")><input type="checkbox" prop:checked=delete_after on:change=move |event| { let value = event_target_checked(&event); folders.update(|items| { if let Some(item) = items.get_mut(index) { item["delete_after"] = Value::Bool(value); } }); } />{ctx_tr("Rimuovi dopo l'aggiunta")}</label>
                                <div class="toolbar">
                                    <button class="btn sm danger" title=ctx_tr("Rimuove questa cartella dall'elenco senza cancellare file dal disco.") on:click=move |_| folders.update(|items| { if index < items.len() { items.remove(index); } })>{ctx_tr("Rimuovi cartella")}</button>
                                </div>
                            </div>
                        </section>
                    }
                }).collect_view()}
            </div>
            <div class="toolbar">
                <button class="btn sm" title=ctx_tr("Aggiunge una nuova cartella osservata; inserisci il percorso prima di salvarla.") on:click=move |_| folders.update(|items| items.push(json!({"path":"","enabled":true,"recursive":false,"delete_after":true})))>{ctx_tr("Aggiungi cartella")}</button>
                <button class="btn primary" title=ctx_tr("Salva tutti i percorsi e le relative opzioni. Il worker li applica al successivo controllo.") on:click=move |_| {
                    let payload = Value::Array(folders.get());
                    spawn_local(async move {
                        match send("POST", "/api/watched-folders", Some(payload)).await {
                             Ok(_) => message.set(tr(data, "Cartelle salvate")),
                             Err(error) => message.set(tr(data, &error)),
                        }
                    });
                }>{ctx_tr("Salva cartelle")}</button>
                <small class="muted">{move || message.get()}</small>
            </div>
        </Panel>
    }
}

#[component]
fn ProvidersStatusPanel() -> impl IntoView {
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let items = RwSignal::new(Vec::<Value>::new());
    let message = RwSignal::new(String::new());
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(value) = get("/api/providers/status").await {
                items.set(array(&value, "items"));
            }
        });
    });
    view! {
        <Panel title="Sorgenti in backoff">
            <p class="hint">{ctx_tr("Sorgenti disattivate temporaneamente dopo ripetuti errori. Rextto le riattiva da solo; qui puoi azzerare il timer di una singola sorgente.")}</p>
            <Show when=move || items.get().is_empty()>
                <Empty text="Nessuna sorgente disattivata." />
            </Show>
            <div class="table-wrap">
                <table class="data-table">
                    <thead><tr><th>{ctx_tr("Sorgente")}</th><th>{ctx_tr("Tipo")}</th><th>{ctx_tr("Livello")}</th><th>{ctx_tr("Riattiva alle")}</th><th>{ctx_tr("Ultimo errore")}</th><th></th></tr></thead>
                    <tbody>
                        {move || items.get().iter().enumerate().map(|(index, item)| {
                            view! {
                                <tr>
                                    <td class="truncate">{text(item, "provider", "-")}</td>
                                    <td>{text(item, "kind", "-")}</td>
                                    <td class="numeric">{number(item, "level")}</td>
                                    <td class="muted">{text(item, "disabled_till", "-")}</td>
                                    <td class="muted truncate">{text(item, "last_error", "-")}</td>
                                    <td><button class="btn sm" title=ctx_tr("Azzera il backoff di questa sorgente e consente un nuovo tentativo immediato.") on:click=move |_| {
                                        let provider = text(&items.get()[index], "provider", "");
                                        spawn_local(async move {
                                            match send("POST", "/api/providers/status", Some(json!({"provider": provider}))).await {
                                                 Ok(_) => message.set(tr(data, "Timer azzerato")),
                                                 Err(error) => message.set(tr(data, &error)),
                                            }
                                            if let Ok(value) = get("/api/providers/status").await {
                                                items.set(array(&value, "items"));
                                            }
                                        });
                                    }>{ctx_tr("Azzera")}</button></td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
            <div class="toolbar"><small class="muted">{move || message.get()}</small></div>
        </Panel>
    }
}

#[component]
fn EventHooksPanel() -> impl IntoView {
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let hooks = RwSignal::new(Vec::<Value>::new());
    let message = RwSignal::new(String::new());
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(value) = get("/api/event-hooks").await {
                hooks.set(array(&value, "items"));
            }
        });
    });
    view! {
        <Panel title="Hook eventi">
            <p class="hint">{ctx_tr("Esegue un programma esterno su eventi Rextto (download_started, torrent_completed, season_pack_completed, torrent_error, ...). Nei campi sono disponibili i segnaposto {event}, {title}, {hash}, {path}, {series}, {season}, {episode}, {kind}, {source}, {quality_score}. Gli stessi valori sono esportati come variabili REXTTO_*.")}</p>
            <div class="stack">
                {move || hooks.get().iter().enumerate().map(|(index, hook)| {
                    let enabled = flag(hook, "enabled", true);
                    view! {
                        <section class="setting-group">
                            <h4>{raw(hook, "name", "Hook")}</h4>
                            <div class="setting-group-body">
                                <div class="field"><span>{ctx_tr("Nome")}</span>
                                    <input prop:value=raw(hook, "name", "") on:input=move |event| { let value = event_target_value(&event); hooks.update(|items| { if let Some(item) = items.get_mut(index) { item["name"] = Value::String(value); } }); } />
                                </div>
                                <label class="check-inline"><input type="checkbox" prop:checked=enabled on:change=move |event| { let value = event_target_checked(&event); hooks.update(|items| { if let Some(item) = items.get_mut(index) { item["enabled"] = Value::Bool(value); } }); } />{ctx_tr("Attivo")}</label>
                                <div class="field"><span>{ctx_tr("Eventi (separati da virgola, vuoto = tutti)")}</span>
                                    <input prop:value=csv_list(hook, "events") on:input=move |event| { let value = event_target_value(&event); hooks.update(|items| { if let Some(item) = items.get_mut(index) { set_csv_list(item, "events", &value); } }); } />
                                </div>
                                <div class="field"><span>{ctx_tr("Programma (percorso eseguibile)")}</span>
                                    <input prop:value=raw(hook, "program", "") on:input=move |event| { let value = event_target_value(&event); hooks.update(|items| { if let Some(item) = items.get_mut(index) { item["program"] = Value::String(value); } }); } />
                                </div>
                                <div class="field"><span>{ctx_tr("Argomenti (supportano i segnaposto)")}</span>
                                    <input prop:value=raw(hook, "args", "") on:input=move |event| { let value = event_target_value(&event); hooks.update(|items| { if let Some(item) = items.get_mut(index) { item["args"] = Value::String(value); } }); } />
                                </div>
                                <div class="field"><span>{ctx_tr("Timeout (secondi)")}</span>
                                    <input prop:value=hook.get("timeout_secs").and_then(Value::as_i64).unwrap_or(60).to_string() on:input=move |event| { let value = event_target_value(&event); hooks.update(|items| { if let Some(item) = items.get_mut(index) { item["timeout_secs"] = value.trim().parse::<i64>().map(Value::from).unwrap_or(Value::from(60)); } }); } />
                                </div>
                                <div class="toolbar">
                                    <button class="btn sm danger" on:click=move |_| hooks.update(|items| { if index < items.len() { items.remove(index); } })>{ctx_tr("Rimuovi hook")}</button>
                                </div>
                            </div>
                        </section>
                    }
                }).collect_view()}
            </div>
            <div class="toolbar">
                <button class="btn sm" on:click=move |_| hooks.update(|items| items.push(json!({"name":"Nuovo hook","enabled":true,"events":[],"program":"","args":"","timeout_secs":60})))>{ctx_tr("Aggiungi hook")}</button>
                <button class="btn primary" on:click=move |_| {
                    let payload = Value::Array(hooks.get());
                    spawn_local(async move {
                        match send("POST", "/api/event-hooks", Some(payload)).await {
                             Ok(_) => message.set(tr(data, "Hook salvati")),
                             Err(error) => message.set(tr(data, &error)),
                        }
                    });
                }>{ctx_tr("Salva hook")}</button>
                <small class="muted">{move || message.get()}</small>
            </div>
        </Panel>
    }
}

#[component]
fn SettingsSaveBar() -> impl IntoView {
    let dirty = use_context::<DirtySettings>().expect("dirty settings context");
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    view! {
        <Show when=move || !dirty.items.get().is_empty()>
            <div class="settings-savebar">
                <strong>{move || tr_format(data, "{count} modifiche non salvate", &[("{count}", dirty.items.get().len().to_string())])}</strong>
                <button class="btn primary" disabled=move || busy.get() on:click=move |_| {
                    let entries: Vec<(String, String)> = dirty.items.get().into_iter().collect();
                    busy.set(true);
                    let items = dirty.items;
                    let message = message;
                    spawn_local(async move {
                        let mut errors = 0usize;
                        for (key, value) in entries {
                            if send("POST", "/api/config/settings", Some(json!({"key": key, "value": value}))).await.is_err() {
                                errors += 1;
                            }
                        }
                        if errors == 0 {
                            items.set(std::collections::BTreeMap::new());
                            message.set(tr(data, "Impostazioni salvate"));
                        } else {
                            message.set(tr_format(data, "{errors} errori durante il salvataggio", &[("{errors}", errors.to_string())]));
                        }
                        busy.set(false);
                    });
                 }>{move || if busy.get() { tr(data, "Salvataggio…") } else { tr(data, "Salva tutte") }}</button>
                <button class="btn" on:click=move |_| dirty.items.set(std::collections::BTreeMap::new())>{ctx_tr("Ignora")}</button>
                <small class="muted">{message}</small>
            </div>
        </Show>
    }
}

#[component]
fn SettingsView(data: RwSignal<Data>) -> impl IntoView {
    let nav = use_context::<SettingsNav>();
    let tab = nav
        .map(|state| state.tab)
        .unwrap_or_else(|| RwSignal::new("daemon".to_string()));
    if let Some(state) = nav {
        Effect::new(move |_| {
            let Some(key) = state.focus.get() else { return };
            let state = state;
            spawn_local(async move {
                // Attende che la tab richiesta sia montata nel DOM.
                TimeoutFuture::new(80).await;
                let Some(element) = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| document.get_element_by_id(&format!("setting-{key}")))
                else {
                    state.focus.set(None);
                    return;
                };
                element.scroll_into_view();
                let _ = element.set_attribute("data-flash", "1");
                TimeoutFuture::new(2200).await;
                let _ = element.remove_attribute("data-flash");
                state.focus.set(None);
            });
        });
    }
    view! {
        <div class="view">
            <SettingsSaveBar />
            <div class="tabs">
                {SETTINGS_TABS.iter().map(|(id, label)| view! {
                    <button class="tab" class:active=move || tab.get() == *id on:click=move |_| tab.set((*id).into())>{ctx_tr(label)}</button>
                }).collect_view()}
            </div>
            <Show when=move || tab.get() == "daemon">
                <Panel title="Daemon">
                    <div class="settings-row" id="setting-active" title=ctx_tr("Attiva o disattiva il daemon rextto (i cicli automatici e i download)")><label>{ctx_tr("Attivo")}</label><select prop:value=move || raw(&data.get().config, "active", "false") on:change=move |event| { let value = event_target_value(&event); run_post(data, "/api/config/settings", Some(json!({"key": "active", "value": value})), "Impostazione salvata"); }><option value="true">{ctx_tr("Sì")}</option><option value="false">{ctx_tr("No")}</option></select><small class="muted"></small></div>
                     <TextSetting label="Ricerca automatica serie/film (secondi)" setting_key="refresh_interval" value=Signal::derive(move || raw(&data.get().config, "refresh_secs", "21600")) placeholder="21600" />
                    <TextSetting label="Età massima release (giorni)" setting_key="max_release_age_days" value=Signal::derive(move || raw(&data.get().config, "max_release_age_days", "0")) placeholder="0 = nessun limite" />
                    <TextSetting label="Gap massimi per serie/ciclo" setting_key="gap_fill_max_per_series" value=Signal::derive(move || raw(&data.get().config, "gap_fill_max_per_series", "0")) placeholder="0 = illimitato" />
                    <BooleanSetting label="Gap filling attivo" setting_key="gap_filling" value=Signal::derive(move || raw(&data.get().config, "gap_filling", "true")) />
                    <TextSetting label="Intervallo deep search (ore)" setting_key="gap_deep_interval_hours" value=Signal::derive(move || raw(&data.get().config, "gap_deep_interval_hours", "6")) placeholder="6" />
                    <TextSetting label="Deep search massime per ciclo" setting_key="gap_deep_max_per_cycle" value=Signal::derive(move || raw(&data.get().config, "gap_deep_max_per_cycle", "5")) placeholder="5" />
                </Panel>
            </Show>
            <Show when=move || tab.get() == "advanced">
                <Panel title="Avanzate">
                    <p class="muted">{ctx_tr("Parametri operativi applicati dal motore. 0 disattiva il controllo corrispondente.")}</p>
                    <TextSetting label="Spazio libero minimo per scaricare (GB)" setting_key="min_free_space_gb" value=Signal::derive(move || raw(&data.get().config, "min_free_space_gb", "0")) placeholder="0 = nessun controllo" />
                    <TextSetting label="Trash — giorni di conservazione (0 = sempre tutto)" setting_key="trash_retention_days" value=Signal::derive(move || raw(&data.get().config, "trash_retention_days", "0")) placeholder="0 = pulisci tutto" />
                    <TextSetting label="Archivio — giorni di conservazione (0 = illimitato)" setting_key="archive_retention_days" value=Signal::derive(move || raw(&data.get().config, "archive_retention_days", "0")) placeholder="0 = conserva sempre" />
                    <BooleanSetting label="Pulizia automatica archivio" setting_key="archive_cleanup_enabled" value=Signal::derive(move || raw(&data.get().config, "archive_cleanup_enabled", "false")) />
                    <TextSetting label="Archivio — età massima (giorni)" setting_key="archive_max_age_days" value=Signal::derive(move || raw(&data.get().config, "archive_max_age_days", "0")) placeholder="0 = nessuna età massima" />
                    <TextSetting label="Archivio — mantieni almeno N voci" setting_key="archive_keep_min" value=Signal::derive(move || raw(&data.get().config, "archive_keep_min", "0")) placeholder="0" />
                    <TextSetting label="Pagine feed da leggere" setting_key="stop_on_old_page_threshold" value=Signal::derive(move || raw(&data.get().config, "stop_on_old_page_threshold", "3")) placeholder="3" />
                    <TextSetting label="Verifica rinomina (ore)" setting_key="rename_verify_interval" value=Signal::derive(move || raw(&data.get().config, "rename_verify_interval", "6")) placeholder="6" />
                    <BooleanSetting label="Sposta gli episodi/pack in archivio (non copiare)" setting_key="move_episodes" value=Signal::derive(move || raw(&data.get().config, "move_episodes", "false")) />
                    <BooleanSetting label="Debug (log dettagliati)" setting_key="debug_enabled" value=Signal::derive(move || raw(&data.get().config, "debug_enabled", "false")) />
                </Panel>
            </Show>
            <Show when=move || tab.get() == "acquisition">
                <Panel title="Acquisizione automatica">
                    <p class="muted" title=ctx_tr("Questa scheda controlla quando parte un download, il riempimento dei gap, la manutenzione dei database e il recupero delle informazioni tecniche dei file.")>{ctx_tr("Attesa prima del download, protezione dagli episodi vecchi e pulizia periodica. Le regole di sanità (sottotitoli hardcoded e dimensione anomala) sono automatiche.")}</p>
                    <TextSetting label="Delay serie (minuti, 0 = nessuno)" setting_key="delay_torrent_minutes" value=Signal::derive(move || raw(&data.get().config, "delay_torrent_minutes", "0")) placeholder="0" />
                    <TextSetting label="Delay film (minuti, 0 = nessuno)" setting_key="delay_movies_minutes" value=Signal::derive(move || raw(&data.get().config, "delay_movies_minutes", "0")) placeholder="0" />
                    <TextSetting label="Bypassa il delay sopra questo punteggio (0 = mai)" setting_key="delay_bypass_score" value=Signal::derive(move || raw(&data.get().config, "delay_bypass_score", "0")) placeholder="0" />
                    <p class="muted" title=ctx_tr("La protezione vale solo per episodi non pertinenti a un gap o a un upgrade. Le azioni manuali e il riempimento dei gap non vengono bloccati.")>{ctx_tr("Rextto non scarica mai un episodio più vecchio fuori dai buchi riconosciuti quando possiede già episodi successivi: è una regola fissa, non disattivabile. Un vero upgrade di qualità resta permesso, e gap-fill e azioni manuali passano sempre.")}</p>
                    <BooleanSetting label="Housekeeping periodico attivo" setting_key="housekeeping_enabled" value=Signal::derive(move || raw(&data.get().config, "housekeeping_enabled", "true")) />
                    <TextSetting label="Housekeeping — intervallo (ore)" setting_key="housekeeping_interval_hours" value=Signal::derive(move || raw(&data.get().config, "housekeeping_interval_hours", "24")) placeholder="24" />
                    <p class="hint">{ctx_tr("Housekeeping: cosa pulisce e cosa non tocca")}</p>
                    <ul class="hint">
                        <li>{ctx_tr("Cicli di ricerca: conserva solo le statistiche di ogni ciclo (release analizzate, candidate, download avviati, gap riempiti ed errori). Con 200, quando arriva il 201° ciclo vengono eliminati solo i contatori del ciclo più vecchio, non la release.")}</li>
                        <li>{ctx_tr("Visti nei feed: con 30 elimina le righe storiche dei feed oltre 30 giorni; con 0 non elimina nulla.")}</li>
                        <li>{ctx_tr("Storico download della sezione Scarico: con 90 elimina le righe dei torrent già rimossi oltre 90 giorni; con 0 conserva lo storico. Non riguarda l'Archivio.")}</li>
                        <li>{ctx_tr("Automaticamente rimuove anche torrent in errore oltre 7 giorni, log dei gap oltre 30 giorni, backup di upgrade oltre 30 giorni e backoff delle sorgenti già scaduti.")}</li>
                        <li>{ctx_tr("Non cancella file della libreria, download completati o release dall'Archivio: la retention dell'Archivio è un'impostazione separata.")}</li>
                    </ul>
                    <TextSetting label="Housekeeping — statistiche cicli di ricerca conservate" setting_key="housekeeping_retain_cycles" value=Signal::derive(move || raw(&data.get().config, "housekeeping_retain_cycles", "200")) placeholder="200" />
                    <TextSetting label="Housekeeping — visti nel feed (giorni, 0 = mai)" setting_key="housekeeping_seen_days" value=Signal::derive(move || raw(&data.get().config, "housekeeping_seen_days", "30")) placeholder="30" />
                    <TextSetting label="Housekeeping — storico download (giorni, 0 = conserva)" setting_key="housekeeping_history_days" value=Signal::derive(move || raw(&data.get().config, "housekeeping_history_days", "0")) placeholder="0" />
                    <BooleanSetting label="Backfill MediaInfo automatico" setting_key="media_info_backfill_enabled" value=Signal::derive(move || raw(&data.get().config, "media_info_backfill_enabled", "true")) />
                    <TextSetting label="Backfill MediaInfo — intervallo (minuti)" setting_key="media_info_backfill_interval_minutes" value=Signal::derive(move || raw(&data.get().config, "media_info_backfill_interval_minutes", "60")) placeholder="60" />
                    <TextSetting label="Backfill MediaInfo — file per volta" setting_key="media_info_backfill_batch" value=Signal::derive(move || raw(&data.get().config, "media_info_backfill_batch", "10")) placeholder="10" />
                    <div class="toolbar" style="margin-top:12px">
                        <button class="btn" title=ctx_tr("Esegue subito pulizia tabelle e compattazione dei database, senza attendere il prossimo intervallo automatico.") on:click=move |_| { run_post(data, "/api/maintenance/housekeeping", None, "Housekeeping completato"); }>{ctx_tr("Esegui housekeeping ora")}</button>
                        <button class="btn" title=ctx_tr("Azzera i periodi di disattivazione di tutte le sorgenti e consente nuovi tentativi immediati.") on:click=move |_| { run_post(data, "/api/providers/status", Some(json!({})), "Backoff sorgenti azzerato"); }>{ctx_tr("Azzera backoff sorgenti")}</button>
                    </div>
                </Panel>
                <WatchedFoldersPanel />
                <ProvidersStatusPanel />
            </Show>
            <Show when=move || tab.get() == "sources">
                <Panel title="Sorgenti di ricerca">
                    <div class="grid-2">
                        <div>
                            <div class="field span-full" style="margin:0 0 8px"><span>{ctx_tr("Feed RSS")}</span><small class="hint">{ctx_tr("Una riga per sorgente: incolla l'URL del feed e premi Salva.")}</small></div>
                            <FeedEditor data />
                        </div>
                        <div>
                            <div class="field span-full" style="margin:0 0 8px"><span>{ctx_tr("Indexer Torznab (Jackett / Prowlarr)")}</span><small class="hint">{ctx_tr("Un riquadro per indexer con nome, URL, API key e interruttore attivo.")}</small></div>
                            <IndexerEditor data />
                            <div class="field span-full" style="margin:18px 0 8px"><span>{ctx_tr("FlareSolverr")}</span><small class="hint">{ctx_tr("Serve per superare Cloudflare su alcuni siti/indexer.")}</small></div>
                            <FlareSolverrEditor data />
                        </div>
                    </div>
                    <div class="grid-2" style="margin-top:18px">
                        <div>
                            <div class="field span-full" style="margin:0 0 8px"><span>{ctx_tr("Motori web")}</span><small class="hint">{ctx_tr("Spunta i motori di ricerca da usare per i gap.")}</small></div>
                            <EngineSetting data />
                        </div>
                        <div>
                            <div class="field span-full" style="margin:0 0 8px"><span>{ctx_tr("Filtri contenuto esclusi")}</span><small class="hint">{ctx_tr("Le release che contengono queste parole o script (es. [non-latino], [porno]) vengono escluse.")}</small></div>
                            <ContentFilterEditor data />
                        </div>
                    </div>
                    <div class="grid-2" style="margin-top:18px">
                        <AreaSetting label="Blacklist (parole separate da virgola)" setting_key="blacklist" value=Signal::derive(move || raw(&data.get().config, "blacklist", "[]")) placeholder="cam, ts, screener" rows=2 />
                    </div>
                    <div class="field span-full" style="margin:18px 0 8px">
                        <span>{ctx_tr("Filtri per sorgente")}</span>
                        <small class="hint">{ctx_tr("Blocca per parola chiave solo le release di una sorgente specifica (feed, indexer o motore web), senza disattivarla.")}</small>
                    </div>
                    <SourceFilterEditor data />
                </Panel>
            </Show>
            <Show when=move || tab.get() == "libtorrent">
                <Panel title="Libtorrent">
                    <div class="toolbar" style="margin-bottom:12px">
                        <span class="badge ok">{move || format!("libtorrent {}", text(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "version", "n/d"))}</span>
                         <button class="btn sm" title=ctx_tr("Riapplica subito le impostazioni libtorrent alla sessione attiva") on:click=move |_| { run_post(data, "/api/torrents/apply_settings", None, "Impostazioni libtorrent applicate"); }>{ctx_tr("Applica ora")}</button>
                         <button class="btn sm primary" title=ctx_tr("Applica una base sicura e suggerisce cache e buffer in base alla RAM; la coda si adatta dinamicamente durante il funzionamento") on:click=move |_| {
                             spawn_local(async move {
                                 match send("POST", "/api/torrents/optimize_settings", None).await {
                                     Ok(value) => {
                                         let explanation = text(&value, "explanation", "Base applicata; i valori dinamici verranno adattati durante il funzionamento.");
                                         flash_text(data, "ok", explanation);
                                         trigger_refresh();
                                     }
                                     Err(error) => flash_text(data, "err", error),
                                 }
                             });
                         }>{ctx_tr("Ottimizza")}</button>
                        <span class="muted">{ctx_tr("Alcune modifiche si applicano al riavvio del servizio.")}</span>
                    </div>
                    <div class="grid-2">
                        <SettingGroup title="Generale">
                            <BooleanSetting label="Client abilitato" setting_key="libtorrent_enabled" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "enabled", "false")) />
                             <BooleanSetting label="Auto-gestione dinamica coda e risorse" setting_key="libtorrent_dynamic_queue" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "dynamic_queue", "false")) />
                             <BooleanSetting label="Ottimizzazione continua (periodica)" setting_key="libtorrent_auto_optimize" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "auto_optimize", "false")) />
                             <TextSetting label="Slot download dinamici minimi" setting_key="libtorrent_dynamic_queue_min" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "dynamic_queue_min", "1")) placeholder="1" />
                             <TextSetting label="Slot download dinamici massimi" setting_key="libtorrent_dynamic_queue_max" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "dynamic_queue_max", "10")) placeholder="10" />
                             <p class="hint">{ctx_tr("Con l'ottimizzazione automatica attiva i campi che mostrano Auto sono di sola lettura: li gestisce Rextto. Disattivandola tornano modificabili: 3/3/5 sono il valore base, che la coda dinamica adatta comunque a runtime (download tra minimo e massimo, seed a 1 quando ci sono download in coda, limite = download + seed + 2).")}</p>
                             <BooleanSetting label="Non contare i torrent fermi negli slot attivi" setting_key="libtorrent_dont_count_slow_torrents" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "dont_count_slow_torrents", "true")) />
                             <TextSetting label="Considera stalled dopo (minuti)" setting_key="libtorrent_stall_after_min" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "stall_after_min", "60")) placeholder="60" />
                             <TextSetting label="Retry stalled (minuti)" setting_key="libtorrent_stall_retry_min" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "stall_retry_min", "60")) placeholder="60" />
                             <TextSetting label="Rimozione stalled (minuti, 0 = mai)" setting_key="libtorrent_stall_giveup_min" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "stall_giveup_min", "20160")) placeholder="20160" />
                             <BooleanSetting label="Download sequenziale" setting_key="libtorrent_sequential" value=Signal::derive(move || raw(&data.get().config, "libtorrent_sequential", "false")) />
                            <TextSetting label="Download attivi" setting_key="libtorrent_active_downloads" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "active_downloads", "3")) placeholder="3" managed=Signal::derive(move || flag(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "auto_optimize", false)) />
                            <TextSetting label="Seed attivi" setting_key="libtorrent_active_seeds" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "active_seeds", "3")) placeholder="3" managed=Signal::derive(move || flag(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "auto_optimize", false)) />
                            <TextSetting label="Limite torrent attivi" setting_key="libtorrent_active_limit" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "active_limit", "5")) placeholder="5" managed=Signal::derive(move || flag(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "auto_optimize", false)) />
                            <TextSetting label="Seed ratio globale (0 = infinito)" setting_key="libtorrent_seed_ratio" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "seed_ratio", "0")) placeholder="0" />
                             <TextSetting label="Seed massimo (minuti, fallback)" setting_key="libtorrent_seed_time" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "seed_time_minutes", "0")) placeholder="0" />
                             <TextSetting label="Seed massimo (giorni)" setting_key="libtorrent_seed_time_days" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "seed_time_days", "0")) placeholder="0" />
                              <BooleanSetting label="Elimina i completati dopo il seed" setting_key="auto_remove_completed" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "auto_remove_completed", "false")) />
                        </SettingGroup>
                        <SettingGroup title="Connessioni e prestazioni">
                            <TextSetting label="Limite connessioni totali" setting_key="libtorrent_connections_limit" value=Signal::derive(move || number(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "connections_limit")) placeholder="200" />
                            <TextSetting label="Slot upload" setting_key="libtorrent_upload_slots_limit" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "upload_slots_limit", "-1")) placeholder="-1 = auto" />
                            <TextSetting label="Half-open limit" setting_key="libtorrent_half_open_limit" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "half_open_limit", "-1")) placeholder="-1 = auto" />
                            <TextSetting label="Connessioni max per torrent" setting_key="libtorrent_max_connections_per_torrent" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "max_connections_per_torrent", "-1")) placeholder="-1 = illimitato" />
                            <TextSetting label="Upload max per torrent" setting_key="libtorrent_max_uploads_per_torrent" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "max_uploads_per_torrent", "-1")) placeholder="-1 = illimitato" />
                            <TextSetting label="Thread AIO disco" setting_key="libtorrent_aio_threads" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "aio_threads", "-1")) placeholder="-1 = auto" />
                             <TextSetting label="Cache disco (blocchi, -1 auto)" setting_key="libtorrent_cache_size" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "cache_size", "-1")) placeholder="-1 = auto" managed=Signal::derive(move || flag(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "auto_optimize", false)) />
                            <TextSetting label="Scadenza cache (s)" setting_key="libtorrent_cache_expiry" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "cache_expiry", "300")) placeholder="300" />
                            <TextSetting label="Coda alert" setting_key="libtorrent_alert_queue_size" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "alert_queue_size", "1000")) placeholder="1000" />
                        </SettingGroup>
                        <SettingGroup title="Protocolli e tracker">
                            <BooleanSetting label="DHT" setting_key="libtorrent_dht" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "dht", "true")) />
                            <BooleanSetting label="PEX" setting_key="libtorrent_pex" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "pex", "true")) />
                            <BooleanSetting label="LSD" setting_key="libtorrent_lsd" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "lsd", "true")) />
                            <BooleanSetting label="UPnP" setting_key="libtorrent_upnp" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "upnp", "true")) />
                            <BooleanSetting label="NAT-PMP" setting_key="libtorrent_natpmp" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "natpmp", "true")) />
                            <BooleanSetting label="uTP" setting_key="libtorrent_utp" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "utp", "true")) />
                            <BooleanSetting label="Preferisci RC4" setting_key="libtorrent_prefer_rc4" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "prefer_rc4", "false")) />
                            <BooleanSetting label="Annuncia a tutti i tracker" setting_key="libtorrent_announce_to_all_trackers" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "announce_to_all_trackers", "false")) />
                            <BooleanSetting label="Annuncia a tutti i tier" setting_key="libtorrent_announce_to_all_tiers" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "announce_to_all_tiers", "false")) />
                            <BooleanSetting label="Più connessioni per IP" setting_key="libtorrent_allow_multiple_connections_per_ip" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "allow_multiple_connections_per_ip", "true")) />
                            <TextSetting label="Intervallo announce (s)" setting_key="libtorrent_announce_interval" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "announce_interval", "1800")) placeholder="1800" />
                            <TextSetting label="Connect boost" setting_key="libtorrent_torrent_connect_boost" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "torrent_connect_boost", "50")) placeholder="50" />
                            <TextSetting label="Nodi bootstrap DHT" setting_key="libtorrent_dht_bootstrap_nodes" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "dht_bootstrap_nodes", "")) placeholder="router.bittorrent.com:6881" />
                        </SettingGroup>
                        <SettingGroup title="Sicurezza, proxy e rete">
                            <SelectSetting label="Cifratura" setting_key="libtorrent_encryption" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "encryption", "1")) options=ENCRYPTION_OPTIONS />
                            <BooleanSetting label="Applica IP filter" setting_key="libtorrent_apply_ip_filter" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "apply_ip_filter", "true")) />
                            <TextSetting label="IP filter (file/URL)" setting_key="libtorrent_ipfilter_url" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "ip_filter_path", "")) placeholder="/path/ipfilter.dat" />
                            <IpFilterControl data />
                            <TextSetting label="Proxy host" setting_key="libtorrent_proxy_host" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "proxy_host", "")) placeholder="127.0.0.1" />
                            <TextSetting label="Proxy porta" setting_key="libtorrent_proxy_port" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "proxy_port", "0")) placeholder="0" />
                            <TextSetting label="Interfacce listen" setting_key="libtorrent_listen_interfaces" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "listen_interfaces", "")) placeholder="0.0.0.0:6881-6891" />
                            <NetworkInterfaceSetting data />
                        </SettingGroup>
                         <SettingGroup title="RAM disk e porte">
                             <BooleanSetting label="Usa il RAM disk" setting_key="libtorrent_ramdisk_enabled" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "ramdisk_enabled", "true")) />
                             <RamDiskControl />
                             <TextSetting label="Dimensione massima per torrent (GB)" setting_key="libtorrent_ramdisk_threshold_gb" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "ramdisk_threshold_gb", "3.5")) placeholder="3.5" />
                            <TextSetting label="Margine libero da mantenere (GB)" setting_key="libtorrent_ramdisk_margin_gb" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "ramdisk_margin_gb", "0.5")) placeholder="0.5" />
                            <TextSetting label="Spazio minimo libero (byte, 0 = dal margine)" setting_key="libtorrent_ramdisk_min_free_bytes" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "ramdisk_min_free_bytes", "")) placeholder="0" />
                             <TextSetting label="Porta minima" setting_key="libtorrent_port_min" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "port_min", "6881")) placeholder="6881" />
                             <TextSetting label="Porta massima" setting_key="libtorrent_port_max" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "port_max", "6891")) placeholder="6891" />
                             <PortCheckControl data />
                         </SettingGroup>
                    </div>
                    <div class="field span-full" style="margin:18px 0 8px"><span>{ctx_tr("Velocità e programmazione")}</span><small class="hint">{ctx_tr("Limiti globali di banda e fasce orarie. Le modifiche si applicano entro un minuto, senza riavvio.")}</small></div>
                    <TextSetting label="Download globale (KiB/s, 0 = illimitato)" setting_key="libtorrent_dl_limit" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "download_limit_kib", "0")) placeholder="0" />
                    <TextSetting label="Upload globale (KiB/s, 0 = illimitato)" setting_key="libtorrent_ul_limit" value=Signal::derive(move || raw(&data.get().config.get("libtorrent").cloned().unwrap_or_default(), "upload_limit_kib", "0")) placeholder="0" />
                    <BooleanSetting label="Programmazione velocità attiva" setting_key="libtorrent_sched_enabled" value=Signal::derive(move || raw(&data.get().config, "libtorrent_sched_enabled", "false")) />
                    <TextSetting label="Programmazione — ora inizio (HH:MM)" setting_key="libtorrent_sched_start" value=Signal::derive(move || raw(&data.get().config, "libtorrent_sched_start", "23:00")) placeholder="23:00" />
                    <TextSetting label="Programmazione — ora fine (HH:MM)" setting_key="libtorrent_sched_end" value=Signal::derive(move || raw(&data.get().config, "libtorrent_sched_end", "08:00")) placeholder="08:00" />
                    <TextSetting label="Programmazione — giorni (0=Lun … 6=Dom, es. 0,1,2,3,4)" setting_key="libtorrent_sched_days" value=Signal::derive(move || raw(&data.get().config, "libtorrent_sched_days", "")) placeholder="0,1,2,3,4" />
                    <TextSetting label="Programmazione — download (KiB/s)" setting_key="libtorrent_sched_dl_limit" value=Signal::derive(move || raw(&data.get().config, "libtorrent_sched_dl_limit", "0")) placeholder="0" />
                    <TextSetting label="Programmazione — upload (KiB/s)" setting_key="libtorrent_sched_ul_limit" value=Signal::derive(move || raw(&data.get().config, "libtorrent_sched_ul_limit", "0")) placeholder="0" />
                    <div class="field span-full" style="margin:18px 0 8px"><span>{ctx_tr("Impostazioni avanzate")}</span><small class="hint">{ctx_tr("Chiavi libtorrent applicate alla sessione, una per riga nel formato chiave=valore. Solo le chiavi riconosciute vengono applicate.")}</small></div>
                    <AreaSetting
                        label="Impostazioni libtorrent avanzate"
                        setting_key="libtorrent_extra_settings"
                        value=Signal::derive(move || raw(&data.get().config, "libtorrent_extra_settings", ""))
                        placeholder="Una per riga: chiave=valore\nmax_peerlist_size=4000\nmax_queued_disk_bytes=104857600\nactive_downloads=6\nsmooth_connects=true\nrequest_timeout=10"
                        rows=6
                    />
                </Panel>
            </Show>
            <Show when=move || tab.get() == "scores">
                <Panel title="Punteggi qualità">
                    <p class="muted">{ctx_tr("Pesi usati per scegliere la release migliore. Valori più alti = preferiti. Lascia vuoto per usare il default.")}</p>
                    <ScoreEditor data />
                </Panel>
                <ScoreGroupsEditor data />
                <ScoreSimulator data />
            </Show>
            <Show when=move || tab.get() == "rename">
                <Panel title="Rinomina e pulizia">
                    <BooleanSetting label="Rinomina episodi" setting_key="rename_episodes" value=Signal::derive(move || raw(&data.get().config, "rename_episodes", "false")) />
                    <RenameEditor data />
                    <TmdbKeySetting data />
                     <TvdbKeySetting data />
                     <TextSetting label="Lingua TVDB (es. ita, eng)" setting_key="tvdb_language" value=Signal::derive(move || raw(&data.get().config, "tvdb_language", "ita")) placeholder="ita" />
                     <TextSetting label="Lingua TMDB (es. it-IT)" setting_key="tmdb_language" value=Signal::derive(move || raw(&data.get().config, "tmdb_language", "it-IT")) placeholder="it-IT" />
                     <TextSetting label="Lingua predefinita (es. ita)" setting_key="default_language" value=Signal::derive(move || raw(&data.get().config, "default_language", "ita")) placeholder="ita" />
                     <BooleanSetting label="Cleanup upgrade" setting_key="cleanup_upgrades" value=Signal::derive(move || raw(&data.get().config, "cleanup_upgrades", "false")) />
                     <TextSetting label="Differenza minima score per cleanup" setting_key="cleanup_min_score_diff" value=Signal::derive(move || number(&data.get().config, "cleanup_min_score_diff")) placeholder="0" />
                    <TextSetting label="Differenza minima score per upgrade" setting_key="upgrade_min_score_diff" value=Signal::derive(move || number(&data.get().config, "upgrade_min_score_diff")) placeholder="200" />
                    <SecretSetting label="Token API Rextto" setting_key="api_token" />
                </Panel>
            </Show>
            <Show when=move || tab.get() == "notify">
                <Panel title="Notifiche">
                    <div class="mode-banner" class:active=move || data.get().config.get("telegram_configured").and_then(Value::as_bool).unwrap_or(false)>
                         <strong>{move || if data.get().config.get("telegram_configured").and_then(Value::as_bool).unwrap_or(false) { tr(data, "Telegram configurato") } else { tr(data, "Telegram non configurato") }}</strong>
                         <span>{move || format!("Chat ID: {} · token: {}", raw(&data.get().config, "telegram_chat_id", "-"), if data.get().config.get("telegram_configured").and_then(Value::as_bool).unwrap_or(false) { tr(data, "salvato") } else { tr(data, "mancante") })}</span>
                    </div>
                    <BooleanSetting label="Telegram attivo" setting_key="notify_telegram" value=Signal::derive(move || raw(&data.get().config, "notify_telegram", "false")) />
                    <SecretSetting label="Telegram bot token" setting_key="telegram_bot_token" />
                    <TextSetting label="Telegram chat ID" setting_key="telegram_chat_id" value=Signal::derive(move || raw(&data.get().config, "telegram_chat_id", "")) placeholder="155086622" />
                    <TextSetting label="Webhook URL" setting_key="notify_webhook_url" value=Signal::derive(move || raw(&data.get().config, "notify_webhook_url", "")) placeholder="https://…" />
                    <SecretSetting label="Webhook secret" setting_key="notify_webhook_secret" />
                    <BooleanSetting label="Email attiva" setting_key="notify_email" value=Signal::derive(move || raw(&data.get().config, "notify_email", "false")) />
                    <TextSetting label="SMTP" setting_key="email_smtp" value=Signal::derive(move || raw(&data.get().config, "email_smtp", "smtp.gmail.com:587")) placeholder="smtp.gmail.com:587" />
                    <TextSetting label="Email mittente" setting_key="email_from" value=Signal::derive(move || raw(&data.get().config, "email_from", "")) placeholder="mittente@example.com" />
                    <TextSetting label="Email destinatario" setting_key="email_to" value=Signal::derive(move || raw(&data.get().config, "email_to", "")) placeholder="destinatario@example.com" />
                    <SecretSetting label="Password email" setting_key="email_password" />
                    <div class="toolbar" style="margin-top:12px">
                        <button class="btn" on:click=move |_| { run_post(data, "/api/test-notification", Some(json!({"message": "Rextto: test notifica"})), "Notifica inviata"); }>{ctx_tr("Invia notifica di test")}</button>
                    </div>
                </Panel>
            </Show>
            <Show when=move || tab.get() == "paths">
                <Panel title="Percorsi runtime">
                     <PathSetting label="Cartella archivio" setting_key="archive_root" value=Signal::derive(move || text(&data.get().config.get("paths").cloned().unwrap_or_default(), "archive_root", "")) placeholder="/mnt/nas" />
                     <PathSetting label="Cartella trash" setting_key="trash_path" value=Signal::derive(move || text(&data.get().config.get("paths").cloned().unwrap_or_default(), "trash_path", "")) placeholder="/mnt/nas/.trash" />
                     <SelectSetting label="Azione cleanup" setting_key="cleanup_action" value=Signal::derive(move || raw(&data.get().config, "cleanup_action", "move")) options=CLEANUP_ACTION_OPTIONS />
                     <PathSetting label="Download libtorrent" setting_key="libtorrent_dir" value=Signal::derive(move || text(&data.get().config.get("paths").cloned().unwrap_or_default(), "libtorrent_dir", "")) placeholder="/mnt/downloads" />
                    <PathSetting label="Cartella temporanea libtorrent" setting_key="libtorrent_temp_dir" value=Signal::derive(move || text(&data.get().config.get("paths").cloned().unwrap_or_default(), "libtorrent_temp_dir", "")) placeholder="/mnt/temp" />
                 </Panel>
                <Panel title="Percorsi NAS per categoria (tag)">
                    <p class="muted">{ctx_tr("Le release vengono archiviate nella cartella finale del tag corrispondente (es. tag 'Film' → /home/user/film). Le serie usano il percorso della serie.")}</p>
                    <NasPathsEditor />
                </Panel>
            </Show>
            <Show when=move || tab.get() == "i18n">
                <TranslationTools />
            </Show>
        </div>
    }
}

fn load_ramdisk_info(
    paths: RwSignal<Vec<Value>>,
    configured: RwSignal<String>,
    create_path: RwSignal<String>,
    create_available: RwSignal<bool>,
    message: RwSignal<String>,
) {
    spawn_local(async move {
        match get("/api/ramdisk").await {
            Ok(value) => {
                paths.set(value.get("paths").and_then(Value::as_array).cloned().unwrap_or_default());
                configured.set(text(&value, "configured", ""));
                create_path.set(text(&value, "create_path", "/dev/shm/rextto"));
                create_available.set(value.get("create_available").and_then(Value::as_bool).unwrap_or(false));
                message.set(String::new());
            }
            Err(error) => message.set(error),
        }
    });
}

#[component]
fn RamDiskControl() -> impl IntoView {
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let paths = RwSignal::new(Vec::<Value>::new());
    let configured = RwSignal::new(String::new());
    let selected = RwSignal::new(String::new());
    let create_path = RwSignal::new("/dev/shm/rextto".to_string());
    let create_available = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let loaded_configured = configured;
    Effect::new(move |_| {
        load_ramdisk_info(paths, loaded_configured, create_path, create_available, message);
    });
    Effect::new(move |_| {
        let current = configured.get();
        if !current.is_empty() {
            selected.set(current);
        }
    });
    view! {
        <div class="field span-full" style="margin-top:8px">
            <span>{ctx_tr("Percorso RAM disk")}</span>
            <select prop:value=selected on:change=move |event| selected.set(event_target_value(&event))>
                <option value="">{ctx_tr("Seleziona un RAM disk disponibile")}</option>
                <For
                    each=move || paths.get()
                    key=|item: &Value| text(item, "path", "")
                    children=move |item| {
                        let path = text(&item, "path", "");
                        let filesystem = text(&item, "filesystem", "tmpfs");
                        let writable = item.get("writable").and_then(Value::as_bool).unwrap_or(false);
                        let label = if writable {
                            format!("{path} · {filesystem} · {} {}", size(&item, "free_bytes"), tr(data, "liberi"))
                        } else {
                            format!("{path} · {filesystem} · {}", tr(data, "non scrivibile"))
                        };
                        view! { <option value=path disabled=!writable>{label}</option> }
                    }
                />
            </select>
            <div class="form-actions">
                <button class="btn sm primary" type="button" on:click=move |_| {
                    let path = selected.get();
                    let message = message;
                    spawn_local(async move {
                        if path.trim().is_empty() {
                            message.set(tr(data, "Seleziona prima un percorso RAM disk."));
                            return;
                        }
                        match send("POST", "/api/ramdisk/select", Some(json!({"path":path.clone()}))).await {
                            Ok(value) => {
                                let recommended = value.get("recommended").cloned().unwrap_or_default();
                                let threshold = text(&recommended, "threshold_gb", "3.5");
                                let margin = text(&recommended, "margin_gb", "0.5");
                                 message.set(tr_format(data, "{label}: {path} · {max_label} {threshold} GB · {margin_label} {margin} GB", &[("{label}", tr(data, "RAM disk configurato")), ("{path}", path), ("{max_label}", tr(data, "massimo torrent")), ("{threshold}", threshold), ("{margin_label}", tr(data, "margine")), ("{margin}", margin)]));
                                trigger_refresh();
                            }
                            Err(error) => message.set(error),
                        }
                    });
                }>{ctx_tr("Usa questo percorso")}</button>
                <Show when=move || create_available.get() && configured.get().is_empty()>
                    <button class="btn sm" type="button" on:click=move |_| {
                        let message = message;
                        spawn_local(async move {
                            match send("POST", "/api/ramdisk/create", Some(json!({}))).await {
                                Ok(value) => {
                                    let path = text(&value, "path", "/dev/shm/rextto");
                                    configured.set(path.clone());
                                    selected.set(path);
                                    message.set(tr(data, "RAM disk creato. Ricorda: il contenuto di /dev/shm si perde al riavvio."));
                                    load_ramdisk_info(paths, configured, create_path, create_available, message);
                                    trigger_refresh();
                                }
                                Err(error) => message.set(error),
                            }
                        });
                    }>{ctx_tr("Crea in /dev/shm")}</button>
                </Show>
                <small class="muted">{message}</small>
            </div>
            <p class="hint">{ctx_tr("Rextto mostra i tmpfs/ramfs disponibili. Se non ne hai uno configurato puoi creare /dev/shm/rextto; scegliendo un percorso vengono proposti automaticamente dimensione massima e margine in base allo spazio reale. Il contenuto di /dev/shm non sopravvive al riavvio della macchina.")}</p>
        </div>
    }
}

#[component]
fn TextSetting(
    label: &'static str,
    setting_key: &'static str,
    value: Signal<String>,
    placeholder: &'static str,
    /// True quando il valore è gestito dall'ottimizzazione automatica: il campo
    /// diventa di sola lettura e mostra "Auto" finché quella resta attiva.
    #[prop(optional)] managed: Option<Signal<bool>>,
) -> impl IntoView {
    let draft = RwSignal::new(value.get());
    let message = RwSignal::new(String::new());
    let is_managed = move || managed.map(|signal| signal.get()).unwrap_or(false);
    let dirty = use_context::<DirtySettings>();
    let data = use_context::<RwSignal<Data>>();
    let saved = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        let current = value.get();
        if let Some(expected) = saved.get() {
            if current == expected {
                saved.set(None);
            } else {
                draft.set(expected);
                return;
            }
        }
        let is_dirty = dirty
            .map(|current| current.items.get().contains_key(setting_key))
            .unwrap_or(false);
        if !is_dirty {
            draft.set(current);
        }
    });
    view! {
        <form class="settings-row" id=format!("setting-{setting_key}") title=ctx_tr(setting_tooltip(setting_key)) on:submit=move |event| {
            event.prevent_default();
            let draft_value = draft.get();
            spawn_local(async move {
                match send("POST", "/api/config/settings", Some(json!({"key": setting_key, "value": draft_value.clone()}))).await {
                    Ok(value) => {
                        saved.set(Some(draft_value));
                        message.set(tr_opt(data, &save_message(&value)));
                        if let Some(dirty) = dirty {
                            dirty.items.update(|items| { items.remove(setting_key); });
                        }
                        trigger_refresh();
                    }
                    Err(error) => message.set(error),
                }
            });
        }>
            <label>{ctx_tr(label)}</label>
            <input
                prop:value=move || if is_managed() { "Auto".to_string() } else { draft.get() }
                disabled=is_managed
                on:input=move |event| {
                    let next = event_target_value(&event);
                    draft.set(next.clone());
                    if let Some(dirty) = dirty {
                        dirty.items.update(|items| { items.insert(setting_key.to_string(), next.clone()); });
                    }
                } placeholder=ctx_tr(placeholder) />
            <div class="form-actions">
                <Show
                    when=move || !is_managed()
                    fallback=move || view! { <small class="muted">{ctx_tr("Gestito dall'ottimizzazione automatica")}</small> }>
                    <button class="btn sm primary">{ctx_tr("Salva")}</button>
                    <small class="muted">{message}</small>
                </Show>
            </div>
        </form>
    }
}

#[component]
fn PathSetting(
    label: &'static str,
    setting_key: &'static str,
    value: Signal<String>,
    placeholder: &'static str,
) -> impl IntoView {
    let draft = RwSignal::new(value.get());
    let message = RwSignal::new(String::new());
    let dirty = use_context::<DirtySettings>();
    let data = use_context::<RwSignal<Data>>();
    Effect::new(move |_| {
        let is_dirty = dirty
            .map(|current| current.items.get().contains_key(setting_key))
            .unwrap_or(false);
        if !is_dirty {
            draft.set(value.get());
        }
    });
    view! {
        <form class="settings-row" id=format!("setting-{setting_key}") title=ctx_tr(setting_tooltip(setting_key)) on:submit=move |event| {
            event.prevent_default();
            let draft_value = draft.get();
            spawn_local(async move {
                match send("POST", "/api/config/settings", Some(json!({"key": setting_key, "value": draft_value}))).await {
                    Ok(value) => {
                        message.set(tr_opt(data, &save_message(&value)));
                        if let Some(dirty) = dirty {
                            dirty.items.update(|items| { items.remove(setting_key); });
                        }
                        trigger_refresh();
                    }
                    Err(error) => message.set(error),
                }
            });
        }>
            <label>{ctx_tr(label)}</label>
            <div class="path-picker">
                <input prop:value=draft on:input=move |event| {
                    let next = event_target_value(&event);
                    draft.set(next.clone());
                    if let Some(dirty) = dirty {
                        dirty.items.update(|items| { items.insert(setting_key.to_string(), next.clone()); });
                    }
                } placeholder=ctx_tr(placeholder) />
                <BrowseButton value=draft />
            </div>
            <div class="form-actions"><button class="btn sm primary">{ctx_tr("Salva")}</button><small class="muted">{message}</small></div>
        </form>
    }
}

#[component]
fn AreaSetting(
    label: &'static str,
    setting_key: &'static str,
    value: Signal<String>,
    placeholder: &'static str,
    rows: usize,
) -> impl IntoView {
    let draft = RwSignal::new(value.get());
    let message = RwSignal::new(String::new());
    let dirty = use_context::<DirtySettings>();
    let data = use_context::<RwSignal<Data>>();
    Effect::new(move |_| {
        let is_dirty = dirty
            .map(|current| current.items.get().contains_key(setting_key))
            .unwrap_or(false);
        if !is_dirty {
            draft.set(value.get());
        }
    });
    view! {
        <form class="settings-row" id=format!("setting-{setting_key}") title=ctx_tr(setting_tooltip(setting_key)) on:submit=move |event| {
            event.prevent_default();
            let draft_value = draft.get();
            spawn_local(async move {
                match send("POST", "/api/config/settings", Some(json!({"key": setting_key, "value": draft_value}))).await {
                    Ok(value) => {
                        message.set(tr_opt(data, &save_message(&value)));
                        if let Some(dirty) = dirty {
                            dirty.items.update(|items| { items.remove(setting_key); });
                        }
                    }
                    Err(error) => message.set(error),
                }
            });
        }>
            <label>{ctx_tr(label)}</label>
            <textarea prop:value=draft on:input=move |event| {
                let next = event_target_value(&event);
                draft.set(next.clone());
                if let Some(dirty) = dirty {
                    dirty.items.update(|items| { items.insert(setting_key.to_string(), next.clone()); });
                }
            } placeholder=ctx_tr(placeholder) style=format!("min-height:{}px", rows * 26)></textarea>
            <div class="form-actions"><button class="btn sm primary">{ctx_tr("Salva")}</button><small class="muted">{message}</small></div>
        </form>
    }
}

#[component]
fn SecretSetting(label: &'static str, setting_key: &'static str) -> impl IntoView {
    let draft = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    let dirty = use_context::<DirtySettings>();
    let data = use_context::<RwSignal<Data>>();
    view! {
        <form class="settings-row" id=format!("setting-{setting_key}") title=ctx_tr(setting_tooltip(setting_key)) on:submit=move |event| {
            event.prevent_default();
            let value = draft.get();
            if value.trim().is_empty() {
                message.set(tr_opt(data, "Inserire un valore"));
                return;
            }
            spawn_local(async move {
                match send("POST", "/api/config/settings", Some(json!({"key": setting_key, "value": value}))).await {
                    Ok(_) => {
                        draft.set(String::new());
                        message.set(tr_opt(data, "Salvato"));
                        if let Some(dirty) = dirty {
                            dirty.items.update(|items| { items.remove(setting_key); });
                        }
                    }
                    Err(error) => message.set(error),
                }
            });
        }>
            <label>{ctx_tr(label)}</label>
            <input type="password" prop:value=draft on:input=move |event| {
                let next = event_target_value(&event);
                draft.set(next.clone());
                if let Some(dirty) = dirty {
                    dirty.items.update(|items| { items.remove(setting_key); items.insert(setting_key.to_string(), next.clone()); });
                }
            } placeholder=ctx_tr("non visualizzato") />
            <div class="form-actions"><button class="btn sm primary">{ctx_tr("Salva")}</button><small class="muted">{message}</small></div>
        </form>
    }
}

#[component]
fn TmdbKeySetting(data: RwSignal<Data>) -> impl IntoView {
    let draft = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    let reveal = RwSignal::new(false);
    let configured = Signal::derive(move || {
        data.get()
            .config
            .get("tmdb_configured")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    Effect::new(move |_| {
        let value = raw(&data.get().config, "tmdb_api_key", "");
        if !value.is_empty() {
            draft.set(value);
        }
    });
    view! {
        <div class="settings-row" id="setting-tmdb_api_key" title=ctx_tr("Chiave API TMDB: serve per titoli episodi, poster e metadati.")>
            <label>{ctx_tr("TMDB API key")}</label>
            <div class="stack">
                <div class="path-picker">
                    <input type=move || if reveal.get() { "text" } else { "password" } prop:value=draft on:input=move |event| draft.set(event_target_value(&event)) placeholder=ctx_tr("incolla la chiave API TMDB") />
                     <button type="button" class="btn sm" on:click=move |_| reveal.update(|value| *value = !*value)>{move || if reveal.get() { tr(data, "Nascondi") } else { tr(data, "Mostra") }}</button>
                </div>
                <small class="hint">{ctx_tr("Serve per titoli episodi, poster e metadati. Ottienila su themoviedb.org → Impostazioni → API.")}</small>
            </div>
             <div class="form-actions settings-actions-top">
                 <button class="btn sm primary" on:click=move |_| {
                    let value = draft.get();
                     if value.trim().is_empty() { message.set(tr(data, "Inserire un valore")); return; }
                    let message = message;
                    spawn_local(async move {
                        match send("POST", "/api/config/settings", Some(json!({"key": "tmdb_api_key", "value": value}))).await {
                             Ok(_) => { trigger_refresh(); message.set(tr(data, "Salvato")); }
                            Err(error) => message.set(error),
                        }
                    });
                }>{ctx_tr("Salva")}</button>
                 <span class="badge" class:ok=move || configured.get()>{move || if configured.get() { tr(data, "configurato") } else { tr(data, "non configurato") }}</span>
                <small class="muted">{message}</small>
            </div>
        </div>
    }
}

#[component]
fn TvdbKeySetting(data: RwSignal<Data>) -> impl IntoView {
    let draft = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    let reveal = RwSignal::new(false);
    let configured = Signal::derive(move || {
        data.get()
            .config
            .get("tvdb_configured")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    Effect::new(move |_| {
        let value = raw(&data.get().config, "tvdb_api_key", "");
        if !value.is_empty() {
            draft.set(value);
        }
    });
    view! {
        <div class="settings-row" id="setting-tvdb_api_key" title=ctx_tr("Chiave API TVDB: serve per ricerca serie e metadati.")>
            <label>{ctx_tr("TVDB API key")}</label>
            <div class="stack">
                <div class="path-picker">
                    <input type=move || if reveal.get() { "text" } else { "password" } prop:value=draft on:input=move |event| draft.set(event_target_value(&event)) placeholder=ctx_tr("incolla la chiave API TVDB") />
                     <button type="button" class="btn sm" on:click=move |_| reveal.update(|value| *value = !*value)>{move || if reveal.get() { tr(data, "Nascondi") } else { tr(data, "Mostra") }}</button>
                </div>
            </div>
            <div class="form-actions settings-actions-top">
                <button class="btn sm primary" on:click=move |_| {
                    let value = draft.get();
                    if value.trim().is_empty() { message.set(tr(data, "Inserire un valore")); return; }
                    let message = message;
                    spawn_local(async move {
                        match send("POST", "/api/config/settings", Some(json!({"key": "tvdb_api_key", "value": value}))).await {
                            Ok(_) => { trigger_refresh(); message.set(tr(data, "Salvato")); }
                            Err(error) => message.set(error),
                        }
                    });
                }>{ctx_tr("Salva")}</button>
                 <span class="badge" class:ok=move || configured.get()>{move || if configured.get() { tr(data, "configurato") } else { tr(data, "non configurato") }}</span>
                <small class="muted">{message}</small>
            </div>
        </div>
    }
}

#[component]
fn SelectSetting(
    label: &'static str,
    setting_key: &'static str,
    value: Signal<String>,
    options: &'static [(&'static str, &'static str)],
) -> impl IntoView {
    let draft = RwSignal::new(value.get());
    let message = RwSignal::new(String::new());
    let data = use_context::<RwSignal<Data>>();
    Effect::new(move |_| draft.set(value.get()));
    view! {
        <div class="settings-row" id=format!("setting-{setting_key}") title=ctx_tr(setting_tooltip(setting_key))>
            <label>{ctx_tr(label)}</label>
            <select prop:value=draft on:change=move |event| {
                let selected = event_target_value(&event);
                draft.set(selected.clone());
                spawn_local(async move {
                    match send("POST", "/api/config/settings", Some(json!({"key": setting_key, "value": selected}))).await {
                        Ok(value) => message.set(tr_opt(data, &save_message(&value))),
                        Err(error) => message.set(error),
                    }
                });
            }>
                {options.iter().map(|(value, text)| view! { <option value=*value>{ctx_tr(text)}</option> }).collect_view()}
            </select>
            <small class="muted">{message}</small>
        </div>
    }
}

#[component]
fn BooleanSetting(label: &'static str, setting_key: &'static str, value: Signal<String>) -> impl IntoView {
    view! { <SelectSetting label setting_key value options=BOOL_OPTIONS /> }
}

/// Killswitch VPN: elenca le interfacce di rete del server (con IP e tipo) e
/// forza il traffico libtorrent, in entrata e in uscita, sulla scheda scelta.
#[component]
fn NetworkInterfaceSetting(data: RwSignal<Data>) -> impl IntoView {
    let interfaces = RwSignal::new(Vec::<Value>::new());
    let current = Signal::derive(move || {
        raw(
            &data.get().config.get("libtorrent").cloned().unwrap_or_default(),
            "outgoing_interface",
            "",
        )
    });
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(value) = get("/api/network/interfaces").await {
                let mut list = Vec::new();
                if let Some(map) = value.get("interfaces").and_then(Value::as_object) {
                    for (name, info) in map {
                        list.push(json!({
                            "name": name,
                            "ip": info.get("ip").cloned().unwrap_or_default(),
                            "type": info.get("type").cloned().unwrap_or_default(),
                        }));
                    }
                }
                list.sort_by(|a, b| text(a, "name", "").cmp(&text(b, "name", "")));
                interfaces.set(list);
            }
        });
    });
    view! {
        <div class="settings-row" title=ctx_tr("Killswitch VPN: vincola ascolto e connessioni in uscita a questa interfaccia (es. tun0 o wg0). Richiede il riavvio del servizio.")>
            <label>{ctx_tr("Interfaccia VPN (killswitch)")}</label>
            <select prop:value=move || current.get() on:change=move |event| {
                let value = event_target_value(&event);
                run_post(data, "/api/config/settings", Some(json!({"key": "libtorrent_outgoing_interface", "value": value})), "Interfaccia salvata");
            }>
                <option value="">{ctx_tr("Automatica (nessun vincolo)")}</option>
                {move || interfaces.get().into_iter().map(|item| {
                    let name = text(&item, "name", "");
                    let kind = text(&item, "type", "");
                    let ip = text(&item, "ip", "");
                    let label = if ip.is_empty() { format!("{name} ({kind})") } else { format!("{name} · {ip} ({kind})") };
                    view! { <option value=name.clone()>{label}</option> }
                }).collect_view()}
            </select>
            <small class="muted">{ctx_tr("Se impostata, libtorrent usa solo questa scheda: nessun traffico fuori dalla VPN.")}</small>
        </div>
    }
}

#[component]
fn PortCheckControl(data: RwSignal<Data>) -> impl IntoView {
    let ports = RwSignal::new(Vec::<Value>::new());
    let message = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    view! {
        <div class="settings-row" title=ctx_tr("Test locale delle porte libtorrent: verifica se il sistema può ascoltare su ogni porta dell'intervallo indicato.")>
            <label>{ctx_tr("Test porte torrent")}</label>
            <div class="toolbar">
                <button class="btn sm" type="button" disabled=move || busy.get() on:click=move |_| {
                    busy.set(true);
                    let ports = ports;
                    let message = message;
                    spawn_local(async move {
                        match get("/api/config/check-ports").await {
                            Ok(value) => {
                                let items = value.get("ports").and_then(Value::as_array).cloned().unwrap_or_default();
                                let available = items.iter().filter(|item| item.get("available").and_then(Value::as_bool).unwrap_or(false)).count();
                                 message.set(tr_format(data, "{label}: {available}/{total} {free}", &[("{label}", tr(data, "Test completato")), ("{available}", available.to_string()), ("{total}", items.len().to_string()), ("{free}", tr(data, "libere localmente"))]));
                                ports.set(items);
                            }
                            Err(error) => message.set(error),
                        }
                        busy.set(false);
                    });
                }>{move || if busy.get() { tr(data, "Verifica…") } else { tr(data, "Testa porte") }}</button>
                <small class="muted">{message}</small>
            </div>
            <div class="toolbar">
                {move || ports.get().into_iter().map(|item| {
                    let port = number(&item, "port");
                    let available = item.get("available").and_then(Value::as_bool).unwrap_or(false);
                    let tcp = item.get("tcp_available").and_then(Value::as_bool).unwrap_or(false);
                    let udp = item.get("udp_available").and_then(Value::as_bool).unwrap_or(false);
                    let label = if available { format!("{port} · {}", tr(data, "libera")) } else { format!("{port} · {}", tr(data, "occupata")) };
                    let title = format!("TCP: {} · UDP: {}", if tcp { tr(data, "libera") } else { tr(data, "occupata") }, if udp { tr(data, "libera") } else { tr(data, "occupata") });
                    view! { <span class=if available { "badge ok" } else { "badge" } title=title>{label}</span> }
                }).collect_view()}
            </div>
            <small class="hint">{ctx_tr("Il test verifica solo la disponibilità locale del bind. Non può confermare da solo l'apertura sul router/firewall o la raggiungibilità da Internet; una porta occupata dal listener libtorrent è normale quando il servizio è attivo.")}</small>
        </div>
    }
}

#[component]
fn EngineSetting(data: RwSignal<Data>) -> impl IntoView {
    view! {
        <div class="settings-row" title=ctx_tr("Motori di ricerca web usati per il gap-filling quando feed e indexer non trovano nulla")>
            <label>{ctx_tr("Motori web")}</label>
            <div class="toolbar">
                {ENGINE_OPTIONS.iter().map(|(id, name)| {
                    let id = *id;
                    let checked = move || array(&data.get().config, "websearch_engines").iter().any(|value| value.as_str() == Some(id));
                    view! {
                        <label class="check">
                            <input type="checkbox" prop:checked=checked on:change=move |_| {
                                let mut engines = array(&data.get().config, "websearch_engines").iter().filter_map(|value| value.as_str().map(str::to_owned)).collect::<Vec<_>>();
                                if engines.iter().any(|value| value == id) { engines.retain(|value| value != id); } else { engines.push(id.to_string()); }
                                run_post(data, "/api/config/settings", Some(json!({"key": "websearch_engines", "value": serde_json::to_string(&engines).unwrap_or_else(|_| "[]".into())})), "Motori aggiornati");
                            } />
                            <span>{tr(data, name)}</span>
                        </label>
                    }
                }).collect_view()}
            </div>
            <small class="muted"></small>
        </div>
    }
}

const CONTENT_FILTER_OPTIONS: &[(&str, &str)] = &[
    ("[non-latino]", "Non latino (cirillico, arabo, CJK…)"),
    ("[cjk]", "CJK (cinese, giapponese, coreano)"),
    ("[cirillico]", "Cirillico"),
    ("[arabo]", "Arabo"),
    ("[ebraico]", "Ebraico"),
    ("[thai]", "Thai"),
    ("[porno]", "Porno / contenuti per adulti"),
];

#[component]
fn FeedEditor(data: RwSignal<Data>) -> impl IntoView {
    let feeds = RwSignal::new(Vec::<String>::new());
    let message = RwSignal::new(String::new());
    let loaded = RwSignal::new(false);
    Effect::new(move |_| {
        if loaded.get() {
            return;
        }
        feeds.set(
            array(&data.get().config, "feed_urls")
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect(),
        );
        loaded.set(true);
    });
    view! {
        <div class="stack" style="margin-top:10px">
            <div class="list">
                {move || feeds.get().iter().enumerate().map(|(index, feed)| {
                    let value = feed.clone();
                    view! {
                        <div class="list-item">
                            <input prop:value=value title=ctx_tr("URL del feed RSS: incolla l'indirizzo completo e premi Salva feed") on:input=move |event| { let next = event_target_value(&event); feeds.update(|items| { if let Some(item) = items.get_mut(index) { *item = next; } }); } placeholder=ctx_tr("https://feed.example/rss") />
                            <button class="btn sm danger" title=ctx_tr("Rimuovi questo feed") on:click=move |_| feeds.update(|items| { if index < items.len() { items.remove(index); } })>{ctx_tr("X")}</button>
                        </div>
                    }
                }).collect_view()}
            </div>
            <div class="toolbar">
                <button class="btn sm" on:click=move |_| feeds.update(|items| items.push(String::new()))>{ctx_tr("Aggiungi feed")}</button>
                <button class="btn sm primary" on:click=move |_| {
                    let payload = serde_json::to_string(&feeds.get()).unwrap_or_else(|_| "[]".into());
                    let message = message;
                    spawn_local(async move {
                        match send("POST", "/api/config/settings", Some(json!({"key": "url", "value": payload}))).await {
                             Ok(_) => message.set(tr(data, "Feed salvati")),
                            Err(error) => message.set(error),
                        }
                    });
                }>{ctx_tr("Salva feed")}</button>
                <small class="muted">{message}</small>
            </div>
        </div>
    }
}

#[component]
fn IndexerEditor(data: RwSignal<Data>) -> impl IntoView {
    let indexers = RwSignal::new(Vec::<Value>::new());
    let message = RwSignal::new(String::new());
    let test_results = RwSignal::new(Vec::<Value>::new());
    let verify_busy = RwSignal::new(false);
    let reveal_key = RwSignal::new(false);
    let loaded = RwSignal::new(false);
    Effect::new(move |_| {
        if loaded.get() {
            return;
        }
        indexers.set(array(&data.get().config, "indexers"));
        loaded.set(true);
    });
    view! {
        <div class="stack" style="margin-top:10px">
            {move || indexers.get().iter().enumerate().map(|(index, indexer)| {
                let name = text(indexer, "name", "");
                let url = text(indexer, "url", "");
                let api_key = text(indexer, "api_key", "");
                let enabled = move || indexers.get().get(index).and_then(|item| item.get("enabled")).and_then(Value::as_bool).unwrap_or(true);
                view! {
                    <div class="panel" style="padding:12px">
                        <div class="form-grid">
                            <label class="field" title=ctx_tr("Nome")><span>{ctx_tr("Nome")}</span><input prop:value=name on:input=move |event| { let next = event_target_value(&event); indexers.update(|items| { if let Some(item) = items.get_mut(index) { item["name"] = Value::String(next); } }); } placeholder=ctx_tr("jackett / prowlarr") /></label>
                            <label class="field span-2" title=ctx_tr("URL base dell'indexer: Rextto aggiunge da sé il percorso Torznab corretto")><span>{ctx_tr("URL base")}</span><input prop:value=url on:input=move |event| { let next = event_target_value(&event); indexers.update(|items| { if let Some(item) = items.get_mut(index) { item["url"] = Value::String(next); } }); } placeholder=ctx_tr("http://127.0.0.1:9117") /></label>
                            <label class="field span-2" title=ctx_tr("API key dell'indexer (mascherata)")><span>{ctx_tr("API key")}</span>
                                <div class="path-picker">
                                    <input type=move || if reveal_key.get() { "text" } else { "password" } prop:value=api_key on:input=move |event| { let next = event_target_value(&event); indexers.update(|items| { if let Some(item) = items.get_mut(index) { item["api_key"] = Value::String(next); } }); } placeholder=ctx_tr("chiave API") />
                                    <button type="button" class="btn sm" title=ctx_tr("Mostra o nascondi la chiave") on:click=move |_| reveal_key.update(|value| *value = !*value)>{move || if reveal_key.get() { tr(data, "Nascondi") } else { tr(data, "Mostra") }}</button>
                                </div>
                            </label>
                        </div>
                        <div class="toolbar" style="margin-top:8px">
                            <label class="check" title=ctx_tr("Indexer abilitato: se disattivato non viene interrogato")><input type="checkbox" prop:checked=enabled on:change=move |_| indexers.update(|items| { if let Some(item) = items.get_mut(index) { let current = item.get("enabled").and_then(Value::as_bool).unwrap_or(true); item["enabled"] = Value::Bool(!current); } }) /> <span>{ctx_tr("attivo")}</span></label>
                            <button class="btn sm danger" title=ctx_tr("Rimuovi questo indexer") on:click=move |_| indexers.update(|items| { if index < items.len() { items.remove(index); } })>{ctx_tr("Rimuovi")}</button>
                        </div>
                    </div>
                }
            }).collect_view()}
            <div class="toolbar">
                <button class="btn sm" title=ctx_tr("Aggiungi un indexer Jackett") on:click=move |_| indexers.update(|items| items.push(json!({"name": "jackett", "url": "http://127.0.0.1:9117", "api_key": "", "enabled": true})))>{ctx_tr("+ Jackett")}</button>
                <button class="btn sm" title=ctx_tr("Aggiungi un indexer Prowlarr") on:click=move |_| indexers.update(|items| items.push(json!({"name": "prowlarr", "url": "http://127.0.0.1:9696", "api_key": "", "enabled": true})))>{ctx_tr("+ Prowlarr")}</button>
                <button class="btn sm" title=ctx_tr("Aggiungi un indexer vuoto") on:click=move |_| indexers.update(|items| items.push(json!({"name": "", "url": "", "api_key": "", "enabled": true})))>{ctx_tr("+ Vuoto")}</button>
            </div>
            <div class="toolbar">
                <button class="btn sm primary" title=ctx_tr("Salva la configurazione degli indexer") on:click=move |_| {
                    let payload = serde_json::to_string(&indexers.get()).unwrap_or_else(|_| "[]".into());
                    let message = message;
                    spawn_local(async move {
                        match send("POST", "/api/config/settings", Some(json!({"key": "indexers", "value": payload}))).await {
                             Ok(_) => message.set(tr(data, "Indexer salvati")),
                            Err(error) => message.set(error),
                        }
                    });
                }>{ctx_tr("Salva indexer")}</button>
                <button class="btn sm" disabled=move || verify_busy.get() title=ctx_tr("Verifica gli indexer con una ricerca di prova") on:click=move |_| {
                    let results = test_results;
                    let busy = verify_busy;
                    let msg = message;
                    busy.set(true);
                     msg.set(tr(data, "Verifica indexer in corso…"));
                    spawn_local(async move {
                        match get("/api/sources/health").await {
                            Ok(value) => {
                                let items = array(&value, "items");
                                let found: Vec<Value> = items.into_iter().filter(|item| text(item, "kind", "") == "indexer").collect();
                                let ok_count = found.iter().filter(|item| item.get("ok").and_then(Value::as_bool).unwrap_or(false)).count();
                                if found.is_empty() {
                                     msg.set(tr(data, "Nessun indexer configurato o attivo da verificare"));
                                } else {
                                     msg.set(tr_format(data, "Verifica completata: {ok_count}/{total} indexer ok", &[("{ok_count}", ok_count.to_string()), ("{total}", found.len().to_string())]));
                                }
                                results.set(found);
                            }
                             Err(error) => msg.set(tr_format(data, "Verifica fallita: {error}", &[("{error}", error)])),
                        }
                        busy.set(false);
                    });
                    }>{move || if verify_busy.get() { tr(data, "Verifica…") } else { tr(data, "Verifica indexer") }}</button>
                <small class="muted">{message}</small>
            </div>
            <Show when=move || !test_results.get().is_empty()>
                <div class="list">
                    {move || test_results.get().iter().cloned().map(|item| {
                        let ok = item.get("ok").and_then(Value::as_bool).unwrap_or(false);
                        view! {
                            <div class="list-item">
                                 <div><strong>{text(&item, "name", "-")}</strong><small>{text(&item, "error", &tr_format(data, "{count} risultati", &[("{count}", number(&item, "results"))]))}</small></div>
                                 <span class="badge" class:ok=ok class:err=move || !ok>{if ok { tr(data, "ok") } else { tr(data, "errore") }}</span>
                            </div>
                        }
                    }).collect_view()}
                </div>
            </Show>
        </div>
    }
}

#[component]
fn FlareSolverrEditor(data: RwSignal<Data>) -> impl IntoView {
    let draft = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    Effect::new(move |_| {
        let value = raw(&data.get().config, "flaresolverr_url", "");
        draft.set(value);
    });
    view! {
        <form class="settings-row" title=ctx_tr("URL del servizio FlareSolverr (es. http://127.0.0.1:8191)") on:submit=move |event| {
            event.prevent_default();
            let url = draft.get();
            let message = message;
            spawn_local(async move {
                match send("POST", "/api/config/settings", Some(json!({"key": "flaresolverr_url", "value": url}))).await {
                     Ok(_) => { trigger_refresh(); message.set(tr(data, "Salvato")); }
                    Err(error) => message.set(error),
                }
            });
        }>
            <label>{ctx_tr("URL FlareSolverr")}</label>
            <input prop:value=draft on:input=move |event| draft.set(event_target_value(&event)) placeholder=ctx_tr("http://127.0.0.1:8191") />
            <div class="form-actions"><button class="btn sm primary">{ctx_tr("Salva")}</button></div>
        </form>
        <div class="toolbar" style="margin-top:8px">
            <button class="btn sm" disabled=move || busy.get() title=ctx_tr("Verifica che FlareSolverr sia raggiungibile") on:click=move |_| {
                let busy = busy;
                let message = message;
                busy.set(true);
                 message.set(tr(data, "Test FlareSolverr in corso…"));
                spawn_local(async move {
                    match send("POST", "/api/flaresolverr/test", None).await {
                        Ok(value) => {
                            let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
                            let sessions = value.get("sessions").and_then(Value::as_array).map(|items| items.len()).unwrap_or(0);
                            if ok {
                                 message.set(tr_format(data, "FlareSolverr raggiungibile ({sessions} sessioni)", &[("{sessions}", sessions.to_string())]));
                            } else {
                                 message.set(tr_format(data, "FlareSolverr risponde ma con stato: {status}", &[("{status}", text(&value, "status", "sconosciuto"))]));
                            }
                        }
                         Err(error) => message.set(tr_format(data, "FlareSolverr non raggiungibile: {error}", &[("{error}", error)])),
                    }
                    busy.set(false);
                });
             }>{move || if busy.get() { tr(data, "Test…") } else { tr(data, "Testa FlareSolverr") }}</button>
            <small class="muted">{message}</small>
        </div>
    }
}

#[component]
fn SourceFilterEditor(data: RwSignal<Data>) -> impl IntoView {
    let source = RwSignal::new(String::new());
    let keywords = RwSignal::new(String::new());
    view! {
        <div class="stack" style="margin-top:10px">
            <div class="toolbar">
                {move || array(&data.get().config, "source_filters").into_iter().map(|item| {
                    let name = text(&item, "source", "");
                    let words = item.get("keywords").and_then(Value::as_array)
                        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
                        .unwrap_or_default();
                    let remove_name = name.clone();
                    let words_title = words.clone();
                    view! {
                        <span class="badge" style="margin-right:6px" title=words_title>
                            {name.clone()} " · " {words}
                            <button class="btn sm" style="margin-left:6px" on:click=move |_| {
                                let mut list = array(&data.get().config, "source_filters");
                                list.retain(|entry| text(entry, "source", "") != remove_name);
                                run_post(data, "/api/config/source-filters", Some(json!({"filters": list})), "Filtro sorgente rimosso");
                            }>"×"</button>
                        </span>
                    }
                }).collect_view()}
            </div>
            <div class="toolbar">
                <input prop:value=source on:input=move |event| source.set(event_target_value(&event)) placeholder=ctx_tr("Sorgente (es. ExtTo - MIRCrewRS, jackett, knaben)") />
                <input prop:value=keywords on:input=move |event| keywords.set(event_target_value(&event)) placeholder=ctx_tr("Parole chiave separate da virgola (es. x265, cam)") />
                <button class="btn sm" on:click=move |_| {
                    let source_value = source.get().trim().to_string();
                    let words = keywords.get().split(',').map(str::trim).filter(|word| !word.is_empty()).map(str::to_owned).collect::<Vec<_>>();
                    if source_value.is_empty() || words.is_empty() { return; }
                    let mut list = array(&data.get().config, "source_filters");
                    list.retain(|entry| text(entry, "source", "") != source_value);
                    list.push(json!({"source": source_value, "keywords": words, "enabled": true}));
                    source.set(String::new());
                    keywords.set(String::new());
                    run_post(data, "/api/config/source-filters", Some(json!({"filters": list})), "Filtro sorgente aggiunto");
                }>{ctx_tr("Aggiungi")}</button>
            </div>
        </div>
    }
}

#[component]
fn ContentFilterEditor(data: RwSignal<Data>) -> impl IntoView {
    let custom = RwSignal::new(String::new());
    view! {
        <div class="stack" style="margin-top:10px">
            <div class="toolbar">
                {CONTENT_FILTER_OPTIONS.iter().map(|(id, label)| {
                    let id = *id;
                    let checked = move || array(&data.get().config, "content_filters").iter().any(|value| value.as_str() == Some(id));
                    view! {
                        <label class="check" title=ctx_tr("Escludi le release che contengono questo filtro")>
                            <input type="checkbox" prop:checked=checked on:change=move |_| {
                                let mut filters = array(&data.get().config, "content_filters").iter().filter_map(|value| value.as_str().map(str::to_owned)).collect::<Vec<_>>();
                                if filters.iter().any(|value| value == id) { filters.retain(|value| value != id); } else { filters.push(id.to_string()); }
                                run_post(data, "/api/config/settings", Some(json!({"key": "content_filters", "value": serde_json::to_string(&filters).unwrap_or_else(|_| "[]".into())})), "Filtri aggiornati");
                            } />
                             <span>{ctx_tr(label)}</span>
                        </label>
                    }
                }).collect_view()}
            </div>
            <div class="toolbar">
                <input prop:value=custom title=ctx_tr("Aggiungi una parola o uno script da escludere dalle release") on:input=move |event| custom.set(event_target_value(&event)) placeholder=ctx_tr("Filtro personalizzato") />
                <button class="btn sm" on:click=move |_| {
                    let value = custom.get().trim().to_string();
                    if value.is_empty() { return; }
                    let mut filters = array(&data.get().config, "content_filters").iter().filter_map(|item| item.as_str().map(str::to_owned)).collect::<Vec<_>>();
                    if !filters.iter().any(|item| item.eq_ignore_ascii_case(&value)) { filters.push(value); }
                    custom.set(String::new());
                    run_post(data, "/api/config/settings", Some(json!({"key": "content_filters", "value": serde_json::to_string(&filters).unwrap_or_else(|_| "[]".into())})), "Filtro aggiunto");
                }>{ctx_tr("Aggiungi")}</button>
            </div>
        </div>
    }
}

#[component]
fn IpFilterControl(data: RwSignal<Data>) -> impl IntoView {
    let status = RwSignal::new(Value::Null);
    let message = RwSignal::new(String::new());
    Effect::new(move |_| {
        let status = status;
        spawn_local(async move {
            if let Ok(value) = get("/api/torrents/ipfilter_status").await {
                status.set(value);
            }
        });
    });
    let refresh = move || {
        let status = status;
        spawn_local(async move {
            if let Ok(value) = get("/api/torrents/ipfilter_status").await {
                status.set(value);
            }
        });
    };
    view! {
        <div class="stack">
            <div class="row">
                <span class="muted">{ctx_tr("Stato IP filter")}</span>
                <strong>{move || {
                    let value = status.get();
                    if value.get("active").and_then(Value::as_bool).unwrap_or(false) {
                        tr_format(data, "{count} regole attive", &[("{count}", number(&value, "rules"))])
                    } else if value.get("configured").and_then(Value::as_bool).unwrap_or(false) {
                         tr(data, "configurato, non caricato")
                    } else {
                         tr(data, "non configurato")
                    }
                }}</strong>
            </div>
            <div class="toolbar">
                <button class="btn sm" title=ctx_tr("Scarica o ricarica la lista IP filter e applicala alla sessione") on:click=move |_| {
                    let message = message;
                    spawn_local(async move {
                        match send("POST", "/api/torrents/ipfilter_update", None).await {
                             Ok(value) => message.set(tr_format(data, "Caricate {count} regole", &[("{count}", number(&value, "rules"))])),
                            Err(error) => message.set(error),
                        }
                        refresh();
                    });
                }>{ctx_tr("Aggiorna IP filter")}</button>
                <small class="muted">{message}</small>
            </div>
        </div>
    }
}

#[component]
fn DbPruneTool(data: RwSignal<Data>) -> impl IntoView {
    let keyword = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    let seen_days = RwSignal::new(String::new());
    let preview_items = RwSignal::new(Vec::<Value>::new());
    let preview_busy = RwSignal::new(false);
    let terms = move || {
        keyword
            .get()
            .split([',', ';', '|', '\n'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    view! {
        <Panel title="Pulizia database per parola chiave">
            <p class="muted">{ctx_tr("Inserisci più parole separate da virgole: vengono cercate in OR. Operazione irreversibile: usa prima Anteprima.")}</p>
            <div class="toolbar" style="margin-top:10px">
                <input prop:value=keyword title=ctx_tr("Parole separate da virgole, cercate in OR nei titoli di serie, film, episodi e torrent") on:input=move |event| keyword.set(event_target_value(&event)) placeholder=ctx_tr("porn, xxx, tits, dick, sex, anal…") />
                <button class="btn sm" disabled=move || preview_busy.get() on:click=move |_| {
                    let values = terms();
                    preview_busy.set(true);
                     message.set(tr(data, "Ricerca in corso…"));
                    spawn_local(async move {
                        match send("POST", "/api/db/prune-keyword", Some(json!({"keywords": values, "preview": true}))).await {
                            Ok(response) => {
                                let items = array(&response, "items");
                                let count = number(&response, "count");
                                preview_items.set(items.clone());
                                 message.set(tr_format(data, "Anteprima: {count} elementi, {shown} mostrati", &[("{count}", count), ("{shown}", items.len().to_string())]));
                            }
                            Err(error) => message.set(error),
                        }
                        preview_busy.set(false);
                    });
                }>{move || if preview_busy.get() { ctx_tr("Cerco…").get() } else { ctx_tr("Anteprima").get() }}</button>
                <button class="btn sm danger" on:click=move |_| {
                    let values = terms();
                    let message = message;
                    spawn_local(async move {
                        match send("POST", "/api/db/prune-keyword", Some(json!({"keywords": values, "preview": false}))).await {
                            Ok(response) => {
                                 message.set(tr_format(data, "Rimossi {count} elementi", &[("{count}", number(&response, "removed"))]));
                                preview_items.set(Vec::new());
                            }
                            Err(error) => message.set(error),
                        }
                    });
                }>{ctx_tr("Elimina")}</button>
                <small class="muted">{message}</small>
            </div>
            <div class="toolbar" style="margin-top:8px">
                <span class="muted">{ctx_tr("Filtri rapidi:")}</span>
                <button type="button" class="btn sm" on:click=move |_| keyword.set("porn, xxx, tits, dick, sex, anal, whore, hentai, milf, blowjob, pussy, cum, fuck, nude, nsfw, adult, gangbang, deepthroat, bdsm, fetish".into())>{ctx_tr("Porn / adult (ampio)")}</button>
                <button type="button" class="btn sm" on:click=move |_| keyword.set("porn, xxx, hentai, milf, blowjob, pussy, cum, gangbang".into())>{ctx_tr("Porn essenziale")}</button>
                <button type="button" class="btn sm" on:click=move |_| keyword.set("script".into())>{ctx_tr("Script")}</button>
                <button type="button" class="btn sm" on:click=move |_| keyword.set("sample".into())>{ctx_tr("Sample")}</button>
                <button type="button" class="btn sm" on:click=move |_| keyword.set("trailer".into())>{ctx_tr("Trailer")}</button>
            </div>
            <p class="muted">{ctx_tr("Le parole vengono combinate in OR. Premi sempre prima Anteprima per vedere quanti elementi verrebbero rimossi. La pulizia copre anche le release 'viste nei feed'.")}</p>
            <div class="toolbar" style="margin-top:12px">
                <span class="muted" title=ctx_tr("Elimina dal database le release rilevate nei feed di film e serie con found_at più vecchio del numero di giorni indicato. Non elimina file, torrent, serie o film monitorati. Il pulsante esegue anche la pulizia standard: conserva gli ultimi 50 cicli e rimuove gli errori torrent più vecchi di 7 giorni.")>{ctx_tr("Release 'viste nei feed' più vecchie di (giorni):")}</span>
                <input style="width:90px" prop:value=seen_days on:input=move |event| seen_days.set(event_target_value(&event)) placeholder="0" title=ctx_tr("Età in giorni oltre la quale eliminare le release viste nei feed; 0 disabilita questa parte della pulizia") />
                <button class="btn sm danger" title=ctx_tr("Elimina le release viste nei feed oltre l'età indicata e applica anche la pulizia standard di cicli ed errori") on:click=move |_| {
                    let days = seen_days.get().trim().parse::<i64>().unwrap_or(0);
                    run_post(data, "/api/db/prune", Some(json!({"retain_cycles": 50, "error_age_days": 7, "seen_retention_days": days})), "Pulizia visti completata");
                }>{ctx_tr("Pulisci visti")}</button>
                <small class="muted">{ctx_tr("0 = conserva tutte le release viste. Non vengono toccati file o download; la pulizia standard conserva 50 cicli e 7 giorni di errori.")}</small>
            </div>
            <Show when=move || !preview_items.get().is_empty()>
                <div class="table-wrap" style="margin-top:10px">
                    <table class="data-table">
                        <thead><tr><th>{ctx_tr("Fonte")}</th><th>{ctx_tr("Titolo")}</th><th>{ctx_tr("Dettaglio")}</th></tr></thead>
                        <tbody>{move || preview_items.get().into_iter().map(|item| {
                            let title = text(&item, "title", "");
                            let title_attr = title.clone();
                            view! {
                                <tr>
                                    <td class="muted">{text(&item, "source", "")}</td>
                                    <td class="truncate" title=title_attr>{title}</td>
                                    <td class="muted truncate">{text(&item, "detail", "")}</td>
                                </tr>
                            }
                        }).collect_view()}</tbody>
                    </table>
                </div>
            </Show>
        </Panel>
    }
}

#[component]
fn DbOptimizeTool(data: RwSignal<Data>) -> impl IntoView {
    let result = RwSignal::new(Value::Null);
    let busy = RwSignal::new(String::new());
    let db_files = RwSignal::new(Vec::<Value>::new());
    let refresh_files = move || {
        let db_files = db_files;
        spawn_local(async move {
            if let Ok(value) = get("/api/db/info").await {
                db_files.set(array(&value, "files"));
            }
        });
    };
    refresh_files();
    let run = move |action: &'static str| {
        let result = result;
        let busy = busy;
        let db_files = db_files;
        busy.set(action.to_string());
        spawn_local(async move {
            match send("POST", "/api/db/action", Some(json!({"action": action}))).await {
                Ok(value) => {
                    result.set(value);
                    if let Ok(info) = get("/api/db/info").await {
                        db_files.set(array(&info, "files"));
                    }
                     push_toast(data, "ok", tr(data, "Fatto"));
                }
                Err(error) => push_toast(data, "err", error),
            }
            busy.set(String::new());
        });
    };
    view! {
        <Panel title="Ottimizzazione database">
            <p class="muted">{ctx_tr("VACUUM compatta i database; ANALYZE aggiorna le statistiche SQLite.")}</p>
            <div class="list" style="margin:10px 0">
                {move || db_files.get().into_iter().map(|file| view! {
                    <div class="list-item"><span class="mono truncate">{text(&file, "name", "-")}</span><strong>{size(&file, "size_bytes")}</strong></div>
                }).collect_view()}
            </div>
            <div class="toolbar" style="margin-top:10px">
                 <button class="btn" disabled=move || !busy.get().is_empty() title=ctx_tr("Compatta il database (VACUUM) e libera spazio") on:click=move |_| run("vacuum")>{move || if busy.get() == "vacuum" { tr(data, "VACUUM…") } else { tr(data, "VACUUM") }}</button>
                 <button class="btn" disabled=move || !busy.get().is_empty() title=ctx_tr("Aggiorna le statistiche del query planner (ANALYZE)") on:click=move |_| run("analyze")>{move || if busy.get() == "analyze" { tr(data, "ANALYZE…") } else { tr(data, "ANALYZE") }}</button>
                <button class="btn sm" disabled=move || !busy.get().is_empty() on:click=move |_| refresh_files()>{ctx_tr("Aggiorna dimensioni")}</button>
                <Show when=move || !busy.get().is_empty()><small class="muted">{ctx_tr("Operazione in corso…")}</small></Show>
            </div>
            <Show when=move || !result.get().is_null()>
                <div class="optimize-result" style="margin-top:12px">
                    <StatLine label="Operazione" value=Signal::derive(move || text(&result.get(), "action", "-")) />
                    <StatLine label="Dimensione prima" value=Signal::derive(move || size(&result.get().get("before").cloned().unwrap_or_default(), "size_bytes")) />
                    <StatLine label="Dimensione dopo" value=Signal::derive(move || size(&result.get().get("after").cloned().unwrap_or_default(), "size_bytes")) />
                    <StatLine label="Spazio liberato" value=Signal::derive(move || {
                        let before = result.get().get("before").and_then(|value| value.get("size_bytes")).and_then(Value::as_f64).unwrap_or(0.0);
                        let after = result.get().get("after").and_then(|value| value.get("size_bytes")).and_then(Value::as_f64).unwrap_or(0.0);
                        size_str((before - after).max(0.0))
                    }) />
                    <StatLine label="Righe prima" value=Signal::derive(move || number(&result.get().get("before").cloned().unwrap_or_default(), "rows")) />
                    <StatLine label="Righe dopo" value=Signal::derive(move || number(&result.get().get("after").cloned().unwrap_or_default(), "rows")) />
                </div>
            </Show>
        </Panel>
    }
}

#[component]
fn BackupSettings(data: RwSignal<Data>) -> impl IntoView {
    let retention = RwSignal::new("5".to_string());
    let schedule = RwSignal::new("0".to_string());
    let schedule_at = RwSignal::new(String::new());
    let telegram = RwSignal::new(false);
    let ftp_host = RwSignal::new(String::new());
    let ftp_user = RwSignal::new(String::new());
    let ftp_password = RwSignal::new(String::new());
    let ftp_path = RwSignal::new(String::new());
    let cloud_dir = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    let ftp_result = RwSignal::new(String::new());
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(value) = get("/api/backup/settings").await {
                retention.set(text(&value, "retention", "5"));
                schedule.set(text(&value, "schedule_hours", "0"));
                schedule_at.set(text(&value, "schedule_at", ""));
                telegram.set(value.get("send_telegram").and_then(Value::as_bool).unwrap_or(false));
                ftp_host.set(text(&value, "ftp_host", ""));
                ftp_user.set(text(&value, "ftp_user", ""));
                ftp_path.set(text(&value, "ftp_path", ""));
                cloud_dir.set(text(&value, "cloud_dir", ""));
            }
        });
    });
    view! {
        <Panel title="Impostazioni backup">
            <div class="form-grid">
                <label class="field" title=ctx_tr("Quanti backup conservare")><span>{ctx_tr("Retention (numero backup)")}</span><input prop:value=retention on:input=move |event| retention.set(event_target_value(&event)) placeholder=ctx_tr("5") /></label>
                <label class="field" title=ctx_tr("0 = solo manuale. Altrimenti backup automatico ogni N ore. Disattivato quando è impostato l'orario giornaliero.")><span>{ctx_tr("Backup automatico (ore, 0 = manuale)")}</span><input prop:value=schedule disabled=move || !schedule_at.get().trim().is_empty() on:input=move |event| schedule.set(event_target_value(&event)) placeholder=ctx_tr("0") /><small class="muted">{move || if schedule_at.get().trim().is_empty() { String::new() } else { tr(data, "Disattivato: è attivo l'orario giornaliero") }}</small></label>
                <label class="field" title=ctx_tr("Orario fisso giornaliero (HH:MM, ora locale). Se compilato disattiva l'intervallo in ore.")><span>{ctx_tr("Backup giornaliero alle (HH:MM, vuoto = disattivo)")}</span><input prop:value=schedule_at on:input=move |event| schedule_at.set(event_target_value(&event)) placeholder=ctx_tr("03:00") /></label>
                <div class="field span-full" title=ctx_tr("Scelte rapide per la frequenza del backup automatico")>
                    <span>{ctx_tr("Frequenza rapida")}</span>
                    <div class="toolbar">
                        <button type="button" class="btn sm" on:click=move |_| { schedule.set("0".into()); schedule_at.set(String::new()); }>{ctx_tr("Manuale")}</button>
                        <button type="button" class="btn sm" disabled=move || !schedule_at.get().trim().is_empty() on:click=move |_| { schedule.set("24".into()); schedule_at.set(String::new()); }>{ctx_tr("Ogni 24h")}</button>
                        <button type="button" class="btn sm" disabled=move || !schedule_at.get().trim().is_empty() on:click=move |_| { schedule.set("168".into()); schedule_at.set(String::new()); }>{ctx_tr("Settimanale (168h)")}</button>
                        <button type="button" class="btn sm" on:click=move |_| { schedule_at.set("03:00".into()); schedule.set("0".into()); }>{ctx_tr("Ogni giorno alle 03:00")}</button>
                    </div>
                </div>
                <label class="field" title=ctx_tr("FTP host")><span>{ctx_tr("FTP host")}</span><input prop:value=ftp_host on:input=move |event| ftp_host.set(event_target_value(&event)) placeholder=ctx_tr("ftp.example.com") /></label>
                <label class="field" title=ctx_tr("FTP utente")><span>{ctx_tr("FTP utente")}</span><input prop:value=ftp_user on:input=move |event| ftp_user.set(event_target_value(&event)) placeholder=ctx_tr("utente") /></label>
                <label class="field" title=ctx_tr("FTP password (lascia vuoto per non cambiarla)")><span>{ctx_tr("FTP password (lascia vuoto per non cambiarla)")}</span><input type="password" prop:value=ftp_password on:input=move |event| ftp_password.set(event_target_value(&event)) /></label>
                <label class="field" title=ctx_tr("FTP percorso")><span>{ctx_tr("FTP percorso")}</span><input prop:value=ftp_path on:input=move |event| ftp_path.set(event_target_value(&event)) placeholder=ctx_tr("/backup") /></label>
                <div class="field span-full" title=ctx_tr("Verifica la connessione FTP con i valori inseriti (usa la password salvata se il campo è vuoto)")>
                    <span>{ctx_tr("Verifica connessione FTP")}</span>
                    <div class="toolbar">
                        <button type="button" class="btn" on:click=move |_| {
                            let body = json!({
                                "host": ftp_host.get(),
                                "user": ftp_user.get(),
                                "password": ftp_password.get(),
                                "path": ftp_path.get(),
                            });
                            let message = message;
                            let ftp_result = ftp_result;
                            spawn_local(async move {
                                 let yesno = |value: &Value| if value.as_bool().unwrap_or(false) { tr(data, "sì") } else { tr(data, "no") };
                                match send("POST", "/api/backup/test-ftp", Some(body)).await {
                                    Ok(value) => {
                                        let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
                                        let error = text_tr(data, &value, "error", "nessun errore");
                                        let summary = tr_format(
                                            data,
                                            "Host: {host}\nUtente: {user}\nPercorso: {path}\nConnessione: {connected}\nLogin: {logged_in}\nCartella remota: {entered_path}\nUpload file di prova: {uploaded}\nRimozione file di prova: {deleted}\nFile di prova: {test_file}\nEsito: {result}",
                                            &[
                                                ("{host}", text(&value, "host", "-")),
                                                ("{user}", text(&value, "user", "-")),
                                                ("{path}", {
                                                    let path = text(&value, "path", "");
                                                    if path.is_empty() { tr(data, "(radice)") } else { path }
                                                }),
                                                ("{connected}", yesno(&value.get("connected").cloned().unwrap_or_default())),
                                                ("{logged_in}", yesno(&value.get("logged_in").cloned().unwrap_or_default())),
                                                ("{entered_path}", yesno(&value.get("entered_path").cloned().unwrap_or_default())),
                                                ("{uploaded}", yesno(&value.get("uploaded").cloned().unwrap_or_default())),
                                                ("{deleted}", yesno(&value.get("deleted").cloned().unwrap_or_default())),
                                                ("{test_file}", text(&value, "test_file", "-")),
                                                ("{result}", if ok { tr(data, "OK") } else { error.clone() }),
                                            ],
                                        );
                                        ftp_result.set(summary);
                                         message.set(if ok { tr(data, "FTP: test riuscito") } else { tr_format(data, "FTP: {error}", &[("{error}", error.clone())]) });
                                        if ok {
                                             push_toast(data, "ok", tr(data, "FTP: test riuscito (file di prova caricato e rimosso)"));
                                        } else {
                                            push_toast(data, "err", format!("FTP: {error}"));
                                        }
                                    }
                                    Err(error) => {
                                        let text = format!("FTP: {error}");
                                        ftp_result.set(text.clone());
                                        message.set(text.clone());
                                        push_toast(data, "err", text);
                                    }
                                }
                            });
                        }>{ctx_tr("Test FTP")}</button>
                    </div>
                    <Show when=move || !ftp_result.get().is_empty()>
                        <pre class="output" style="margin-top:8px;white-space:pre-wrap">{move || ftp_result.get()}</pre>
                    </Show>
                </div>
                 <div class="field span-full" title=ctx_tr("Copia lo snapshot anche in questa cartella: una directory cloud sincronizzata (OneDrive/Drive/Dropbox) o un mount remoto")>
                     <span>{ctx_tr("Cartella cloud/sync (copia aggiuntiva)")}</span>
                     <div class="path-picker"><input prop:value=cloud_dir on:input=move |event| cloud_dir.set(event_target_value(&event)) placeholder=ctx_tr("/home/user/Cloud/rextto-backup") /><BrowseButton value=cloud_dir /></div>
                 </div>
            </div>
            <div class="form-actions" style="margin-top:12px">
                <label class="check" title=ctx_tr("Invia lo snapshot anche su Telegram")><input type="checkbox" prop:checked=telegram on:change=move |event| telegram.set(event_target_checked(&event)) /> <span>{ctx_tr("Invia su Telegram")}</span></label>
                 <button class="btn primary" on:click=move |_| {
                    let mut body = json!({
                        "backup_retention": retention.get(),
                        "backup_schedule_hours": schedule.get(),
                        "backup_schedule_at": schedule_at.get(),
                        "backup_send_telegram": if telegram.get() { "yes" } else { "no" },
                        "backup_ftp_host": ftp_host.get(),
                        "backup_ftp_user": ftp_user.get(),
                        "backup_ftp_path": ftp_path.get(),
                        "backup_cloud_dir": cloud_dir.get(),
                    });
                    let password = ftp_password.get();
                    if !password.trim().is_empty() {
                        body["backup_ftp_password"] = json!(password);
                    }
                    let message = message;
                    spawn_local(async move {
                        match send("POST", "/api/backup/settings", Some(body)).await {
                             Ok(_) => message.set(tr(data, "Impostazioni backup salvate")),
                            Err(error) => message.set(error),
                        }
                    });
                 }>{ctx_tr("Salva")}</button>
                 <button class="btn" title=ctx_tr("Genera subito un backup dei database e delle impostazioni") on:click=move |_| {
                     spawn_local(async move {
                         match send("POST", "/api/backup", None).await {
                             Ok(_) => {
                                  push_toast(data, "ok", tr(data, "Backup creato"));
                                 trigger_refresh();
                             }
                             Err(error) => push_toast(data, "err", error),
                         }
                     });
                 }>{ctx_tr("Genera backup ora")}</button>
                <small class="muted">{message}</small>
            </div>
        </Panel>
    }
}

const SCORE_GROUPS: &[(&str, &[(&str, &str, &str)])] = &[
    (
        "Risoluzione / Video",
        &[
            ("2160p", "score_res_2160p", "2000"),
            ("1080p", "score_res_1080p", "1000"),
            ("720p", "score_res_720p", "400"),
            ("576p", "score_res_576p", "80"),
        ],
    ),
    (
        "Sorgente",
        &[
            ("BluRay", "score_source_bluray", "300"),
            ("Remux", "score_source_remux", "280"),
            ("WEB-DL", "score_source_webdl", "200"),
            ("WEBRip", "score_source_webrip", "150"),
            ("HDTV", "score_source_hdtv", "50"),
        ],
    ),
    (
        "Codec",
        &[
            ("H.265 / x265", "score_codec_h265", "200"),
            ("H.264 / x264", "score_codec_h264", "50"),
        ],
    ),
    (
        "Audio",
        &[
            ("TrueHD", "score_audio_truehd", "150"),
            ("DTS-HD", "score_audio_dts-hd", "120"),
            ("DTS", "score_audio_dts", "100"),
            ("DDP / EAC3", "score_audio_ddp", "80"),
            ("AC3 / 5.1", "score_audio_ac3", "50"),
            ("AAC", "score_audio_aac", "30"),
            ("MP3", "score_audio_mp3", "10"),
        ],
    ),
    (
        "Bonus",
        &[
            ("Dolby Vision", "score_bonus_dv", "300"),
            ("HDR", "score_bonus_hdr", "100"),
            ("PROPER", "score_bonus_proper", "75"),
            ("REPACK", "score_bonus_repack", "50"),
            ("REAL", "score_bonus_real", "100"),
        ],
    ),
];

#[component]
fn ScoreGroupsEditor(data: RwSignal<Data>) -> impl IntoView {
    let group = RwSignal::new(String::new());
    let value = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    let save = move |name: String, amount: String| {
        if name.trim().is_empty() || amount.trim().is_empty() {
            return;
        }
        let key = format!("score_group_{}", name.trim().to_ascii_lowercase());
        let message = message;
        let label = name.clone();
        spawn_local(async move {
            match send("POST", "/api/config/settings", Some(json!({"key": key, "value": amount}))).await {
                Ok(_) => {
                     message.set(tr_format(data, "Gruppo '{label}' salvato", &[("{label}", label)]));
                    trigger_refresh();
                }
                Err(error) => message.set(error),
            }
        });
    };
    let remove = move |name: String| {
        let key = format!("score_group_{}", name.trim().to_ascii_lowercase());
        let message = message;
        spawn_local(async move {
            match send("DELETE", &format!("/api/config/settings/{}", urlencoding::encode(&key)), None).await {
                Ok(_) => {
                     message.set(tr_format(data, "Gruppo '{name}' rimosso", &[("{name}", name)]));
                    trigger_refresh();
                }
                Err(error) => message.set(error),
            }
        });
    };
    view! {
        <Panel title="Gruppi custom (release group)">
            <p class="muted">{ctx_tr("Punteggio extra assegnato alle release di un determinato gruppo (es. un team di release preferito).")}</p>
            <div class="toolbar" style="margin-top:10px">
                <input prop:value=group placeholder=ctx_tr("Nome gruppo (es. NTb)") title=ctx_tr("Nome del release group") />
                <input prop:value=value placeholder=ctx_tr("Punti (es. 50)") style="width:130px" title=ctx_tr("Punteggio extra (0-1000)") />
                <button type="button" class="btn primary" on:click=move |_| save(group.get(), value.get())>{ctx_tr("Aggiungi / aggiorna")}</button>
                <small class="muted">{message}</small>
            </div>
            <div class="table-wrap" style="margin-top:10px">
                <table class="data-table">
                    <thead><tr><th>{ctx_tr("Gruppo")}</th><th>{ctx_tr("Punti")}</th><th></th></tr></thead>
                    <tbody>
                        {move || {
                            let settings = data.get().config.get("score_settings").and_then(Value::as_object).cloned().unwrap_or_default();
                            let mut rows: Vec<(String, String)> = settings.into_iter()
                                .filter(|(key, _)| key.starts_with("score_group_"))
                                .map(|(key, value)| (key.trim_start_matches("score_group_").to_string(), value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())))
                                .collect();
                            rows.sort();
                            rows.into_iter().map(|(name, current)| {
                                 let edit_value = RwSignal::new(current);
                                 let save_name = name.clone();
                                 let remove_name = name.clone();
                                 view! {
                                     <tr>
                                         <td class="mono">{name}</td>
                                         <td><input style="width:130px" prop:value=edit_value on:input=move |event| edit_value.set(event_target_value(&event)) /></td>
                                         <td><div class="toolbar"><button type="button" class="btn sm" on:click=move |_| save(save_name.clone(), edit_value.get())>{ctx_tr("Salva")}</button><button type="button" class="btn sm danger" title=ctx_tr("Rimuovi questo gruppo custom") on:click=move |_| remove(remove_name.clone())>{ctx_tr("Rimuovi")}</button></div></td>
                                     </tr>
                                }
                            }).collect_view()
                        }}
                    </tbody>
                </table>
            </div>
        </Panel>
    }
}

#[component]
fn ScoreSimulator(data: RwSignal<Data>) -> impl IntoView {
    let title = RwSignal::new(String::new());
    let result = RwSignal::new(Value::Null);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let run = move || {
        let query = title.get();
        if query.trim().is_empty() {
            return;
        }
        busy.set(true);
        error.set(String::new());
        spawn_local(async move {
            match send("POST", "/api/score/preview", Some(json!({"title": query}))).await {
                Ok(value) => result.set(value),
                Err(message) => {
                    result.set(Value::Null);
                    error.set(message);
                }
            }
            busy.set(false);
        });
    };
    view! {
        <Panel title="Simulatore punteggio">
            <p class="muted">{ctx_tr("Incolla il titolo di una release per vedere come viene analizzato e quanto vale con i pesi attuali.")}</p>
            <form class="search-row" on:submit=move |event| { event.prevent_default(); run(); }>
                <input prop:value=title on:input=move |event| title.set(event_target_value(&event)) placeholder=ctx_tr("es. Example.Show.S01E01.1080p.WEB-DL.H264.ITA") title=ctx_tr("Titolo della release da analizzare") />
                 <button class="btn primary" disabled=move || busy.get()>{move || if busy.get() { tr(data, "Calcolo…") } else { tr(data, "Calcola") }}</button>
            </form>
            <Show when=move || !error.get().is_empty()>
                <div class="alert" style="margin-top:10px">{move || error.get()}</div>
            </Show>
            <Show when=move || !result.get().is_null()>
                <div class="stack" style="margin-top:12px">
                    <div class="metrics">
                        <Metric label="Punteggio (impostazioni)" value=Signal::derive(move || number(&result.get(), "score")) tone="mint" />
                        <Metric label="Punteggio base" value=Signal::derive(move || number(&result.get(), "base_score")) tone="blue" />
                        <Metric label="Tipo" value=Signal::derive(move || text(&result.get(), "kind", "-")) tone="amber" />
                         <Metric label="Accettata" value=Signal::derive(move || if result.get().get("allowed").and_then(Value::as_bool).unwrap_or(false) { tr(data, "sì") } else { tr(data, "no") }) tone="violet" />
                    </div>
                    <div class="grid-2">
                        <div class="table-wrap">
                            <table class="data-table">
                                <thead><tr><th>{ctx_tr("Campo")}</th><th>{ctx_tr("Valore")}</th></tr></thead>
                                <tbody>
                                    {move || {
                                        let value = result.get();
                                        let quality = value.get("quality").cloned().unwrap_or_default();
                                        let rows = vec![
                                            ("Risoluzione", text(&quality, "resolution", "-")),
                                            ("Sorgente", text(&quality, "source", "-")),
                                            ("Codec", text(&quality, "codec", "-")),
                                            ("Audio", text(&quality, "audio", "-")),
                                            ("HDR", text(&quality, "hdr", "-")),
                                            ("Gruppo", text(&quality, "group", "-")),
                                            ("Lingue", { let langs = array(&quality, "languages"); if langs.is_empty() { "-".to_string() } else { langs.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>().join(", ") } }),
                                            ("Stagione/Episodio", format!("{} / {}", number(&value, "season"), number(&value, "episode"))),
                                            ("Pacchetto stagione", if value.get("is_pack").and_then(Value::as_bool).unwrap_or(false) { "sì".into() } else { "no".into() }),
                                            ("Serie corrispondente", text(&value, "matched_series", "-")),
                                            ("Film corrispondente", text(&value, "matched_movie", "-")),
                                        ];
                                        rows.into_iter().map(|(label, value)| view! {
                                            <tr><td class="muted">{label}</td><td>{value}</td></tr>
                                        }).collect_view()
                                    }}
                                </tbody>
                            </table>
                        </div>
                        <div class="table-wrap">
                            <table class="data-table">
                                <thead><tr><th>{ctx_tr("Componente")}</th><th>{ctx_tr("Punti")}</th></tr></thead>
                                <tbody>
                                    {move || array(&result.get(), "breakdown").into_iter().map(|entry| view! {
                                        <tr><td class="muted">{text(&entry, "label", "-")}</td><td class="numeric">{number(&entry, "value")}</td></tr>
                                    }).collect_view()}
                                </tbody>
                            </table>
                        </div>
                    </div>
                </div>
            </Show>
        </Panel>
    }
}

#[component]
fn ScoreEditor(data: RwSignal<Data>) -> impl IntoView {
    view! {
        <div class="stack">
            {SCORE_GROUPS.iter().map(|(group, fields)| view! {
                <div>
                     <div class="field span-full" style="margin:10px 0 6px"><span>{ctx_tr(group)}</span></div>
                    <div class="form-grid">
                        {fields.iter().map(|(label, key, default)| {
                            let value = Signal::derive(move || text(&data.get().config.get("score_settings").cloned().unwrap_or_default(), key, default));
                            view! { <ScoreField label=*label setting_key=*key value=value placeholder=*default /> }
                        }).collect_view()}
                    </div>
                </div>
            }).collect_view()}
        </div>
    }
}

#[component]
fn ScoreField(
    label: &'static str,
    setting_key: &'static str,
    value: Signal<String>,
    placeholder: &'static str,
) -> impl IntoView {
    let draft = RwSignal::new(value.get());
    let message = RwSignal::new(String::new());
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let dirty = use_context::<DirtySettings>();
    Effect::new(move |_| draft.set(value.get()));
    view! {
        <label class="field" id=format!("setting-{setting_key}") title=ctx_tr(score_tooltip(setting_key))>
            <span>{ctx_tr(label)}</span>
            <div class="path-picker">
                <input prop:value=draft on:input=move |event| {
                    let next = event_target_value(&event);
                    draft.set(next.clone());
                    if let Some(dirty) = dirty {
                        dirty.items.update(|items| { items.insert(setting_key.to_string(), next.clone()); });
                    }
                } placeholder=ctx_tr(placeholder) />
                <button type="button" class="btn sm primary" on:click=move |_| {
                    let draft_value = draft.get();
                    let message = message;
                    spawn_local(async move {
                        match send("POST", "/api/config/settings", Some(json!({"key": setting_key, "value": draft_value}))).await {
                            Ok(_) => {
                                 message.set(tr(data, "Salvato"));
                                if let Some(dirty) = dirty {
                                    dirty.items.update(|items| { items.remove(setting_key); });
                                }
                            }
                            Err(error) => message.set(error),
                        }
                    });
                }>{ctx_tr("Salva")}</button>
            </div>
            <small class="muted">{message}</small>
        </label>
    }
}

#[component]
fn TranslationTools() -> impl IntoView {
    let data = use_context::<RwSignal<Data>>().expect("data context");
    let lang = RwSignal::new("it".to_string());
    let yaml = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    view! {
        <Panel title="Importa/esporta traduzioni interfaccia">
            <p class="hint">{ctx_tr("Questo pannello serve a fare una copia o una modifica avanzata delle traduzioni dell'interfaccia. Scegli una lingua e premi Esporta per caricare nell'editor le coppie chiave/valore salvate; modifica il testo YAML e premi Importa per aggiornare o aggiungere le traduzioni. Le chiavi non presenti nel file importato non vengono cancellate. Esempio: \"Testa porte\": \"Test ports\". Dopo l'importazione l'interfaccia si aggiorna automaticamente se hai importato la lingua attiva; altrimenti selezionala dal menu in alto.")}</p>
            <div class="toolbar">
                <label>{ctx_tr("Lingua delle traduzioni")}</label>
                <select prop:value=lang on:change=move |event| lang.set(event_target_value(&event))>
                    <option value="it">{ctx_tr("Italiano")}</option>
                    <option value="en">{ctx_tr("English")}</option>
                </select>
                <button class="btn sm" title=ctx_tr("Carica nell'editor le traduzioni salvate per la lingua scelta") on:click=move |_| { let selected = lang.get(); let output = yaml; let status = message; spawn_local(async move { match get_text(&format!("/api/i18n/export/{selected}")).await { Ok(value) => { output.set(value); status.set(tr(data, "Traduzioni esportate")); } Err(error) => status.set(error) } }); }>{ctx_tr("Esporta YAML")}</button>
                <button class="btn sm" title=ctx_tr("Aggiorna o aggiunge le traduzioni della lingua scelta usando il testo YAML nell'editor; la lingua attiva si aggiorna automaticamente se coincide con quella scelta") on:click=move |_| { let selected = lang.get(); let value = yaml.get(); let status = message; spawn_local(async move { match send("POST", &format!("/api/i18n/import/{selected}"), Some(json!({"yaml": value}))).await { Ok(_) => { let active = data.get().language; if active == selected { status.set(tr(data, "Traduzioni importate; interfaccia aggiornata")); trigger_refresh(); } else { status.set(format!("{}: {}. {}", tr(data, "Traduzioni importate per"), selected, tr(data, "seleziona questa lingua nel menu in alto per visualizzarle"))); } }, Err(error) => status.set(error) } }); }>{ctx_tr("Importa YAML")}</button>
                <small class="muted">{message}</small>
            </div>
            <textarea style="width:100%;min-height:220px;margin-top:10px" prop:value=yaml on:input=move |event| yaml.set(event_target_value(&event)) placeholder=ctx_tr("Incolla qui le traduzioni nel formato YAML chiave: valore")></textarea>
        </Panel>
    }
}

/* ------------------------------------------------------------------ */
/* Integrations                                                        */
/* ------------------------------------------------------------------ */

#[component]
fn IntegrationsView(data: RwSignal<Data>) -> impl IntoView {
    let trakt_id = RwSignal::new(String::new());
    let trakt_secret = RwSignal::new(String::new());
    let trakt_configured = RwSignal::new(false);
    let simkl_id = RwSignal::new(String::new());
    let simkl_configured = RwSignal::new(false);
    let trakt_code = RwSignal::new(String::new());
    let simkl_code = RwSignal::new(String::new());
    let trakt_watchlist_sync = RwSignal::new(false);
    let trakt_scrobble_enabled = RwSignal::new(false);
    let trakt_calendar_days = RwSignal::new("7".to_string());
    let simkl_watchlist_status = RwSignal::new("plantowatch".to_string());
    let simkl_calendar_days = RwSignal::new("7".to_string());
    let simkl_mark_watched = RwSignal::new(false);
    let jellyfin_url = RwSignal::new(String::new());
    let jellyfin_key = RwSignal::new(String::new());
    let plex_url = RwSignal::new(String::new());
    let plex_token = RwSignal::new(String::new());
    let sync_message = RwSignal::new(String::new());
    let output = RwSignal::new(String::new());
    let provider = RwSignal::new(Option::<Value>::None);
    Effect::new(move |_| {
        jellyfin_url.set(raw(&data.get().config, "jellyfin_url", ""));
        plex_url.set(raw(&data.get().config, "plex_url", ""));
    });
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(value) = get("/api/trakt/settings").await {
                trakt_id.set(text(&value, "client_id", ""));
                trakt_configured.set(value.get("client_id_configured").and_then(Value::as_bool).unwrap_or(false)
                    && value.get("client_secret_configured").and_then(Value::as_bool).unwrap_or(false));
                trakt_watchlist_sync.set(value.get("watchlist_sync").and_then(Value::as_bool).unwrap_or(false));
                trakt_scrobble_enabled.set(value.get("scrobble_enabled").and_then(Value::as_bool).unwrap_or(false));
                trakt_calendar_days.set(text(&value, "calendar_days", "7"));
            }
            if let Ok(value) = get("/api/simkl/settings").await {
                simkl_id.set(text(&value, "client_id", ""));
                simkl_configured.set(value.get("client_id_configured").and_then(Value::as_bool).unwrap_or(false));
                simkl_watchlist_status.set(text(&value, "watchlist_status", "plantowatch"));
                simkl_calendar_days.set(text(&value, "calendar_days", "7"));
                simkl_mark_watched.set(value.get("mark_watched").and_then(Value::as_bool).unwrap_or(false));
            }
        });
    });
    view! {
        <div class="view">
            <div class="grid-2">
                <Panel title="Trakt">
                    <div class="stack">
                        <div class="mode-banner" class:active=move || data.get().trakt.get("authenticated").and_then(Value::as_bool).unwrap_or(false)>
                             <strong>{move || if data.get().trakt.get("authenticated").and_then(Value::as_bool).unwrap_or(false) { tr(data, "Trakt: autenticato") } else if trakt_configured.get() { tr(data, "Trakt: configurato, da autenticare") } else { tr(data, "Trakt: non configurato") }}</strong>
                             <span>{move || format!("client_id: {} · secret: {}", if trakt_id.get().is_empty() { tr(data, "mancante") } else { tr(data, "salvato") }, if trakt_configured.get() { tr(data, "salvato") } else { tr(data, "mancante") })}</span>
                        </div>
                        <p class="hint">{ctx_tr("Crea un'app su trakt.tv (Settings → Your API Apps), imposta come Redirect URI una qualsiasi (es. urn:ietf:wg:oauth:2.0:oob) e incolla qui client_id e client_secret. Poi premi Avvia accesso.")}</p>
                        <form class="form" on:submit=move |event| { event.prevent_default(); let body = json!({"trakt_client_id": trakt_id.get(), "trakt_client_secret": trakt_secret.get()}); trakt_secret.set(String::new()); run_post(data, "/api/trakt/settings", Some(body), "Credenziali Trakt salvate"); }>
                            <div class="form-grid">
                                <label class="field" title=ctx_tr("Trakt client_id")><span>{ctx_tr("Trakt client_id")}</span><input prop:value=trakt_id on:input=move |event| trakt_id.set(event_target_value(&event)) placeholder=ctx_tr("client id") /></label>
                                <label class="field" title=ctx_tr("Trakt client_secret")><span>{ctx_tr("Trakt client_secret")}</span><input type="password" prop:value=trakt_secret on:input=move |event| trakt_secret.set(event_target_value(&event)) placeholder=ctx_tr("non visualizzato (lascia vuoto per non cambiare)") /></label>
                            </div>
                            <div class="form-actions"><button class="btn sm primary">{ctx_tr("Salva credenziali")}</button></div>
                        </form>
                        <form class="form" on:submit=move |event| { event.prevent_default(); let body = json!({
                            "trakt_watchlist_sync": if trakt_watchlist_sync.get() { "yes" } else { "no" },
                            "trakt_scrobble_enabled": if trakt_scrobble_enabled.get() { "yes" } else { "no" },
                            "trakt_calendar_days": trakt_calendar_days.get(),
                        }); run_post(data, "/api/trakt/settings", Some(body), "Opzioni Trakt salvate"); }>
                            <div class="toolbar">
                                <label class="check" title=ctx_tr("Sincronizza automaticamente la watchlist ad ogni ciclo")><input type="checkbox" prop:checked=trakt_watchlist_sync on:change=move |event| trakt_watchlist_sync.set(event_target_checked(&event)) /> <span>{ctx_tr("Sync watchlist automatico")}</span></label>
                                <label class="check" title=ctx_tr("Invia lo scrobble a Trakt")><input type="checkbox" prop:checked=trakt_scrobble_enabled on:change=move |event| trakt_scrobble_enabled.set(event_target_checked(&event)) /> <span>{ctx_tr("Scrobble attivo")}</span></label>
                                <label class="check" title=ctx_tr("Quanti giorni di calendario Trakt caricare")><span>{ctx_tr("Giorni calendario")}</span><input style="width:80px" prop:value=trakt_calendar_days on:input=move |event| trakt_calendar_days.set(event_target_value(&event)) placeholder=ctx_tr("7") /></label>
                                <button class="btn sm primary">{ctx_tr("Salva opzioni")}</button>
                            </div>
                        </form>
                        <div class="toolbar">
                            <button class="btn sm" on:click=move |_| { let output = output; spawn_local(async move { if let Ok(value) = send("POST", "/api/trakt/auth/start", None).await { output.set(value.to_string()); provider.set(Some(value.clone())); } }); }>{ctx_tr("Avvia accesso")}</button>
                            <input prop:value=trakt_code on:input=move |event| trakt_code.set(event_target_value(&event)) placeholder=ctx_tr("Codice") />
                            <button class="btn sm" on:click=move |_| { let code = trakt_code.get(); let output = output; spawn_local(async move { if let Ok(value) = send("POST", "/api/trakt/auth/poll", Some(json!({"code": code}))).await { output.set(value.to_string()); provider.set(Some(value.clone())); } }); }>{ctx_tr("Conferma")}</button>
                        </div>
                        <div class="toolbar">
                            <button class="btn sm" on:click=move |_| { let output = output; spawn_local(async move { if let Ok(value) = get("/api/trakt/watchlist").await { output.set(value.to_string()); provider.set(Some(value.clone())); } }); }>{ctx_tr("Watchlist")}</button>
                            <button class="btn sm" on:click=move |_| run_post(data, "/api/trakt/watchlist/import", None, "Watchlist Trakt importata")>{ctx_tr("Importa watchlist")}</button>
                            <button class="btn sm" on:click=move |_| { let output = output; spawn_local(async move { if let Ok(value) = get("/api/trakt/calendar").await { output.set(value.to_string()); provider.set(Some(value.clone())); } }); }>{ctx_tr("Calendario")}</button>
                            <button class="btn sm" on:click=move |_| run_post(data, "/api/trakt/auth/refresh", None, "Token aggiornato")>{ctx_tr("Refresh")}</button>
                            <button class="btn sm danger" on:click=move |_| run_post(data, "/api/trakt/auth/revoke", None, "Token revocato")>{ctx_tr("Revoca")}</button>
                        </div>
                    </div>
                </Panel>
                <Panel title="Simkl">
                    <div class="stack">
                             <StatLine label="Stato" value=Signal::derive(move || if data.get().simkl.get("authenticated").and_then(Value::as_bool).unwrap_or(false) { tr(data, "autenticato") } else if data.get().simkl.get("configured").and_then(Value::as_bool).unwrap_or(false) { tr(data, "configurato") } else { tr(data, "non configurato") }) />
                        <p class="hint">{ctx_tr("Autenticazione PIN: crea un'app PIN/device su simkl.com/settings/developer/new, incolla il client_id e premi Salva; poi Avvia PIN, apri simkl.com/pin, inserisci il codice mostrato e premi Conferma. Usa un'app PIN/device (legacy V1): un client OAuth V2 può restituire unauthorized_client.")}</p>
                        <form class="toolbar" on:submit=move |event| { event.prevent_default(); let body = json!({"simkl_client_id": simkl_id.get()}); run_post(data, "/api/simkl/settings", Some(body), "Client ID salvato"); }>
                            <input prop:value=simkl_id on:input=move |event| simkl_id.set(event_target_value(&event)) placeholder=ctx_tr("Simkl client_id") />
                            <button class="btn sm primary">{ctx_tr("Salva")}</button>
                        </form>
                        <form class="form" on:submit=move |event| { event.prevent_default(); let body = json!({
                            "simkl_watchlist_status": simkl_watchlist_status.get(),
                            "simkl_calendar_days": simkl_calendar_days.get(),
                            "simkl_mark_watched": if simkl_mark_watched.get() { "yes" } else { "no" },
                        }); run_post(data, "/api/simkl/settings", Some(body), "Opzioni Simkl salvate"); }>
                            <div class="toolbar">
                                <label class="field" title=ctx_tr("Stato watchlist")><span>{ctx_tr("Stato watchlist")}</span><select prop:value=simkl_watchlist_status on:change=move |event| simkl_watchlist_status.set(event_target_value(&event))>
                                    <option value="plantowatch">{ctx_tr("Da guardare")}</option>
                                    <option value="watching">{ctx_tr("In corso")}</option>
                                    <option value="completed">{ctx_tr("Completati")}</option>
                                    <option value="dropped">{ctx_tr("Abbandonati")}</option>
                                </select></label>
                                <label class="check" title=ctx_tr("Quanti giorni di calendario Simkl caricare")><span>{ctx_tr("Giorni calendario")}</span><input style="width:80px" prop:value=simkl_calendar_days on:input=move |event| simkl_calendar_days.set(event_target_value(&event)) placeholder=ctx_tr("7") /></label>
                                <label class="check" title=ctx_tr("Segna come visti gli episodi scaricati")><input type="checkbox" prop:checked=simkl_mark_watched on:change=move |event| simkl_mark_watched.set(event_target_checked(&event)) /> <span>{ctx_tr("Segna come visti")}</span></label>
                                <button class="btn sm primary">{ctx_tr("Salva opzioni")}</button>
                            </div>
                        </form>
                        <div class="toolbar">
                            <button class="btn sm" on:click=move |_| { let output = output; spawn_local(async move { if let Ok(value) = send("POST", "/api/simkl/auth/start", None).await { output.set(value.to_string()); provider.set(Some(value.clone())); } }); }>{ctx_tr("Avvia PIN")}</button>
                            <input prop:value=simkl_code on:input=move |event| simkl_code.set(event_target_value(&event)) placeholder=ctx_tr("PIN") />
                            <button class="btn sm" on:click=move |_| { let code = simkl_code.get(); let output = output; spawn_local(async move { if let Ok(value) = send("POST", "/api/simkl/auth/poll", Some(json!({"code": code}))).await { output.set(value.to_string()); provider.set(Some(value.clone())); } }); }>{ctx_tr("Conferma")}</button>
                        </div>
                        <div class="toolbar">
                            <button class="btn sm" on:click=move |_| { let output = output; spawn_local(async move { if let Ok(value) = get("/api/simkl/watchlist").await { output.set(value.to_string()); provider.set(Some(value.clone())); } }); }>{ctx_tr("Watchlist")}</button>
                            <button class="btn sm" on:click=move |_| run_post(data, "/api/simkl/watchlist/import", None, "Watchlist Simkl importata")>{ctx_tr("Importa watchlist")}</button>
                            <button class="btn sm" on:click=move |_| { let output = output; spawn_local(async move { if let Ok(value) = get("/api/simkl/calendar").await { output.set(value.to_string()); provider.set(Some(value.clone())); } }); }>{ctx_tr("Calendario")}</button>
                            <button class="btn sm danger" on:click=move |_| run_post(data, "/api/simkl/auth/revoke", None, "Token revocato")>{ctx_tr("Revoca")}</button>
                        </div>
                    </div>
                </Panel>
            </div>
            <Panel title="Jellyfin / Plex">
                <div class="grid-2">
                    <div class="stack">
                         <div class="toolbar"><strong>{ctx_tr("Jellyfin")}</strong><span class="badge" class:ok=move || data.get().config.get("jellyfin_configured").and_then(Value::as_bool).unwrap_or(false)>{move || if data.get().config.get("jellyfin_configured").and_then(Value::as_bool).unwrap_or(false) { tr(data, "configurato") } else { tr(data, "non configurato") }}</span></div>
                        <label class="field" title=ctx_tr("URL del server Jellyfin")><span>{ctx_tr("URL Jellyfin")}</span><input prop:value=jellyfin_url on:input=move |event| jellyfin_url.set(event_target_value(&event)) placeholder=ctx_tr("http://127.0.0.1:8096") /></label>
                        <label class="field" title=ctx_tr("API key di Jellyfin (non visualizzata dopo il salvataggio)")><span>{ctx_tr("API key Jellyfin")}</span><input type="password" prop:value=jellyfin_key on:input=move |event| jellyfin_key.set(event_target_value(&event)) placeholder=ctx_tr("non visualizzata") /></label>
                        <div class="toolbar">
                             <button class="btn sm primary" on:click=move |_| { let url = jellyfin_url.get(); let key = jellyfin_key.get(); let message = sync_message; spawn_local(async move { if !url.trim().is_empty() { let _ = send("POST", "/api/config/settings", Some(json!({"key":"jellyfin_url","value":url}))).await; } if !key.trim().is_empty() { let _ = send("POST", "/api/config/settings", Some(json!({"key":"jellyfin_api_key","value":key}))).await; } message.set(tr(data, "Jellyfin salvato")); trigger_refresh(); }); }>{ctx_tr("Salva")}</button>
                            <button class="btn sm" title=ctx_tr("Forza l'aggiornamento della libreria Jellyfin") on:click=move |_| run_post(data, "/api/jellyfin/refresh", None, "Jellyfin aggiornato")>{ctx_tr("Aggiorna libreria")}</button>
                        </div>
                    </div>
                    <div class="stack">
                         <div class="toolbar"><strong>{ctx_tr("Plex")}</strong><span class="badge" class:ok=move || data.get().config.get("plex_configured").and_then(Value::as_bool).unwrap_or(false)>{move || if data.get().config.get("plex_configured").and_then(Value::as_bool).unwrap_or(false) { tr(data, "configurato") } else { tr(data, "non configurato") }}</span></div>
                        <label class="field" title=ctx_tr("URL del server Plex")><span>{ctx_tr("URL Plex")}</span><input prop:value=plex_url on:input=move |event| plex_url.set(event_target_value(&event)) placeholder=ctx_tr("http://127.0.0.1:32400") /></label>
                        <label class="field" title=ctx_tr("Token Plex (non visualizzato dopo il salvataggio)")><span>{ctx_tr("Token Plex")}</span><input type="password" prop:value=plex_token on:input=move |event| plex_token.set(event_target_value(&event)) placeholder=ctx_tr("non visualizzato") /></label>
                        <div class="toolbar">
                             <button class="btn sm primary" on:click=move |_| { let url = plex_url.get(); let token = plex_token.get(); let message = sync_message; spawn_local(async move { if !url.trim().is_empty() { let _ = send("POST", "/api/config/settings", Some(json!({"key":"plex_url","value":url}))).await; } if !token.trim().is_empty() { let _ = send("POST", "/api/config/settings", Some(json!({"key":"plex_token","value":token}))).await; } message.set(tr(data, "Plex salvato")); trigger_refresh(); }); }>{ctx_tr("Salva")}</button>
                            <button class="btn sm" title=ctx_tr("Forza l'aggiornamento della libreria Plex") on:click=move |_| run_post(data, "/api/plex/refresh", None, "Plex aggiornato")>{ctx_tr("Aggiorna libreria")}</button>
                        </div>
                    </div>
                </div>
                <small class="muted" style="display:block;margin-top:8px">{sync_message}</small>
            </Panel>
            <Panel title="Watchlist e calendario provider">
                {move || match provider.get() {
                    Some(value) => {
                        let rows = provider_rows(&value);
                        if rows.is_empty() {
                            let raw = value.to_string();
                            view! { <pre class="output">{raw}</pre> }.into_any()
                        } else {
                            view! {
                                <div class="table-wrap">
                                    <table class="data-table">
                                        <thead><tr><th>{ctx_tr("Titolo")}</th><th>{ctx_tr("Dettaglio")}</th><th>{ctx_tr("Data / Stato")}</th></tr></thead>
                                        <tbody>
                                            {rows.into_iter().map(|(title, detail, date)| view! {
                                                <tr><td class="truncate">{title}</td><td class="muted">{detail}</td><td class="mono muted">{date}</td></tr>
                                            }).collect_view()}
                                        </tbody>
                                    </table>
                                </div>
                            }.into_any()
                        }
                    }
                    None => view! {
                        <div class="stack">
                            <p class="muted">{ctx_tr("Usa i pulsanti Watchlist / Calendario di Trakt o Simkl per caricare i dati in tabella.")}</p>
                            {move || if output.get().is_empty() { None } else { Some(view! { <pre class="output">{output.get()}</pre> }) }}
                        </div>
                    }.into_any(),
                }}
            </Panel>
            <Panel title="Browser (magnet / .torrent)">
                <p class="muted">{ctx_tr("Handler magnet/.torrent: scarica i file nella stessa cartella ed esegui install.sh (Linux, xdg-utils). Da quel momento ogni click su un link magnet: o file .torrent nel browser invia il download a Rextto.")}</p>
                <div class="toolbar" style="margin-top:10px">
                    <a class="btn sm" href="/api/browser-handlers/download?file=rextto-magnet" download="rextto-magnet">{"rextto-magnet"}</a>
                    <a class="btn sm" href="/api/browser-handlers/download?file=rextto-torrent" download="rextto-torrent">{"rextto-torrent"}</a>
                    <a class="btn sm" href="/api/browser-handlers/download?file=rextto-magnet.desktop" download="rextto-magnet.desktop">{"rextto-magnet.desktop"}</a>
                    <a class="btn sm" href="/api/browser-handlers/download?file=rextto-torrent.desktop" download="rextto-torrent.desktop">{"rextto-torrent.desktop"}</a>
                    <a class="btn sm" href="/api/browser-handlers/download?file=install.sh" download="install.sh">{"install.sh"}</a>
                </div>
                <label class="field span-full" title=ctx_tr("Comando da eseguire nel terminale del tuo PC: scarica e installa gli handler")>
                    <span>{ctx_tr("Comando rapido (scarica e installa)")}</span>
                    <input readonly prop:value=move || {
                        let origin = web_sys::window().and_then(|window| window.location().origin().ok()).unwrap_or_default();
                        format!("cd /tmp && mkdir -p rextto-handlers && cd rextto-handlers && for f in rextto-magnet rextto-torrent rextto-magnet.desktop rextto-torrent.desktop install.sh; do curl -fsSL \"{origin}/api/browser-handlers/download?file=$f\" -o \"$f\"; done && chmod +x rextto-magnet rextto-torrent install.sh && bash install.sh")
                    } />
                </label>
                <div class="toolbar" style="margin-top:10px">
                    <button type="button" class="btn sm" title=ctx_tr("Registra Rextto come gestore dei link magnet nel browser (funziona solo su HTTPS o localhost)") on:click=move |_| {
                        if let Some(window) = web_sys::window() {
                            let origin = window.location().origin().unwrap_or_default();
                            let handler = format!("{origin}/magnet?url=%s");
                            match window.navigator().register_protocol_handler("magnet", &handler, "Rextto") {
                                 Ok(_) => push_toast(data, "info", tr(data, "Registrazione gestore magnet inviata al browser")),
                                 Err(_) => push_toast(data, "err", tr(data, "Il browser non ha accettato la registrazione (serve HTTPS o localhost): usa gli handler xdg qui sopra")),
                            }
                        }
                    }>{ctx_tr("Registra gestore magnet nel browser")}</button>
                </div>
            </Panel>
            <EventHooksPanel />
        </div>
    }
}

/* ------------------------------------------------------------------ */
/* Maintenance                                                         */
/* ------------------------------------------------------------------ */

#[component]
fn MaintenanceView(data: RwSignal<Data>) -> impl IntoView {
    let sources = RwSignal::new(Vec::<Value>::new());
    let ports = RwSignal::new(Value::Null);
    let initial_trash_path = text(
        &data
            .get()
            .config
            .get("paths")
            .cloned()
            .unwrap_or_default(),
        "trash_path",
        "-",
    );
    let trash = RwSignal::new(json!({
        "path": initial_trash_path,
        "exists": false,
        "count": 0,
        "total_bytes": 0,
        "retention_days": 0,
        "items": []
    }));
    let source_query = RwSignal::new("ita 1080p".to_string());
    let retain = RwSignal::new("50".to_string());
    let age = RwSignal::new("7".to_string());
    let duplicates = RwSignal::new(Vec::<Value>::new());
    let duplicates_count = RwSignal::new(0_i64);
    let duplicates_busy = RwSignal::new(false);
    let duplicates_message = RwSignal::new(String::new());
    let rename_status = RwSignal::new(String::new());
    let refresh_trash = move || {
        let trash = trash;
        spawn_local(async move {
            if let Ok(value) = get("/api/trash").await {
                trash.set(value);
            }
        });
    };
    Effect::new(move |_| {
        let sources = sources;
        spawn_local(async move {
            if let Ok(value) = get("/api/sources/health").await {
                sources.set(array(&value, "items"));
            }
        });
        refresh_trash();
    });
    view! {
        <div class="view">
            <Panel title="Azioni">
                <div class="toolbar">
                    <button class="btn primary" title=ctx_tr("Crea subito un backup compresso di database e configurazione") on:click=move |_| run_post(data, "/api/backup", None, "Backup creato")>{ctx_tr("Backup")}</button>
                    <button class="btn" title=ctx_tr("Svuota subito il cestino ignorando la conservazione configurata") on:click=move |_| run_post(data, "/api/maintenance/clean-trash", Some(json!({"force": true})), "Trash pulito")>{ctx_tr("Pulisci trash")}</button>
                    <button class="btn" title=ctx_tr("Ricalcola il punteggio di qualità degli episodi indicizzati con le regole scoring attuali") on:click=move |_| run_post(data, "/api/database/rescore", None, "Scoring ricalcolato")>{ctx_tr("Ricalcola scoring")}</button>
                    <button class="btn" title=ctx_tr("Rileggi le cartelle archivio e registra nel database i file video già presenti") on:click=move |_| run_post(data, "/api/scan-all-archives", None, "Archivi scansionati")>{ctx_tr("Scansiona archivi")}</button>
                    <button class="btn" title=ctx_tr("Analizza con ffprobe i file archiviati senza dati MediaInfo e li salva nel database") on:click=move |_| run_post(data, "/api/maintenance/backfill-media-info", Some(json!({"limit": 200})), "MediaInfo aggiornato")>{ctx_tr("Aggiorna MediaInfo")}</button>
                    <button class="btn" title=ctx_tr("Rinomina in background tutti i file archiviati, con progresso") on:click=move |_| {
                        run_post(data, "/api/rename-all", Some(json!({})), "Rinomina avviata…");
                        spawn_local(async move {
                            loop {
                                gloo_timers::future::TimeoutFuture::new(2000).await;
                                let Ok(value) = get("/api/rename-progress").await else { break; };
                                let progress = value.get("progress").cloned().unwrap_or_default();
                                let running = progress.get("running").and_then(Value::as_bool).unwrap_or(false);
                                let current = progress.get("current").and_then(Value::as_u64).unwrap_or(0);
                                let total = progress.get("total").and_then(Value::as_u64).unwrap_or(0);
                                let series = text(&progress, "series", "");
                                let message = text(&progress, "message", "");
                                rename_status.set(format!("{current}/{total} {series} {message}"));
                                if !running { break; }
                            }
                        });
                    }>{ctx_tr("Rinomina tutto")}</button>
                    <small class="muted">{move || rename_status.get()}</small>
                    <button class="btn" title=ctx_tr("Importa serie, film e storico dai database già presenti nella cartella dati") on:click=move |_| run_post(data, "/api/setup/import", None, "Import eseguito")>{ctx_tr("Importa dati esistenti")}</button>
                    <button class="btn" title=ctx_tr("Riavvia il servizio rextto per applicare gli aggiornamenti (richiede l'helper installato una volta da root)") on:click=move |_| {
                        spawn_local(async move {
                            match send("POST", "/api/service/restart", None).await {
                         Ok(_) => push_toast(data, "ok", tr(data, "Riavvio del servizio richiesto…")),
                                Err(error) => push_toast(data, "err", error),
                            }
                        });
                    }>{ctx_tr("Riavvia servizio")}</button>
                </div>
                <div class="toolbar" style="margin-top:10px">
                    <label title=ctx_tr("Numero di cicli recenti da conservare")>{ctx_tr("Retain cicli")}</label><input prop:value=retain on:input=move |event| retain.set(event_target_value(&event)) title=ctx_tr("Quanti cicli di ricerca recenti tenere nello storico") />
                    <label title=ctx_tr("Età in giorni oltre la quale eliminare gli errori")>{ctx_tr("Giorni errori")}</label><input prop:value=age on:input=move |event| age.set(event_target_value(&event)) title=ctx_tr("Elimina gli errori più vecchi di questi giorni") />
                    <button class="btn danger" title=ctx_tr("Elimina dallo storico cicli e errori oltre i limiti indicati") on:click=move |_| {
                        let body = json!({"retain_cycles": retain.get().parse::<i64>().ok(), "error_age_days": age.get().parse::<i64>().ok()});
                        run_post(data, "/api/db/prune", Some(body), "Database pulito");
                    }>{ctx_tr("Pulisci database")}</button>
                </div>
            </Panel>
            <div class="grid-2">
            <Panel title="Duplicati video in libreria">
                <p class="muted">{ctx_tr("Individua i file video chiaramente inferiori (risoluzione più bassa) rimasti accanto alla versione migliore. Il controllo usa il nome del file e i dati tecnici dichiarati nel nome, non confronta il contenuto né calcola hash. Conservativo: tocca solo le risoluzioni riconosciute e lascia intatte le versioni con la stessa risoluzione. La pulizia sposta i file nel trash.")}</p>
                <div class="toolbar" style="margin-top:8px">
                    <button class="btn sm" disabled=move || duplicates_busy.get() on:click=move |_| {
                        duplicates_busy.set(true);
                        duplicates_message.set(String::new());
                        spawn_local(async move {
                            let result = send("POST", "/api/maintenance/clean-duplicates", Some(json!({"execute": false}))).await;
                            match result {
                                Ok(value) => {
                                    let items = array(&value, "items");
                                    let count = value.get("count").and_then(Value::as_i64).unwrap_or(items.len() as i64);
                                    duplicates.set(items);
                                    duplicates_count.set(count);
                                     duplicates_message.set(if count == 0 {
                                         tr(data, "Nessun duplicato trovato")
                                     } else {
                                         tr_format(data, "{count} candidati trovati (in tabella sotto)", &[("{count}", count.to_string())])
                                     });
                                 }
                                 Err(error) => {
                                     duplicates_message.set(tr_format(data, "Errore: {error}", &[("{error}", error.clone())]));
                                     push_toast(data, "err", tr(data, &error));
                                }
                            }
                            duplicates_busy.set(false);
                        });
                    }>{move || if duplicates_busy.get() { ctx_tr("Scansione…").get() } else { ctx_tr("Anteprima duplicati").get() }}</button>
                    <button class="btn sm danger" disabled=move || duplicates_busy.get() on:click=move |_| {
                        duplicates_busy.set(true);
                        duplicates_message.set(String::new());
                        spawn_local(async move {
                            let result = send("POST", "/api/maintenance/clean-duplicates", Some(json!({"execute": true}))).await;
                            match result {
                                Ok(value) => {
                                    let removed = number(&value, "removed");
                                    duplicates.set(Vec::new());
                                    duplicates_count.set(0);
                                     duplicates_message.set(tr_format(data, "Pulizia completata: {removed} nel trash", &[("{removed}", removed.clone())]));
                                     push_toast(data, "ok", tr_format(data, "{removed} duplicati spostati nel trash", &[("{removed}", removed.to_string())]));
                                }
                                Err(error) => {
                                     duplicates_message.set(tr_format(data, "Errore: {error}", &[("{error}", error.clone())]));
                                     push_toast(data, "err", tr(data, &error));
                                }
                            }
                            duplicates_busy.set(false);
                        });
                    }>{move || if duplicates_busy.get() { ctx_tr("Attendere…").get() } else { ctx_tr("Pulisci duplicati").get() }}</button>
                     <span class="muted">{move || if duplicates_message.get().is_empty() { tr_format(data, "{count} candidati", &[("{count}", duplicates_count.get().to_string())]) } else { duplicates_message.get() }}</span>
                </div>
                <Show when=move || duplicates_busy.get()>
                    <p class="muted">{ctx_tr("Scansione delle cartelle in corso…")}</p>
                </Show>
                <Show when=move || !duplicates.get().is_empty()>
                    <div class="table-wrap" style="margin-top:10px">
                        <table class="data-table">
                            <thead><tr><th>{ctx_tr("Serie")}</th><th>{ctx_tr("Episodio")}</th><th>{ctx_tr("File")}</th><th>{ctx_tr("Risoluzione")}</th></tr></thead>
                            <tbody>{move || duplicates.get().into_iter().map(|item| {
                                let season = item.get("season").and_then(Value::as_i64).unwrap_or(0);
                                let episode = item.get("episode").and_then(Value::as_i64).unwrap_or(0);
                                let path = text(&item, "path", "");
                                let path_title = path.clone();
                                view! {
                                    <tr>
                                        <td>{text(&item, "series", "")}</td>
                                        <td class="mono">{format!("S{season:02}E{episode:02}")}</td>
                                        <td class="truncate mono muted" title=path_title>{path}</td>
                                        <td class="numeric">{format!("{} < {}", number(&item, "resolution_rank"), number(&item, "best_rank"))}</td>
                                    </tr>
                                }
                            }).collect_view()}</tbody>
                        </table>
                    </div>
                </Show>
            </Panel>
            <DbOptimizeTool data />
            </div>
            <Panel title="Diagnostica sorgenti e servizi">
                <p class="muted" title=ctx_tr("Questa sezione esegue una ricerca reale con query e include anche test porte e notifiche; la sezione Stato sorgenti in Salute mostra invece il controllo generale delle sorgenti.")>{ctx_tr("Test query sulle sorgenti, porte e notifiche")}</p>
                <div class="toolbar">
                    <input prop:value=source_query on:input=move |event| source_query.set(event_target_value(&event)) placeholder=ctx_tr("Query di test (es. 9-1-1 S10)") />
                    <button class="btn sm" title=ctx_tr("Esegue una ricerca reale su feed, indexer e motori web e mostra quanti risultati trova ciascuno") on:click=move |_| {
                        let q = source_query.get();
                        let sources = sources;
                        spawn_local(async move {
                            let path = format!("/api/sources/health?q={}", urlencoding::encode(&q));
                            if let Ok(value) = get(&path).await {
                                sources.set(array(&value, "items"));
                            }
                        });
                    }>{ctx_tr("Testa sorgenti")}</button>
                    <span class="muted">{move || tr_format(data, "{count} sorgenti", &[("{count}", sources.get().len().to_string())])}</span>
                    <button class="btn sm" on:click=move |_| { let ports = ports; spawn_local(async move { if let Ok(value) = get("/api/config/check-ports").await { ports.set(value); } }); }>{ctx_tr("Verifica porte")}</button>
                    <button class="btn sm" on:click=move |_| run_post(data, "/api/test-notification", Some(json!({"message": "Rextto test"})), "Notifica inviata")>{ctx_tr("Test notifica")}</button>
                </div>
                <div class="list" style="margin-top:10px; max-height:320px; overflow:auto">
                    {move || sources.get().iter().cloned().map(|item| {
                        let ok = item.get("ok").and_then(Value::as_bool).unwrap_or(false);
                        let results = item.get("results").and_then(Value::as_i64);
                        view! {
                            <div class="list-item">
                                 <div><strong>{text(&item, "name", "-")}</strong><small class="mono">{text(&item, "kind", "-")}{results.map(|value| format!(" · {}", tr_format(data, "{count} risultati", &[("{count}", value.to_string())]))).unwrap_or_default()}</small></div>
                                 <span class="badge" class:ok=ok class:err=move || !ok>{if ok { tr(data, "ok") } else { tr(data, "errore") }}</span>
                            </div>
                        }
                    }).collect_view()}
                </div>
                <Show when=move || !ports.get().is_null()>
                    <div class="list" style="margin-top:8px">
                        {move || ports.get().get("ports").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().map(|item| {
                            let available = item.get("available").and_then(Value::as_bool).unwrap_or(false);
                             view! { <div class="list-item"><span class="mono">{tr_format(data, "porta {port}", &[("{port}", number(&item, "port"))])}</span><span class="badge" class:ok=available class:err=move || !available>{if available { tr(data, "libera") } else { tr(data, "occupata") }}</span></div> }
                        }).collect_view()}
                    </div>
                </Show>
            </Panel>
            <Panel title="Backup disponibili">
                <div class="grid-2">
                    {move || data.get().backups.iter().cloned().map(|item| {
                        let name = text(&item, "name", "-");
                        let label = text(&item, "label", &name);
                        let name_title = name.clone();
                        view! {
                            <div class="list-item">
                                <div class="truncate" title=name_title>{tr_format(data, "Backup {label}", &[("{label}", label)])}</div>
                                <span class="badge">{size(&item, "size_bytes")}</span>
                            </div>
                        }
                    }).collect_view()}
                </div>
                <Show when=move || data.get().backups.is_empty()><Empty text="Nessun backup disponibile." /></Show>
            </Panel>
            <BackupSettings data />
            <DbPruneTool data />
             <Panel title="Trash">
                 <div class="stack">
                     <div class="row"><span class="muted">{ctx_tr("Percorso trash")}</span><strong class="mono">{move || text(&trash.get(), "path", "-")}</strong></div>
                      <div class="row"><span class="muted">{ctx_tr("Stato cartella")}</span><strong>{move || if trash.get().get("exists").and_then(Value::as_bool).unwrap_or(false) { tr(data, "presente") } else { tr(data, "non presente") }}</strong></div>
                      <div class="row"><span class="muted">{ctx_tr("Elementi")}</span><strong>{move || number(&trash.get(), "count")}</strong></div>
                      <div class="row"><span class="muted">{ctx_tr("Dimensione")}</span><strong>{move || size(&trash.get(), "total_bytes")}</strong></div>
                      <div class="row"><span class="muted">{ctx_tr("Conservazione")}</span><strong>{move || format!("{} {}", number(&trash.get(), "retention_days"), tr(data, "giorni"))}</strong></div>
                     <div class="toolbar">
                         <button class="btn" on:click=move |_| {
                             let refresh_trash = refresh_trash;
                             spawn_local(async move {
                                 let result = send("POST", "/api/maintenance/clean-trash", None).await;
                                 flash(data, result, "Trash pulito");
                                 refresh_trash();
                             });
                         }>{ctx_tr("Pulisci trash")}</button>
                         <button class="btn sm" on:click=move |_| refresh_trash()>{ctx_tr("Aggiorna")}</button>
                    </div>
                    <div class="table-wrap">
                        <table class="data-table">
                            <thead><tr><th>{ctx_tr("File")}</th><th>{ctx_tr("Dimensione")}</th></tr></thead>
                            <tbody>
                                {move || array(&trash.get(), "items").into_iter().take(100).map(|item| view! {
                                    <tr><td class="truncate mono">{text(&item, "name", "-")}</td><td class="numeric">{size(&item, "size_bytes")}</td></tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                </div>
            </Panel>
        </div>
    }
}

/// Escapes HTML metacharacters so raw log lines can be injected with `inner_html`.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Turns a raw log line into HTML: colourises the record level and highlights
/// the meaningful keywords (NAS, download, sources, errors, filters, scores) so
/// the viewer is scannable at a glance.
fn highlight_log_line(line: &str) -> String {
    const KEYWORDS: &[(&str, &str)] = &[
        ("source report", "hl-src"),
        ("nas", "hl-nas"),
        ("download", "hl-dl"),
        ("indexer", "hl-src"),
        ("feed", "hl-src"),
        ("engine", "hl-src"),
        ("scraping", "hl-src"),
        ("error", "hl-err"),
        ("failed", "hl-err"),
        ("failure", "hl-err"),
        ("stalled", "hl-warn"),
        ("warning", "hl-warn"),
        ("completed", "hl-ok"),
        ("complete", "hl-ok"),
        ("archived", "hl-ok"),
        ("moved", "hl-ok"),
        ("approved", "hl-ok"),
        ("upgrade", "hl-score"),
        ("score", "hl-score"),
        ("filter", "hl-filter"),
        ("rejected", "hl-filter"),
        ("skipped", "hl-filter"),
        ("blocklist", "hl-filter"),
    ];
    // Whole-word matching: avoids highlighting "error" inside "errors" or
    // "moved" inside "removed".
    fn is_word(character: char) -> bool {
        character.is_alphanumeric() || character == '_'
    }
    let lower = line.to_ascii_lowercase();
    let mut body = String::with_capacity(line.len() + 32);
    let mut index = 0;
    let mut previous_is_word = false;
    while index < line.len() {
        let mut best: Option<(usize, &str, bool)> = None;
        for &(keyword, class) in KEYWORDS {
            if lower[index..].starts_with(keyword) {
                let end = index + keyword.len();
                let next_is_word = line[end..].chars().next().is_some_and(is_word);
                let last_is_word = line[index..end].chars().next_back().is_some_and(is_word);
                if !previous_is_word
                    && !next_is_word
                    && best.map_or(true, |(len, _, _)| keyword.len() > len)
                {
                    best = Some((keyword.len(), class, last_is_word));
                }
            }
        }
        if let Some((len, class, last_is_word)) = best {
            let matched = &line[index..index + len];
            body.push_str("<mark class=\"");
            body.push_str(class);
            body.push_str("\">");
            body.push_str(&escape_html(matched));
            body.push_str("</mark>");
            index += len;
            previous_is_word = last_is_word;
        } else {
            let ch = line[index..].chars().next().unwrap();
            let end = index + ch.len_utf8();
            body.push_str(&escape_html(&line[index..end]));
            index = end;
            previous_is_word = is_word(ch);
        }
    }
    let class = if line.contains(" ERROR ") {
        "log-error"
    } else if line.contains(" WARN ") {
        "log-warn"
    } else if line.contains(" INFO ") {
        "log-info"
    } else if line.contains(" DEBUG ") || line.contains(" TRACE ") {
        "log-debug"
    } else {
        ""
    };
    if class.is_empty() {
        body
    } else {
        format!("<span class=\"{class}\">{body}</span>")
    }
}

#[component]
fn LogsView(data: RwSignal<Data>) -> impl IntoView {
    let _ = data;
    let filter = RwSignal::new(String::new());
    let lines = RwSignal::new(Vec::<String>::new());
    let follow = RwSignal::new(true);
    let limit = RwSignal::new("400".to_string());
    let loading = RwSignal::new(true);
    let log_ref = NodeRef::<leptos::html::Pre>::new();
    // Caricamento semplice e affidabile: una GET iniziale e poi un refresh
    // periodico. Evita di dipendere dall'evento iniziale di uno stream SSE,
    // che può arrivare prima che il browser abbia installato il listener.
    let polling = RwSignal::new(true);
    on_cleanup(move || polling.set(false));
    spawn_local(async move {
        loop {
            let requested = limit.get_untracked().parse::<usize>().unwrap_or(400).clamp(50, 2000);
            if let Ok(value) = get(&format!("/api/logs?limit={requested}")).await {
                let next: Vec<String> = array(&value, "items")
                    .into_iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect();
                // Evita di ri-renderizzare (e ri-evidenziare) tutto il log ad
                // ogni poll quando non è cambiato nulla.
                if lines.get_untracked() != next {
                    lines.set(next);
                }
            }
            loading.set(false);
            TimeoutFuture::new(5_000).await;
            if !polling.get_untracked() {
                break;
            }
        }
    });
    let reload_limit = move |event: web_sys::Event| {
        let requested = event_target_value(&event)
            .parse::<usize>()
            .unwrap_or(400)
            .clamp(50, 5000);
        limit.set(requested.to_string());
        lines.set(Vec::new());
        loading.set(true);
        spawn_local(async move {
            let result = get(&format!("/api/logs?limit={requested}")).await;
            if limit.get() == requested.to_string() {
                if let Ok(value) = result {
                    let next: Vec<String> = array(&value, "items")
                        .into_iter()
                        .filter_map(|item| item.as_str().map(str::to_owned))
                        .collect();
                    if lines.get_untracked() != next {
                        lines.set(next);
                    }
                }
                loading.set(false);
            }
        });
    };
    let filtered = Signal::derive(move || {
        let term = filter.get().to_ascii_lowercase();
        lines
            .get()
            .iter()
            .filter(|line| term.is_empty() || line.to_ascii_lowercase().contains(&term))
            .map(|line| highlight_log_line(line))
            .collect::<Vec<_>>()
    });
    Effect::new(move |_| {
        let _ = filtered.get();
        if follow.get() {
            if let Some(element) = log_ref.get() {
                element.set_scroll_top(element.scroll_height());
            }
        }
    });
    view! {
        <div class="view">
            <Panel title="Log daemon">
                <div class="toolbar" style="margin-bottom:10px">
                    <input style="flex:1" prop:value=filter on:input=move |event| filter.set(event_target_value(&event)) placeholder=ctx_tr("Filtra…") />
                    <label class="check" title=ctx_tr("Numero di righe di log da caricare (50–5000)")><span>{ctx_tr("Righe")}</span><input style="width:80px" prop:value=limit on:change=reload_limit /></label>
                    <button class="btn sm" class:primary=move || follow.get() on:click=move |_| follow.update(|value| *value = !*value)>
                        {move || if follow.get() { tr(data, "⏸ Ferma scorrimento") } else { tr(data, "▶ Riprendi") }}
                    </button>
                    <small class="muted">{move || if loading.get() { tr(data, "Caricamento…") } else { format!("{} {}", filtered.get().len(), tr(data, "righe")) }}</small>
                </div>
                <pre class="log-view" node_ref=log_ref inner_html=move || filtered.get().join("\n")></pre>
            </Panel>
        </div>
    }
}

#[component]
fn ServicesPanel(data: RwSignal<Data>) -> impl IntoView {
    let services = RwSignal::new(Vec::<Value>::new());
    let indexers = RwSignal::new(Vec::<Value>::new());
    let busy = RwSignal::new(false);
    let reload = move || {
        if busy.get() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            if let Ok(value) = get("/api/services").await {
                services.set(array(&value, "services"));
                indexers.set(array(&value, "indexers"));
            }
            busy.set(false);
        });
    };
    Effect::new(move |_| reload());
    view! {
        <Panel title="Servizio Rextto">
            <div class="toolbar" style="margin-bottom:10px">
                <button class="btn sm" disabled=move || busy.get() on:click=move |_| reload()>
                    {move || if busy.get() { tr(data, "Aggiorna…") } else { tr(data, "Aggiorna") }}
                </button>
                <span class="muted">{ctx_tr("Stato di rextto.service e riavvio senza password.")}</span>
            </div>
            <div class="table-wrap">
                <table class="data-table">
                    <thead><tr><th>{ctx_tr("Unità")}</th><th>{ctx_tr("Attivo")}</th><th>{ctx_tr("Abilitato")}</th><th>{ctx_tr("Azioni")}</th></tr></thead>
                    <tbody>
                        {move || services.get().into_iter().map(|entry| {
                            let unit = text(&entry, "unit", "-");
                            let active = text(&entry, "active", "unknown");
                            let enabled = text(&entry, "enabled", "unknown");
                            let is_active = active == "active";
                            view! {
                                <tr>
                                    <td class="mono">{unit}</td>
                                    <td><span class="badge" class:ok=is_active class:err=!is_active>{active}</span></td>
                                    <td class="muted">{enabled}</td>
                                    <td>
                                        <div class="row-actions">
                                            <button class="btn sm" title=ctx_tr("Avvia rextto.service") on:click=move |_| {
                                                spawn_local(async move {
                                                    match send("POST", "/api/service/restart", Some(json!({"action": "start"}))).await {
                                                         Ok(_) => push_toast(data, "ok", tr(data, "Avvio del servizio richiesto…")),
                                                        Err(error) => push_toast(data, "err", error),
                                                    }
                                                });
                                            }>{ctx_tr("Avvia")}</button>
                                            <button class="btn sm" title=ctx_tr("Riavvia rextto.service senza password") on:click=move |_| {
                                                spawn_local(async move {
                                                    match send("POST", "/api/service/restart", Some(json!({"action": "restart"}))).await {
                                                         Ok(_) => push_toast(data, "ok", tr(data, "Riavvio del servizio richiesto…")),
                                                        Err(error) => push_toast(data, "err", error),
                                                    }
                                                });
                                            }>{ctx_tr("Riavvia")}</button>
                                            <button class="btn sm danger" title=ctx_tr("Ferma rextto.service (la UI si interrompe finché non lo riavvii da terminale)") on:click=move |_| {
                                                spawn_local(async move {
                                                    match send("POST", "/api/service/restart", Some(json!({"action": "stop"}))).await {
                                                         Ok(_) => push_toast(data, "info", tr(data, "Arresto del servizio richiesto…")),
                                                        Err(error) => push_toast(data, "err", error),
                                                    }
                                                });
                                            }>{ctx_tr("Ferma")}</button>
                                        </div>
                                    </td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
        </Panel>
        <Panel title="Indexer e FlareSolverr">
            <div class="table-wrap">
                <table class="data-table">
                    <thead><tr><th>{ctx_tr("Nome")}</th><th>{ctx_tr("URL")}</th><th>{ctx_tr("Abilitato")}</th><th>{ctx_tr("Raggiungibile")}</th></tr></thead>
                    <tbody>
                        {move || {
                            let items = indexers.get();
                            if items.is_empty() {
                                return view! { <tr><td colspan="4" class="muted">{ctx_tr("Nessun indexer o FlareSolverr configurato.")}</td></tr> }.into_any();
                            }
                            items.into_iter().map(|entry| {
                                let is_flaresolverr = text(&entry, "kind", "") == "flaresolverr";
                                let reachable = entry.get("reachable").and_then(Value::as_bool).unwrap_or(false);
                                let status = entry.get("status").and_then(Value::as_i64).map(|value| value.to_string()).unwrap_or_else(|| "-".into());
                                view! {
                                    <tr title=if is_flaresolverr { Some(ctx_tr("Servizio FlareSolverr configurato come supporto anti-Cloudflare").get()) } else { None }>
                                        <td>{text(&entry, "name", "-")}</td>
                                        <td class="mono truncate muted">{text(&entry, "url", "-")}</td>
                                         <td><span class="badge ok">{if is_flaresolverr { tr(data, "configurato") } else { tr(data, "sì") }}</span></td>
                                         <td><span class="badge" class:ok=reachable class:err=!reachable>{if reachable { format!("{} ({status})", tr(data, "ok")) } else { tr(data, "non raggiungibile") }}</span></td>
                                    </tr>
                                }
                            }).collect_view().into_any()
                        }}
                    </tbody>
                </table>
            </div>
        </Panel>
    }
}

#[component]
fn HealthView(data: RwSignal<Data>) -> impl IntoView {
    let sources = RwSignal::new(Vec::<Value>::new());
    let sources_busy = RwSignal::new(false);
    let sources_loaded = RwSignal::new(false);
    let live_cpu = Signal::derive(move || data.get().live_history.get("cpu").and_then(|values| values.last()).copied().unwrap_or(0.0));
    let live_download = Signal::derive(move || data.get().live_history.get("down").and_then(|values| values.last()).copied().unwrap_or(0.0));
    let live_upload = Signal::derive(move || data.get().live_history.get("up").and_then(|values| values.last()).copied().unwrap_or(0.0));
    let live_ramdisk = Signal::derive(move || data.get().live_history.get("ramdisk_free").and_then(|values| values.last()).copied().unwrap_or(0.0));
    view! {
        <div class="view">
            <div class="metrics health-metrics">
                <Metric label="Stato" value=Signal::derive(move || text(&data.get().health, "status", "offline")) tone="mint" />
                <Metric label="RAM processo" value=Signal::derive(move || size(&data.get().health, "resident_bytes")) tone="blue" />
                <Metric label="Spazio libero" value=Signal::derive(move || size(&data.get().health, "disk_free_bytes")) tone="amber" />
                <Metric label="Trash" value=Signal::derive(move || size(&data.get().health, "trash_bytes")) tone="danger" />
                <Metric label="CPU live" value=Signal::derive(move || format!("{:.0}%", live_cpu.get())) tone="violet" />
                <Metric label="Download live" value=Signal::derive(move || format!("{}/s", size_str(live_download.get()))) tone="blue" />
                <Metric label="Upload live" value=Signal::derive(move || format!("{}/s", size_str(live_upload.get()))) tone="mint" />
                <Metric label="RAM disk" value=Signal::derive(move || size_str(live_ramdisk.get())) tone="amber" />
            </div>
            <Panel title="Runtime">
                <div class="grid-3">
                    <StatLine label="Processo" value=Signal::derive(move || number(&data.get().health, "process_id")) />
                     <StatLine label="Data directory scrivibile" value=Signal::derive(move || if data.get().health.get("data_dir_writable").and_then(Value::as_bool).unwrap_or(false) { tr(data, "sì") } else { tr(data, "no") }) />
                    <StatLine label="Load average" value=Signal::derive(move || data.get().health.get("load_average").and_then(Value::as_f64).map(|value| format!("{value:.2}")).unwrap_or_else(|| "-".into())) />
                    <StatLine label="CPU / RAM sistema" value=Signal::derive(move || format!("{} / {}", data.get().health.get("cpu_percent").and_then(Value::as_f64).map(|value| format!("{value:.0}%")).unwrap_or_else(|| "-".into()), size(&data.get().health, "memory_available_bytes"))) />
                    <StatLine label="File in trash" value=Signal::derive(move || number(&data.get().health, "trash_file_count")) />
                    <Show when=move || data.get().health.get("ramdisk").map(|value| !value.is_null()).unwrap_or(false)>
                        <StatLine label="RAM disk" value=Signal::derive(move || {
                            let ramdisk = data.get().health.get("ramdisk").cloned().unwrap_or_default();
                            tr_format(data, "{path} — {free} liberi / {total}", &[("{path}", text(&ramdisk, "path", "-")), ("{free}", size(&ramdisk, "free_bytes")), ("{total}", size(&ramdisk, "total_bytes"))])
                        }) />
                    </Show>
                </div>
            </Panel>
            <ServicesPanel data />
            <Panel title="Percorsi e dischi">
                <div class="grid-2">
                    <div class="setting-group">
                        <h4>{ctx_tr("Permessi percorsi")}</h4>
                        <div class="table-wrap">
                            <table class="data-table">
                                <thead><tr><th>{ctx_tr("Percorso")}</th><th>{ctx_tr("Path")}</th><th>{ctx_tr("Esiste")}</th><th>{ctx_tr("Scrivibile")}</th></tr></thead>
                                <tbody>
                                    {move || data.get().health.get("paths").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().map(|entry| {
                                        let exists = entry.get("exists").and_then(Value::as_bool).unwrap_or(false);
                                        let writable = entry.get("writable").and_then(Value::as_bool).unwrap_or(false);
                                        view! {
                                            <tr>
                                                <td>{text(&entry, "label", "-")}</td>
                                                <td class="mono truncate muted">{text(&entry, "path", "-")}</td>
                                                 <td><span class="badge" class:ok=exists class:err=!exists>{if exists { tr(data, "sì") } else { tr(data, "no") }}</span></td>
                                                 <td><span class="badge" class:ok=writable class:err=!writable>{if writable { tr(data, "sì") } else { tr(data, "no") }}</span></td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                        </div>
                    </div>
                    <div class="setting-group">
                        <h4>{ctx_tr("Dischi")}</h4>
                        <div class="stack">
                            <Show when=move || data.get().health.get("disks").and_then(Value::as_array).map(|items| items.is_empty()).unwrap_or(true)>
                                <span class="muted">{ctx_tr("Nessun filesystem rilevato.")}</span>
                            </Show>
                            {move || data.get().health.get("disks").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().map(|disk| {
                                let mount = text(&disk, "mount", "-");
                                let mount_hint = mount.clone();
                                let total = disk.get("total_bytes").and_then(Value::as_f64).unwrap_or(0.0);
                                let free = disk.get("free_bytes").and_then(Value::as_f64).unwrap_or(0.0);
                                let used = if total > 0.0 { (total - free) / total * 100.0 } else { 0.0 };
                                view! {
                                    <div class="disk-row" title=format!("{} · {}", text(&disk, "filesystem", "-"), mount_hint)>
                                        <span class="mono truncate">{mount}</span>
                                         <span class="muted">{tr_format(data, "{free} liberi / {total} · {used}% usato", &[("{free}", size_str(free)), ("{total}", size_str(total)), ("{used}", format!("{used:.0}"))])}</span>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    </div>
                </div>
            </Panel>
            <Panel title="Stato sorgenti">
                <div class="toolbar" style="margin-bottom:10px">
                    <button class="btn" disabled=move || sources_busy.get() on:click=move |_| {
                        if sources_busy.get() { return; }
                        sources_busy.set(true);
                        let sources = sources;
                        let loaded = sources_loaded;
                        spawn_local(async move {
                            if let Ok(value) = get("/api/sources/health").await {
                                sources.set(array(&value, "items"));
                            }
                            loaded.set(true);
                            sources_busy.set(false);
                        });
                     }>{move || if sources_busy.get() { tr(data, "Verifica…") } else { tr(data, "Verifica sorgenti") }}</button>
                    <span class="muted">{move || {
                        let items = sources.get();
                        let ok = items.iter().filter(|item| item.get("ok").and_then(Value::as_bool).unwrap_or(false)).count();
                         if items.is_empty() { if sources_loaded.get() { tr(data, "nessuna sorgente") } else { tr(data, "non verificato") } } else { format!("{ok}/{} {}", items.len(), tr(data, "sane")) }
                    }}</span>
                </div>
                <div class="table-wrap">
                    <table class="data-table">
                        <thead><tr><th>{ctx_tr("Tipo")}</th><th>{ctx_tr("Nome")}</th><th>{ctx_tr("Esito")}</th><th>{ctx_tr("Dettaglio")}</th></tr></thead>
                        <tbody>
                            {move || sources.get().into_iter().map(|entry| {
                                let ok = entry.get("ok").and_then(Value::as_bool).unwrap_or(false);
                                let detail = {
                                    let error = text(&entry, "error", "");
                                     if !error.is_empty() { error } else { let results = entry.get("results").and_then(Value::as_i64).unwrap_or(0); if results > 0 { tr_format(data, "{count} risultati", &[("{count}", results.to_string())]) } else { String::new() } }
                                };
                                let detail_hint = detail.clone();
                                view! {
                                    <tr>
                                        <td class="muted">{text(&entry, "kind", "-")}</td>
                                        <td class="truncate">{text(&entry, "name", "-")}</td>
                                        <td><span class="badge" class:ok=ok class:err=!ok>{if ok { tr(data, "ok") } else { tr(data, "errore") }}</span></td>
                                        <td class="truncate muted" title=detail_hint>{detail}</td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
            </Panel>
            <Panel title="Ultimi errori">
                <Show when=move || array(&data.get().health, "last_errors").is_empty()><span class="muted">{ctx_tr("Nessun errore recente nel log.")}</span></Show>
                <pre class="output">{move || array(&data.get().health, "last_errors").into_iter().filter_map(|item| item.as_str().map(str::to_owned)).collect::<Vec<_>>().join("\n")}</pre>
            </Panel>
        </div>
    }
}

#[component]
fn MissingView(data: RwSignal<Data>) -> impl IntoView {
    let results = RwSignal::new(Vec::<Value>::new());
    view! {
        <div class="view">
            <Panel title="Episodi mancanti">
                <div class="stack">
                    {move || {
                        let mut grouped = std::collections::BTreeMap::<String, Vec<Value>>::new();
                        for item in data.get().gaps {
                            grouped.entry(text_tr(data, &item, "series", "Serie")).or_default().push(item);
                        }
                        grouped.into_iter().map(|(series_name, mut episodes)| {
                            episodes.sort_by_key(|item| (
                                item.get("season").and_then(Value::as_i64).unwrap_or(0),
                                item.get("episode").and_then(Value::as_i64).unwrap_or(0),
                            ));
                            let count = episodes.len();
                            view! {
                                <details class="panel" style="padding:0">
                                    <summary class="list-item" style="cursor:pointer">
                                        <strong>{series_name.clone()}</strong>
                                         <span class="badge warn">{tr_format(data, "{count} episodi mancanti", &[("{count}", count.to_string())])}</span>
                                    </summary>
                                    <div class="table-wrap" style="padding:0 12px 12px">
                                        <table class="data-table">
                                            <thead><tr><th>{ctx_tr("Episodio")}</th><th>{ctx_tr("Data")}</th><th></th></tr></thead>
                                            <tbody>
                                            {episodes.into_iter().map(|item| {
                                let series = text(&item, "series", "");
                                let season = item.get("season").and_then(Value::as_i64).unwrap_or(0);
                                let episode = item.get("episode").and_then(Value::as_i64).unwrap_or(0);
                                let search_series = series.clone();
                                let ignore_series = series.clone();
                                view! {
                                    <tr>
                                        <td class="mono">{format!("S{season:02}E{episode:02}")}</td>
                                        <td class="muted">{text(&item, "air_date", "-")}</td>
                                        <td>
                                            <div class="toolbar">
                                                <button class="btn sm" on:click=move |_| {
                                                    let body = json!({"series": search_series.clone(), "season": season, "episode": episode});
                                                    let results = results;
                                                    spawn_local(async move {
                                                        if let Ok(value) = send("POST", "/api/missing/search", Some(body)).await {
                                                            results.set(array(&value, "results"));
                                                        }
                                                    });
                                                }>{ctx_tr("Cerca")}</button>
                                                <button class="btn sm" on:click=move |_| {
                                                    let path = format!("/api/episodes/{}/{}/{}/ignore", urlencoding::encode(&ignore_series), season, episode);
                                                    run_post(data, &path, Some(json!({"ignored": true, "reason": "ui"})), "Episodio ignorato");
                                                }>{ctx_tr("Ignora")}</button>
                                            </div>
                                        </td>
                                    </tr>
                                }
                                            }).collect_view()}
                                            </tbody>
                                        </table>
                                    </div>
                                </details>
                            }
                        }).collect_view()
                    }}
                </div>
            </Panel>
            <Show when=move || !results.get().is_empty()>
                <Panel title="Release trovate">
                    <div class="list">
                        {move || results.get().iter().cloned().map(|item| {
                            let release = item.clone();
                            view! {
                                <div class="list-item">
                                    <div><strong>{text(&item, "title", "Release")}</strong><small>{text(&item, "source", "-")}</small></div>
                                    <button class="btn sm primary" on:click=move |_| { let release = release.clone(); run_post(data, "/api/search/add", Some(json!({"release": release})), "Release accodata"); }>{ctx_tr("Accoda")}</button>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </Panel>
            </Show>
        </div>
    }
}

#[component]
fn CalendarView(data: RwSignal<Data>) -> impl IntoView {
    view! {
        <div class="view">
            <Panel title="Calendario TMDB">
                <div class="calendar-grid">
                    {move || data.get().calendar.iter().cloned().map(|item| {
                        let episode = item.get("episode").cloned().unwrap_or_default();
                        let poster = item.get("poster").and_then(Value::as_str).map(str::to_owned);
                        let series_name = text_tr(data, &item, "series", "Serie");
                        let tmdb_id = text(&item, "tmdb_id", "");
                        let season_number = episode.get("season_number").and_then(Value::as_i64);
                        let episode_number = episode.get("episode_number").and_then(Value::as_i64);
                        let href = match (tmdb_id.is_empty(), season_number, episode_number) {
                            (false, Some(season), Some(episode_number)) => format!("https://www.themoviedb.org/tv/{tmdb_id}/season/{season}/episode/{episode_number}"),
                            _ => format!("https://www.themoviedb.org/search?query={}", urlencoding::encode(&series_name)),
                        };
                        view! {
                            <a class="list-item" href=href target="_blank" rel="noopener" title=ctx_tr("Apri l'episodio su TMDB")>
                                {match poster {
                                    Some(url) => view! { <img class="list-poster" src=url alt=series_name.clone() loading="lazy" /> }.into_any(),
                                    None => view! { <div class="list-poster placeholder">{ctx_tr("N/D")}</div> }.into_any(),
                                }}
                                <div style="flex:1"><strong>{series_name.clone()}</strong><small>{format!("S{}E{} · {}", number(&episode, "season_number"), number(&episode, "episode_number"), text(&episode, "air_date", "-"))}</small></div>
                            </a>
                        }
                    }).collect_view()}
                </div>
                <Show when=move || data.get().calendar.is_empty()><Empty text="Nessun prossimo episodio disponibile." /></Show>
            </Panel>
        </div>
    }
}

/// Pannello Blocklist, usato dalla pagina Blocklist.
#[component]
fn BlocklistPanel(data: RwSignal<Data>) -> impl IntoView {
    view! {
        <Panel title="Blocklist">
            <div class="list">
                {move || data.get().blocklist.iter().cloned().map(|item| {
                    let hash = text(&item, "hash", "");
                    view! {
                        <div class="list-item">
                            <div><strong>{text(&item, "title", "Release")}</strong><small class="mono">{hash.clone()}"/ "{text(&item, "reason", "-")}</small></div>
                            <button class="btn sm" on:click=move |_| run_post(data, &format!("/api/blocklist/{hash}/remove"), None, "Elemento sbloccato")>{ctx_tr("Sblocca")}</button>
                        </div>
                    }
                }).collect_view()}
            </div>
            <Show when=move || data.get().blocklist.is_empty()><Empty text="Nessun magnet bloccato." /></Show>
        </Panel>
    }
}

#[component]
fn BlocklistView(data: RwSignal<Data>) -> impl IntoView {
    view! {
        <div class="view">
            <BlocklistPanel data />
        </div>
    }
}

/// Manuale utente, incorporato nel bundle della UI in italiano e inglese.
const MANUAL_IT: &str = include_str!("../../../docs/MANUAL.it.md");
const MANUAL_EN: &str = include_str!("../../../docs/MANUAL.en.md");

/// Slug per gli id dei titoli: rende funzionanti i link interni del manuale
/// (es. `#1-primo-avvio`), con lo stesso criterio di GitHub.
fn manual_slug(text: &str) -> String {
    let mut slug = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
        } else if (ch.is_whitespace() || ch == '-' || ch == '_') && !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

/// Ritorna il contenuto tra due marcatori se `text` inizia con `marker`.
fn manual_delimited<'a>(text: &'a str, marker: &str) -> Option<(&'a str, usize)> {
    let after = text.strip_prefix(marker)?;
    let end = after.find(marker)?;
    if end == 0 {
        return None;
    }
    Some((&after[..end], marker.len() + end + marker.len()))
}

/// Riconosce `[etichetta](url)` all'inizio di `text`.
fn manual_link(text: &str) -> Option<(&str, &str, usize)> {
    if !text.starts_with('[') {
        return None;
    }
    let close = text.find("](")?;
    let label = &text[1..close];
    let after = &text[close + 2..];
    let end = after.find(')')?;
    let url = &after[..end];
    if label.is_empty() || url.is_empty() {
        return None;
    }
    Some((label, url, close + 2 + end + 1))
}

/// Formattazione inline del manuale: `code`, **grassetto**, *corsivo* e link.
/// Il testo in ingresso è già passato da `escape_html`.
fn render_manual_inline(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len() + 16);
    let mut index = 0usize;
    while index < escaped.len() {
        let rest = &escaped[index..];
        if let Some((inner, consumed)) = manual_delimited(rest, "`") {
            out.push_str("<code>");
            out.push_str(inner);
            out.push_str("</code>");
            index += consumed;
            continue;
        }
        if let Some((inner, consumed)) = manual_delimited(rest, "**") {
            out.push_str("<strong>");
            out.push_str(&render_manual_inline(inner));
            out.push_str("</strong>");
            index += consumed;
            continue;
        }
        if let Some((inner, consumed)) = manual_delimited(rest, "*") {
            out.push_str("<em>");
            out.push_str(&render_manual_inline(inner));
            out.push_str("</em>");
            index += consumed;
            continue;
        }
        if let Some((label, url, consumed)) = manual_link(rest) {
            if url.ends_with(".md") {
                out.push_str(&render_manual_inline(label));
            } else if url.starts_with('#') {
                out.push_str("<a href=\"");
                out.push_str(url);
                out.push_str("\">");
                out.push_str(&render_manual_inline(label));
                out.push_str("</a>");
            } else {
                out.push_str("<a href=\"");
                out.push_str(url);
                out.push_str("\" target=\"_blank\" rel=\"noopener\">");
                out.push_str(&render_manual_inline(label));
                out.push_str("</a>");
            }
            index += consumed;
            continue;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn flush_manual_paragraph(html: &mut String, paragraph: &mut String) {
    let text = paragraph.trim();
    if !text.is_empty() {
        html.push_str("<p>");
        html.push_str(&render_manual_inline(&escape_html(text)));
        html.push_str("</p>");
    }
    paragraph.clear();
}

fn flush_manual_list_item(html: &mut String, item: &mut String) {
    let text = item.trim();
    if !text.is_empty() {
        html.push_str("<li>");
        html.push_str(&render_manual_inline(&escape_html(text)));
        html.push_str("</li>");
    }
    item.clear();
}

/// Converte il manuale Markdown in HTML. Copre la sintassi usata dai file
/// `docs/MANUAL.*.md`: titoli, paragrafi, elenchi puntati, righe orizzontali e
/// formattazione inline. Evita di introdurre una dipendenza markdown.
fn render_manual_markdown(markdown: &str) -> String {
    let mut html = String::with_capacity(markdown.len() * 2);
    let mut paragraph = String::new();
    let mut list_item = String::new();
    let mut in_list = false;

    let heading = |html: &mut String, level: usize, text: &str| {
        html.push_str(&format!(
            "<h{0} id=\"{1}\">{2}</h{0}>",
            level,
            manual_slug(text),
            render_manual_inline(&escape_html(text))
        ));
    };

    for raw in markdown.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            flush_manual_paragraph(&mut html, &mut paragraph);
            flush_manual_list_item(&mut html, &mut list_item);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            continue;
        }
        if trimmed == "---" {
            flush_manual_paragraph(&mut html, &mut paragraph);
            flush_manual_list_item(&mut html, &mut list_item);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            html.push_str("<hr>");
            continue;
        }
        if let Some(text) = trimmed.strip_prefix("### ") {
            flush_manual_paragraph(&mut html, &mut paragraph);
            flush_manual_list_item(&mut html, &mut list_item);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            heading(&mut html, 3, text);
            continue;
        }
        if let Some(text) = trimmed.strip_prefix("## ") {
            flush_manual_paragraph(&mut html, &mut paragraph);
            flush_manual_list_item(&mut html, &mut list_item);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            heading(&mut html, 2, text);
            continue;
        }
        if let Some(text) = trimmed.strip_prefix("# ") {
            flush_manual_paragraph(&mut html, &mut paragraph);
            flush_manual_list_item(&mut html, &mut list_item);
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            heading(&mut html, 1, text);
            continue;
        }
        if let Some(text) = trimmed.strip_prefix("- ") {
            flush_manual_paragraph(&mut html, &mut paragraph);
            flush_manual_list_item(&mut html, &mut list_item);
            if !in_list {
                html.push_str("<ul>");
                in_list = true;
            }
            list_item.push_str(text);
            continue;
        }
        // Continuazione di una voce di elenco (riga indentata).
        if in_list && raw.len() > trimmed.len() {
            if !list_item.is_empty() {
                list_item.push(' ');
            }
            list_item.push_str(trimmed);
            continue;
        }
        flush_manual_list_item(&mut html, &mut list_item);
        if in_list {
            html.push_str("</ul>");
            in_list = false;
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(trimmed);
    }
    flush_manual_paragraph(&mut html, &mut paragraph);
    flush_manual_list_item(&mut html, &mut list_item);
    if in_list {
        html.push_str("</ul>");
    }
    html
}

#[component]
fn ManualView(data: RwSignal<Data>) -> impl IntoView {
    view! {
        <div class="view">
            <Panel title="Manuale">
                <div class="manual-body" inner_html=move || {
                    let markdown = if data.get().language == "en" { MANUAL_EN } else { MANUAL_IT };
                    render_manual_markdown(markdown)
                }></div>
            </Panel>
        </div>
    }
}

#[component]
fn LicenseView() -> impl IntoView {
    view! {
        <div class="view">
            <Panel title="Licenza">
                <div class="stack">
                    <p>{ctx_tr("Rextto — media daemon.")}</p>
                    <p class="muted">{ctx_tr("Licenza EUPL-1.2. Il testo completo della licenza è riportato qui sotto.")}</p>
                    <pre style="white-space:pre-wrap;max-height:70vh;overflow:auto;padding:16px;border:1px solid var(--border-color);border-radius:8px;background:var(--bg-secondary);font:0.82rem/1.5 ui-monospace,SFMono-Regular,Menlo,monospace">{FULL_EUPL_LICENSE}</pre>
                </div>
            </Panel>
        </div>
    }
}
