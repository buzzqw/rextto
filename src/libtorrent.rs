use crate::{
    config::{Config, LibtorrentSettings},
    models::{FileView, PeerView, TorrentEvent, TorrentView, TrackerView},
    utils::{atomic_write, magnet_hash, sanitize_magnet},
};
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::{c_char, c_void, CStr, CString},
    fs,
    path::{Path, PathBuf},
    sync::RwLock,
};

unsafe extern "C" {
    fn rextto_lt_create(
        port_min: u16,
        port_max: u16,
        download_limit: i32,
        upload_limit: i32,
        active_downloads: i32,
        active_seeds: i32,
        active_limit: i32,
        connections_limit: i32,
        dht: u8,
        pex: u8,
        lsd: u8,
        upnp: u8,
        natpmp: u8,
        error: *mut c_char,
        error_size: usize,
    ) -> *mut c_void;
    fn rextto_lt_destroy(session: *mut c_void);
    fn rextto_lt_version(output: *mut c_char, output_size: usize);
    fn rextto_lt_add(
        session: *mut c_void,
        magnet: *const c_char,
        save_path: *const c_char,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_add_file(
        session: *mut c_void,
        torrent_path: *const c_char,
        save_path: *const c_char,
        hash: *mut c_char,
        hash_size: usize,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_torrent_count(session: *const c_void) -> u32;
    fn rextto_lt_statuses(
        session: *const c_void,
        output: *mut NativeTorrentStatus,
        capacity: usize,
    ) -> usize;
    fn rextto_lt_events(
        session: *mut c_void,
        output: *mut NativeTorrentEvent,
        capacity: usize,
    ) -> usize;
    fn rextto_lt_save_torrent(
        session: *mut c_void,
        hash: *const c_char,
        path: *const c_char,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_promote_metadata(session: *mut c_void);
    fn rextto_lt_ensure_auto_managed(session: *mut c_void) -> usize;
    fn rextto_lt_adjust_queue(
        session: *mut c_void,
        enabled: i32,
        static_downloads: i32,
        minimum: i32,
        maximum: i32,
        static_seeds: i32,
        static_limit: i32,
        global_download_limit: i32,
    );
    fn rextto_lt_peers(
        session: *mut c_void,
        hash: *const c_char,
        output: *mut NativePeer,
        capacity: usize,
        error: *mut c_char,
        error_size: usize,
    ) -> usize;
    fn rextto_lt_trackers(
        session: *mut c_void,
        hash: *const c_char,
        output: *mut NativeTracker,
        capacity: usize,
        error: *mut c_char,
        error_size: usize,
    ) -> usize;
    fn rextto_lt_files(
        session: *mut c_void,
        hash: *const c_char,
        output: *mut NativeFile,
        capacity: usize,
        error: *mut c_char,
        error_size: usize,
    ) -> usize;
    fn rextto_lt_move_storage(
        session: *mut c_void,
        hash: *const c_char,
        destination: *const c_char,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_set_paused(
        session: *mut c_void,
        hash: *const c_char,
        paused: i32,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_remove(
        session: *mut c_void,
        hash: *const c_char,
        delete_files: i32,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_force_recheck(
        session: *mut c_void,
        hash: *const c_char,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_reannounce(
        session: *mut c_void,
        hash: *const c_char,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_set_limits(
        session: *mut c_void,
        hash: *const c_char,
        download_limit: i32,
        upload_limit: i32,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_restore(
        session: *mut c_void,
        state_dir: *const c_char,
        error: *mut c_char,
        error_size: usize,
    ) -> usize;
    fn rextto_lt_save_resume(
        session: *mut c_void,
        state_dir: *const c_char,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_apply_settings(
        session: *mut c_void,
        settings: *const c_char,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_load_ipfilter(
        session: *mut c_void,
        path: *const c_char,
        rules_out: *mut i32,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_set_pin(
        session: *mut c_void,
        hash: *const c_char,
        pinned: i32,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
    fn rextto_lt_set_sequential(
        session: *mut c_void,
        enabled: i32,
        error: *mut c_char,
        error_size: usize,
    ) -> i32;
}

/// Converte le impostazioni libtorrent extra (righe `chiave=valore`) nei
/// comandi per il bridge: numeri come `i:`, booleani come `b:`, testi come `s:`.
/// Righe vuote o con `#` sono ignorate.
fn extra_settings_lines(raw: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key.is_empty() || value.is_empty() {
            continue;
        }
        let lowered = value.to_ascii_lowercase();
        if matches!(
            lowered.as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off"
        ) {
            let flag = matches!(lowered.as_str(), "true" | "yes" | "on");
            lines.push(format!("b:{key}={}", if flag { 1 } else { 0 }));
        } else if let Ok(number) = value.parse::<i64>() {
            lines.push(format!("i:{key}={number}"));
        } else {
            lines.push(format!("s:{key}={value}"));
        }
    }
    lines
}

pub fn libtorrent_version() -> String {
    let mut buffer = [0_i8; 128];
    unsafe { rextto_lt_version(buffer.as_mut_ptr(), buffer.len()) };
    native_string(&buffer)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeTorrentStatus {
    hash: [c_char; 65],
    name: [c_char; 512],
    save_path: [c_char; 1024],
    progress: f64,
    state: i32,
    paused: i32,
    download_rate: i32,
    upload_rate: i32,
    num_peers: i32,
    num_seeds: i32,
    download_limit: i32,
    upload_limit: i32,
    all_time_upload: i64,
    all_time_download: i64,
    seeding_seconds: i64,
    queue_position: i32,
    has_metadata: i32,
    auto_managed: i32,
    torrent_version: i32,
    total_size: i64,
    total_done: i64,
}

impl Default for NativeTorrentStatus {
    fn default() -> Self {
        Self {
            hash: [0; 65],
            name: [0; 512],
            save_path: [0; 1024],
            progress: 0.0,
            state: 0,
            paused: 0,
            download_rate: 0,
            upload_rate: 0,
            num_peers: 0,
            num_seeds: 0,
            download_limit: -1,
            upload_limit: -1,
            all_time_upload: 0,
            all_time_download: 0,
            seeding_seconds: 0,
            queue_position: -1,
            has_metadata: 0,
            auto_managed: 0,
            torrent_version: 0,
            total_size: 0,
            total_done: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeTorrentEvent {
    kind: i32,
    hash: [c_char; 65],
    name: [c_char; 512],
    save_path: [c_char; 1024],
}

impl Default for NativeTorrentEvent {
    fn default() -> Self {
        Self {
            kind: 0,
            hash: [0; 65],
            name: [0; 512],
            save_path: [0; 1024],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativePeer {
    address: [c_char; 96],
    client: [c_char; 256],
    download_rate: i32,
    upload_rate: i32,
    num_pieces: i32,
    seed: i32,
}

impl Default for NativePeer {
    fn default() -> Self {
        Self {
            address: [0; 96],
            client: [0; 256],
            download_rate: 0,
            upload_rate: 0,
            num_pieces: 0,
            seed: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeTracker {
    url: [c_char; 640],
    tier: i32,
    status: i32,
}

impl Default for NativeTracker {
    fn default() -> Self {
        Self {
            url: [0; 640],
            tier: 0,
            status: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeFile {
    path: [c_char; 1024],
    size: i64,
    downloaded: i64,
}

impl Default for NativeFile {
    fn default() -> Self {
        Self {
            path: [0; 1024],
            size: 0,
            downloaded: 0,
        }
    }
}

#[derive(Debug)]
struct NativeSession(*mut c_void);
unsafe impl Send for NativeSession {}
unsafe impl Sync for NativeSession {}
impl Drop for NativeSession {
    fn drop(&mut self) {
        unsafe { rextto_lt_destroy(self.0) }
    }
}

fn native_error(buffer: &[c_char]) -> String {
    unsafe {
        CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

fn native_string(buffer: &[c_char]) -> String {
    unsafe {
        CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

fn native_state(state: i32, paused: bool) -> String {
    if paused {
        return "paused".into();
    }
    match state {
        1 => "checking_files",
        2 => "downloading_metadata",
        3 => "downloading",
        4 => "finished",
        5 => "seeding",
        7 => "checking_resume_data",
        _ => "unknown",
    }
    .into()
}

#[derive(Debug)]
pub struct LibtorrentClient {
    torrents: RwLock<BTreeMap<String, TorrentView>>,
    session: Option<NativeSession>,
    config_db: PathBuf,
    state_dir: PathBuf,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct SeedLimit {
    ratio: f64,
    days: i64,
}

/// Free space in bytes for the filesystem containing `path`.
pub fn free_space_bytes(path: &Path) -> Option<u64> {
    let raw = CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let mut stats = unsafe { std::mem::zeroed::<libc::statvfs>() };
    let ok = unsafe { libc::statvfs(raw.as_ptr(), &mut stats) } == 0;
    ok.then(|| (stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

/// True when `save_path` lies inside the RAM disk directory (component-wise,
/// following symlinks when possible).
pub fn path_on_ramdisk(save_path: &Path, ramdisk: &Path) -> bool {
    let resolve = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let save_path = resolve(save_path);
    let ramdisk = resolve(ramdisk);
    save_path == ramdisk || save_path.starts_with(&ramdisk)
}

/// legacy parity: decide whether a torrent of `total_size` bytes fits on the RAM
/// disk. `uncommitted_bytes` is the space still to be written there by *other*
/// in-flight torrents, so two downloads cannot saturate the tmpfs together.
pub fn ramdisk_fits(
    threshold_bytes: u64,
    margin_bytes: u64,
    free_bytes: u64,
    uncommitted_bytes: u64,
    total_size: u64,
) -> std::result::Result<(), String> {
    if threshold_bytes > 0 && total_size > threshold_bytes {
        return Err(format!(
            "file too large for the RAM disk: {:.2} GB > threshold {:.2} GB",
            total_size as f64 / 1024_f64.powi(3),
            threshold_bytes as f64 / 1024_f64.powi(3)
        ));
    }
    let effective_free = free_bytes.saturating_sub(uncommitted_bytes);
    let required = total_size.saturating_add(margin_bytes);
    if effective_free < required {
        return Err(format!(
            "insufficient RAM disk space (free {:.2} GB, reserved by other downloads {:.2} GB, required {:.2} GB)",
            free_bytes as f64 / 1024_f64.powi(3),
            uncommitted_bytes as f64 / 1024_f64.powi(3),
            required as f64 / 1024_f64.powi(3),
        ));
    }
    Ok(())
}

impl LibtorrentClient {
    /// Download directory for automatic acquisitions: RAM disk first (size still
    /// unknown), then the temporary disk, then the final directory. A RAM disk
    /// that no longer holds its safety margin is skipped so it cannot fill up.
    fn preferred_download_path(cfg: &Config) -> PathBuf {
        let mut candidates = Vec::new();
        if cfg.ramdisk_enabled() {
            if let Some(dir) = cfg.ramdisk_dir() {
                candidates.push(dir);
            }
        }
        if let Some(path) = cfg.libtorrent_temp_dir.clone() {
            candidates.push(path);
        }
        candidates.push(cfg.libtorrent_dir.clone());
        let explicit = cfg.ramdisk_min_free_bytes();
        let minimum_free = if explicit > 0 {
            explicit
        } else {
            cfg.ramdisk_margin_bytes()
        };
        candidates
            .into_iter()
            .find(|path| {
                if !path.is_dir() {
                    return false;
                }
                free_space_bytes(path).is_some_and(|free| free >= minimum_free)
            })
            .unwrap_or_else(|| cfg.libtorrent_dir.clone())
    }

    /// Bytes still to be written on the RAM disk by torrents other than
    /// `exclude_hash`.
    pub fn ramdisk_uncommitted_bytes(&self, ramdisk: &Path, exclude_hash: &str) -> u64 {
        self.torrents
            .read()
            .unwrap()
            .values()
            .filter(|torrent| !torrent.hash.eq_ignore_ascii_case(exclude_hash))
            .filter(|torrent| path_on_ramdisk(Path::new(&torrent.save_path), ramdisk))
            .map(|torrent| torrent.total_size.saturating_sub(torrent.total_done).max(0) as u64)
            .sum()
    }

    pub fn new(cfg: &Config) -> Result<Self> {
        let dry_run = cfg.dry_run || !cfg.libtorrent_enabled;
        let session = if dry_run {
            None
        } else {
            let mut error = [0_i8; 512];
            let port_min = cfg.libtorrent.port_min;
            let port_max = cfg.libtorrent.port_max;
            let limit = |kib: i64| {
                if kib <= 0 {
                    0
                } else {
                    kib.saturating_mul(1024).min(i32::MAX as i64) as i32
                }
            };
            let raw = unsafe {
                rextto_lt_create(
                    port_min,
                    port_max,
                    limit(cfg.libtorrent.download_limit_kib),
                    limit(cfg.libtorrent.upload_limit_kib),
                    cfg.libtorrent.active_downloads.clamp(1, i32::MAX as i64) as i32,
                    cfg.libtorrent.active_seeds.clamp(1, i32::MAX as i64) as i32,
                    cfg.libtorrent.active_limit.clamp(1, i32::MAX as i64) as i32,
                    cfg.settings
                        .get("libtorrent_connections_limit")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(200_i64)
                        .clamp(1, i32::MAX as i64) as i32,
                    cfg.libtorrent.dht as u8,
                    cfg.libtorrent.pex as u8,
                    cfg.libtorrent.lsd as u8,
                    cfg.libtorrent.upnp as u8,
                    cfg.libtorrent.natpmp as u8,
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            if raw.is_null() {
                bail!("cannot create libtorrent session: {}", native_error(&error));
            }
            Some(NativeSession(raw))
        };
        let client = Self {
            torrents: RwLock::new(BTreeMap::new()),
            session,
            config_db: cfg.data_dir.join("rextto_config.db"),
            state_dir: cfg.state_dir.clone(),
            dry_run,
        };
        client.restore(&cfg.state_dir)?;
        client.apply_extended_settings(cfg)?;
        client.recheck_restored_at_zero();
        Ok(client)
    }

    /// After a restart, paused torrents whose progress reads 0 but whose data is on
    /// disk must be verified so the real completion percentage is shown again.
    pub fn recheck_restored_at_zero(&self) -> usize {
        let Some(_) = &self.session else {
            return 0;
        };
        let mut checked = 0;
        for torrent in self.list() {
            let paused = torrent.state.to_lowercase().contains("paus")
                || torrent.state.to_lowercase().contains("coda");
            let data_exists = std::path::Path::new(&torrent.save_path).exists();
            if torrent.has_metadata
                && paused
                && torrent.progress < 0.5
                && data_exists
                && self.force_recheck(&torrent.hash).unwrap_or(false)
            {
                checked += 1;
            }
        }
        if checked > 0 {
            tracing::info!(
                checked,
                "verifying restored paused torrents to recover progress"
            );
        }
        checked
    }

    /// Re-applies the extended libtorrent settings to the running session.
    pub fn apply_settings(&self, cfg: &Config) -> Result<bool> {
        if self.session.is_none() {
            return Ok(false);
        }
        self.apply_extended_settings(cfg)?;
        Ok(true)
    }

    /// Loads a local ipfilter file into the live session, returning the number of rules.
    pub fn load_ipfilter(&self, path: &std::path::Path) -> Result<usize> {
        let Some(session) = &self.session else {
            return Ok(0);
        };
        let path_display = path.display().to_string();
        let path = CString::new(path.to_string_lossy().as_bytes())?;
        let mut rules = 0_i32;
        let mut error = [0_i8; 512];
        let loaded = unsafe {
            rextto_lt_load_ipfilter(
                session.0,
                path.as_ptr(),
                &mut rules,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if loaded == 0 {
            bail!("libtorrent ip filter not applied: {}", native_error(&error));
        }
        let rules = rules.max(0) as usize;
        tracing::info!(rules, path = %path_display, "ip filter loaded");
        Ok(rules)
    }

    /// Applies global download/upload rate limits (KiB/s, 0 = unlimited) to the live session.
    pub fn set_global_speed_limits(&self, download_kib: i64, upload_kib: i64) -> Result<bool> {
        let Some(session) = &self.session else {
            return Ok(false);
        };
        let dl = download_kib.max(0).saturating_mul(1024);
        let ul = upload_kib.max(0).saturating_mul(1024);
        let payload = format!("i:download_rate_limit={dl}\ni:upload_rate_limit={ul}");
        let payload = CString::new(payload)?;
        let mut error = [0_i8; 512];
        let applied = unsafe {
            rextto_lt_apply_settings(session.0, payload.as_ptr(), error.as_mut_ptr(), error.len())
        };
        if applied == 0 {
            bail!(
                "libtorrent speed limits not applied: {}",
                native_error(&error)
            );
        }
        Ok(true)
    }

    fn apply_extended_settings(&self, cfg: &Config) -> Result<()> {
        let Some(session) = &self.session else {
            return Ok(());
        };
        let lt = &cfg.libtorrent;
        // Readable sentence with units, like the legacy extto line, instead of
        // a dump of key: value fields.
        let cache_label = if lt.cache_size > 0 {
            format!("{} MB", lt.cache_size.saturating_mul(16) / 1024)
        } else {
            "auto".to_string()
        };
        let limit_label = |kib: i64| {
            if kib > 0 {
                format!("{kib} KB/s")
            } else {
                "unlimited".to_string()
            }
        };
        tracing::info!(
            "🧠 libtorrent memory: cache {}, max connections {}, I/O threads {}, base bandwidth {} down / {} up",
            cache_label,
            lt.connections_limit,
            lt.aio_threads,
            limit_label(lt.download_limit_kib),
            limit_label(lt.upload_limit_kib)
        );
        let mut lines: Vec<String> = Vec::new();
        let int = |lines: &mut Vec<String>, key: &str, value: i64| {
            if value >= 0 {
                lines.push(format!("i:{key}={value}"));
            }
        };
        let boolean = |lines: &mut Vec<String>, key: &str, value: bool| {
            lines.push(format!("b:{key}={}", if value { 1 } else { 0 }));
        };
        let text = |lines: &mut Vec<String>, key: &str, value: &str| {
            if !value.trim().is_empty() {
                lines.push(format!("s:{key}={}", value.trim()));
            }
        };
        int(&mut lines, "connections_limit", lt.connections_limit);
        int(&mut lines, "upload_slots_limit", lt.upload_slots_limit);
        int(&mut lines, "half_open_limit", lt.half_open_limit);
        int(&mut lines, "alert_queue_size", lt.alert_queue_size);
        int(
            &mut lines,
            "max_connections_per_torrent",
            lt.max_connections_per_torrent,
        );
        int(
            &mut lines,
            "max_uploads_per_torrent",
            lt.max_uploads_per_torrent,
        );
        int(&mut lines, "aio_threads", lt.aio_threads);
        int(&mut lines, "cache_size", lt.cache_size);
        int(&mut lines, "cache_expiry", lt.cache_expiry);
        int(
            &mut lines,
            "download_rate_limit",
            lt.download_limit_kib.max(0).saturating_mul(1024),
        );
        int(
            &mut lines,
            "upload_rate_limit",
            lt.upload_limit_kib.max(0).saturating_mul(1024),
        );
        int(&mut lines, "announce_interval", lt.announce_interval);
        int(
            &mut lines,
            "torrent_connect_boost",
            lt.torrent_connect_boost,
        );
        boolean(&mut lines, "enable_utp", lt.utp);
        boolean(&mut lines, "prefer_rc4", lt.prefer_rc4);
        boolean(
            &mut lines,
            "announce_to_all_trackers",
            lt.announce_to_all_trackers,
        );
        boolean(
            &mut lines,
            "announce_to_all_tiers",
            lt.announce_to_all_tiers,
        );
        boolean(
            &mut lines,
            "allow_multiple_connections_per_ip",
            lt.allow_multiple_connections_per_ip,
        );
        boolean(&mut lines, "apply_ip_filter", lt.apply_ip_filter);
        boolean(
            &mut lines,
            "dont_count_slow_torrents",
            lt.dont_count_slow_torrents,
        );
        let policy = lt.encryption.clamp(0, 2);
        int(&mut lines, "in_enc_policy", policy);
        int(&mut lines, "out_enc_policy", policy);
        if lt.proxy_type > 0 {
            int(&mut lines, "proxy_type", lt.proxy_type);
            text(&mut lines, "proxy_host", &lt.proxy_host);
            int(&mut lines, "proxy_port", lt.proxy_port);
            text(&mut lines, "proxy_username", &lt.proxy_user);
            text(&mut lines, "proxy_password", &lt.proxy_password);
        }
        text(&mut lines, "ip_filter_path", &lt.ip_filter_path);
        text(
            &mut lines,
            "listen_interfaces",
            &effective_listen_interfaces(lt),
        );
        // Killswitch VPN: forza il traffico in uscita sulla scheda scelta.
        text(&mut lines, "outgoing_interfaces", &lt.outgoing_interface);
        text(&mut lines, "dht_bootstrap_nodes", &lt.dht_bootstrap_nodes);
        for line in extra_settings_lines(
            cfg.settings
                .get("libtorrent_extra_settings")
                .map(String::as_str)
                .unwrap_or(""),
        ) {
            lines.push(line);
        }
        if lines.is_empty() {
            return Ok(());
        }
        let payload = lines.join("\n");
        let payload = CString::new(payload)?;
        let mut error = [0_i8; 512];
        let applied = unsafe {
            rextto_lt_apply_settings(session.0, payload.as_ptr(), error.as_mut_ptr(), error.len())
        };
        if applied == 0 {
            tracing::warn!(
                error = %native_error(&error),
                "some libtorrent settings were not applied"
            );
        }
        Ok(())
    }

    pub fn add(&self, magnet: &str, cfg: &Config) -> Result<bool> {
        self.add_with_path(magnet, cfg, None)
    }

    pub fn add_with_path(
        &self,
        magnet: &str,
        cfg: &Config,
        preferred_path: Option<&std::path::Path>,
    ) -> Result<bool> {
        let clean = sanitize_magnet(magnet, None).context("invalid magnet")?;
        let hash = magnet_hash(&clean).context("missing info hash")?;
        if self.torrents.read().unwrap().contains_key(&hash) {
            return Ok(false);
        }
        let save_path = preferred_path
            .filter(|path| path.is_dir())
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| Self::preferred_download_path(cfg));
        if let Some(session) = &self.session {
            let magnet = CString::new(clean.as_str())?;
            fs::create_dir_all(&save_path)?;
            let save_path = CString::new(save_path.to_string_lossy().as_bytes())?;
            let mut error = [0_i8; 512];
            let added = unsafe {
                rextto_lt_add(
                    session.0,
                    magnet.as_ptr(),
                    save_path.as_ptr(),
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            if added == 0 {
                bail!("libtorrent add failed: {}", native_error(&error));
            }
            self.apply_stored_limits(session, &hash)?;
        } else if self.dry_run {
            tracing::info!(%hash, "dry-run: torrent accepted, not started");
        } else {
            bail!("libtorrent session is unavailable");
        }
        let save_path = save_path;
        self.torrents.write().unwrap().insert(
            hash.clone(),
            TorrentView {
                hash,
                name: clean,
                progress: 0.0,
                state: if self.dry_run {
                    "dry-run".into()
                } else {
                    "downloading_metadata".into()
                },
                download_rate: 0,
                upload_rate: 0,
                save_path: save_path.display().to_string(),
                download_limit: -1,
                upload_limit: -1,
                all_time_upload: 0,
                all_time_download: 0,
                seeding_seconds: 0,
                queue_position: -1,
                num_peers: 0,
                num_seeds: 0,
                seed_ratio: -1.0,
                seed_days: -1,
                has_metadata: false,
                auto_managed: false,
                torrent_version: String::new(),
                total_size: 0,
                total_done: 0,
            },
        );
        Ok(true)
    }

    pub fn add_torrent_file(
        &self,
        torrent_path: &std::path::Path,
        save_path: &std::path::Path,
    ) -> Result<Option<String>> {
        let Some(session) = &self.session else {
            return Ok(None);
        };
        let torrent_path = CString::new(torrent_path.to_string_lossy().as_bytes())?;
        let save_path = CString::new(save_path.to_string_lossy().as_bytes())?;
        let mut hash = [0_i8; 65];
        let mut error = [0_i8; 512];
        let added = unsafe {
            rextto_lt_add_file(
                session.0,
                torrent_path.as_ptr(),
                save_path.as_ptr(),
                hash.as_mut_ptr(),
                hash.len(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if added == 0 {
            bail!(
                "libtorrent torrent-file add failed: {}",
                native_error(&error)
            );
        }
        Ok(Some(native_string(&hash)))
    }

    /// Aggiunge un `.torrent` da file scegliendo la cartella come
    /// [`Self::add_with_path`]. Usato dai feed RSS che espongono solo il
    /// download `.torrent`: i tracker privati richiedono il file per l'announce.
    pub fn add_file_with_path(
        &self,
        torrent_path: &std::path::Path,
        cfg: &Config,
        preferred_path: Option<&std::path::Path>,
    ) -> Result<bool> {
        let save_path = preferred_path
            .filter(|path| path.is_dir())
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| Self::preferred_download_path(cfg));
        fs::create_dir_all(&save_path)?;
        match self.add_torrent_file(torrent_path, &save_path)? {
            Some(_) => Ok(true),
            None if self.dry_run => {
                tracing::info!("dry-run: torrent file accepted, not started");
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn list(&self) -> Vec<TorrentView> {
        let Some(session) = &self.session else {
            return self.torrents.read().unwrap().values().cloned().collect();
        };
        let count = unsafe { rextto_lt_statuses(session.0, std::ptr::null_mut(), 0) };
        let mut statuses = vec![NativeTorrentStatus::default(); count];
        let received =
            unsafe { rextto_lt_statuses(session.0, statuses.as_mut_ptr(), statuses.len()) };
        let limits = match self.seed_limits() {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, "cannot read per-torrent seed limits");
                BTreeMap::new()
            }
        };
        statuses
            .into_iter()
            .take(received)
            .map(|status| {
                let hash = native_string(&status.hash).to_ascii_lowercase();
                let has_limit = limits.contains_key(&hash);
                let limit = limits.get(&hash).copied().unwrap_or_default();
                TorrentView {
                    hash,
                    name: native_string(&status.name),
                    save_path: native_string(&status.save_path),
                    progress: status.progress.clamp(0.0, 100.0),
                    state: native_state(status.state, status.paused != 0),
                    download_rate: status.download_rate.max(0) as u64,
                    upload_rate: status.upload_rate.max(0) as u64,
                    download_limit: status.download_limit as i64,
                    upload_limit: status.upload_limit as i64,
                    all_time_upload: status.all_time_upload,
                    all_time_download: status.all_time_download,
                    seeding_seconds: status.seeding_seconds,
                    queue_position: status.queue_position,
                    num_peers: status.num_peers,
                    num_seeds: status.num_seeds,
                    seed_ratio: if has_limit { limit.ratio } else { -1.0 },
                    seed_days: if has_limit { limit.days } else { -1 },
                    has_metadata: status.has_metadata != 0,
                    auto_managed: status.auto_managed != 0,
                    torrent_version: match status.torrent_version {
                        1 => "v1".to_string(),
                        2 => "v2".to_string(),
                        3 => "hybrid".to_string(),
                        _ => String::new(),
                    },
                    total_size: status.total_size.max(0),
                    total_done: status.total_done.max(0),
                }
            })
            .collect()
    }
    pub fn poll_events(&self) -> Vec<TorrentEvent> {
        let Some(session) = &self.session else {
            return Vec::new();
        };
        let mut native = vec![NativeTorrentEvent::default(); 256];
        let received = unsafe { rextto_lt_events(session.0, native.as_mut_ptr(), native.len()) };
        native
            .into_iter()
            .take(received)
            .filter_map(|event| {
                let hash = native_string(&event.hash).to_ascii_lowercase();
                let kind = match event.kind {
                    1 => "metadata_received",
                    2 => "torrent_finished",
                    3 => "storage_moved",
                    4 => "storage_move_failed",
                    _ => "unknown",
                };
                if kind == "metadata_received" {
                    if let Err(error) = self.save_torrent_metadata(&hash) {
                        tracing::warn!(hash=%hash, %error, "cannot persist torrent metadata");
                    }
                }
                Some(TorrentEvent {
                    kind: kind.into(),
                    hash,
                    name: native_string(&event.name),
                    save_path: native_string(&event.save_path),
                })
            })
            .collect()
    }

    fn save_torrent_metadata(&self, hash: &str) -> Result<()> {
        let Some(session) = &self.session else {
            return Ok(());
        };
        fs::create_dir_all(&self.state_dir)?;
        let hash_string = hash.to_ascii_lowercase();
        let hash = CString::new(hash_string.as_str())?;
        let path = CString::new(
            self.state_dir
                .join(format!("{hash_string}.torrent"))
                .to_string_lossy()
                .as_bytes(),
        )?;
        let mut error = [0_i8; 512];
        let saved = unsafe {
            rextto_lt_save_torrent(
                session.0,
                hash.as_ptr(),
                path.as_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if saved == 0 {
            bail!("libtorrent metadata save failed: {}", native_error(&error));
        }
        Ok(())
    }

    pub fn promote_metadata(&self) {
        if let Some(session) = &self.session {
            unsafe { rextto_lt_promote_metadata(session.0) };
        }
    }

    /// Riarma l'auto-gestione della coda sui torrent che hanno i metadati e non
    /// sono in pausa. I torrent ripristinati da fastresume con
    /// `auto_managed=false` altrimenti aggirano `active_downloads`/`active_limit`
    /// per sempre. I torrent lasciati in pausa dall'utente restano intatti.
    pub fn ensure_auto_managed(&self) -> usize {
        if let Some(session) = &self.session {
            return unsafe { rextto_lt_ensure_auto_managed(session.0) };
        }
        0
    }

    pub fn adjust_queue(&self, cfg: &Config) {
        let Some(session) = &self.session else {
            return;
        };
        let to_bytes = |kib: i64| kib.saturating_mul(1024).clamp(0, i32::MAX as i64) as i32;
        unsafe {
            rextto_lt_adjust_queue(
                session.0,
                cfg.libtorrent.dynamic_queue as i32,
                cfg.libtorrent.active_downloads.clamp(1, i32::MAX as i64) as i32,
                cfg.libtorrent.dynamic_queue_min.clamp(1, i32::MAX as i64) as i32,
                cfg.libtorrent.dynamic_queue_max.clamp(1, i32::MAX as i64) as i32,
                cfg.libtorrent.active_seeds.clamp(1, i32::MAX as i64) as i32,
                cfg.libtorrent.active_limit.clamp(1, i32::MAX as i64) as i32,
                to_bytes(cfg.libtorrent.download_limit_kib),
            )
        };
    }

    pub fn move_storage(&self, hash: &str, destination: &std::path::Path) -> Result<bool> {
        let Some(session) = &self.session else {
            return Ok(false);
        };
        let hash = CString::new(hash.to_ascii_lowercase())?;
        let destination = CString::new(destination.to_string_lossy().as_bytes())?;
        let mut error = [0_i8; 512];
        let moved = unsafe {
            rextto_lt_move_storage(
                session.0,
                hash.as_ptr(),
                destination.as_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if moved == 0 {
            bail!("libtorrent move storage failed: {}", native_error(&error));
        }
        Ok(true)
    }

    pub fn peers(&self, hash: &str) -> Result<Option<Vec<PeerView>>> {
        let Some(session) = &self.session else {
            return Ok(None);
        };
        let hash = CString::new(hash.to_ascii_lowercase())?;
        let mut native = vec![NativePeer::default(); 256];
        let mut error = [0_i8; 512];
        let count = unsafe {
            rextto_lt_peers(
                session.0,
                hash.as_ptr(),
                native.as_mut_ptr(),
                native.len(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if count == 0 && !native_error(&error).is_empty() {
            bail!("libtorrent peer query failed: {}", native_error(&error));
        }
        Ok(Some(
            native
                .into_iter()
                .take(count)
                .map(|peer| PeerView {
                    address: native_string(&peer.address),
                    client: native_string(&peer.client),
                    download_rate: peer.download_rate.max(0) as u64,
                    upload_rate: peer.upload_rate.max(0) as u64,
                    pieces: peer.num_pieces,
                    seed: peer.seed != 0,
                })
                .collect(),
        ))
    }

    pub fn trackers(&self, hash: &str) -> Result<Option<Vec<TrackerView>>> {
        let Some(session) = &self.session else {
            return Ok(None);
        };
        let hash = CString::new(hash.to_ascii_lowercase())?;
        let mut native = vec![NativeTracker::default(); 256];
        let mut error = [0_i8; 512];
        let count = unsafe {
            rextto_lt_trackers(
                session.0,
                hash.as_ptr(),
                native.as_mut_ptr(),
                native.len(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if count == 0 && !native_error(&error).is_empty() {
            bail!("libtorrent tracker query failed: {}", native_error(&error));
        }
        Ok(Some(
            native
                .into_iter()
                .take(count)
                .map(|tracker| TrackerView {
                    url: native_string(&tracker.url),
                    tier: tracker.tier,
                })
                .collect(),
        ))
    }

    pub fn files(&self, hash: &str) -> Result<Option<Vec<FileView>>> {
        let Some(session) = &self.session else {
            return Ok(None);
        };
        let hash = CString::new(hash.to_ascii_lowercase())?;
        let mut native = vec![NativeFile::default(); 2048];
        let mut error = [0_i8; 512];
        let count = unsafe {
            rextto_lt_files(
                session.0,
                hash.as_ptr(),
                native.as_mut_ptr(),
                native.len(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if count == 0 && !native_error(&error).is_empty() {
            bail!("libtorrent file query failed: {}", native_error(&error));
        }
        Ok(Some(
            native
                .into_iter()
                .take(count)
                .map(|file| FileView {
                    path: native_string(&file.path),
                    size: file.size,
                    downloaded: file.downloaded,
                })
                .collect(),
        ))
    }

    fn control(
        &self,
        hash: &str,
        action: unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut c_char, usize) -> i32,
        value: i32,
    ) -> Result<bool> {
        let Some(session) = &self.session else {
            return Ok(false);
        };
        let hash = CString::new(hash.to_ascii_lowercase())?;
        let mut error = [0_i8; 512];
        let ok = unsafe {
            action(
                session.0,
                hash.as_ptr(),
                value,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok == 0 {
            bail!("libtorrent action failed: {}", native_error(&error));
        }
        Ok(true)
    }

    pub fn pause(&self, hash: &str) -> Result<bool> {
        self.control(hash, rextto_lt_set_paused, 1)
    }
    pub fn resume(&self, hash: &str) -> Result<bool> {
        self.control(hash, rextto_lt_set_paused, 0)
    }
    /// Restart a torrent without removing its handle, data, or resume state.
    /// A tracker announce follows the pause/resume transition to immediately
    /// refresh peer discovery.
    pub fn restart(&self, hash: &str) -> Result<bool> {
        if !self.pause(hash)? {
            return Ok(false);
        }
        self.resume(hash)?;
        self.reannounce(hash)
    }
    pub fn set_pin(&self, hash: &str, pinned: bool) -> Result<bool> {
        let Some(session) = &self.session else {
            return Ok(false);
        };
        let hash = CString::new(hash.to_ascii_lowercase())?;
        let mut error = [0_i8; 512];
        let ok = unsafe {
            rextto_lt_set_pin(
                session.0,
                hash.as_ptr(),
                if pinned { 1 } else { 0 },
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok == 0 {
            bail!("libtorrent pin failed: {}", native_error(&error));
        }
        Ok(true)
    }
    pub fn set_sequential(&self, enabled: bool) -> Result<bool> {
        let Some(session) = &self.session else {
            return Ok(false);
        };
        let mut error = [0_i8; 512];
        let ok = unsafe {
            rextto_lt_set_sequential(
                session.0,
                if enabled { 1 } else { 0 },
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok == 0 {
            bail!("libtorrent sequential failed: {}", native_error(&error));
        }
        Ok(true)
    }
    pub fn force_recheck(&self, hash: &str) -> Result<bool> {
        let Some(session) = &self.session else {
            return Ok(false);
        };
        let hash = CString::new(hash.to_ascii_lowercase())?;
        let mut error = [0_i8; 512];
        let ok = unsafe {
            rextto_lt_force_recheck(session.0, hash.as_ptr(), error.as_mut_ptr(), error.len())
        };
        if ok == 0 {
            bail!("libtorrent recheck failed: {}", native_error(&error));
        }
        Ok(true)
    }
    pub fn reannounce(&self, hash: &str) -> Result<bool> {
        let Some(session) = &self.session else {
            return Ok(false);
        };
        let hash = CString::new(hash.to_ascii_lowercase())?;
        let mut error = [0_i8; 512];
        let ok = unsafe {
            rextto_lt_reannounce(session.0, hash.as_ptr(), error.as_mut_ptr(), error.len())
        };
        if ok == 0 {
            bail!("libtorrent reannounce failed: {}", native_error(&error));
        }
        Ok(true)
    }
    fn stored_limits(&self, hash: &str) -> Result<Option<(i32, i32)>> {
        if !self.config_db.exists() {
            return Ok(None);
        }
        let conn = crate::config::open_config_db(&self.config_db)?;
        conn.query_row(
            "SELECT dl_bytes,ul_bytes FROM torrent_limits WHERE lower(info_hash)=?1",
            [hash.to_ascii_lowercase()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(Into::into)
    }

    fn seed_limits_path(&self) -> PathBuf {
        self.state_dir.join("seed_limits.json")
    }

    fn seed_limits(&self) -> Result<BTreeMap<String, SeedLimit>> {
        let path = self.seed_limits_path();
        if !path.is_file() {
            return Ok(BTreeMap::new());
        }
        let stored = serde_json::from_slice::<BTreeMap<String, SeedLimit>>(&fs::read(path)?)?;
        Ok(stored
            .into_iter()
            .map(|(hash, limit)| (hash.to_ascii_lowercase(), limit))
            .collect())
    }

    fn save_seed_limit(&self, hash: &str, ratio: f64, days: i64) -> Result<()> {
        fs::create_dir_all(&self.state_dir)?;
        let mut limits = self.seed_limits()?;
        limits.insert(hash.to_ascii_lowercase(), SeedLimit { ratio, days });
        atomic_write(
            &self.seed_limits_path(),
            &serde_json::to_vec_pretty(&limits)?,
        )
    }
    fn apply_stored_limits(&self, session: &NativeSession, hash: &str) -> Result<()> {
        let Some((download_limit, upload_limit)) = self.stored_limits(hash)? else {
            return Ok(());
        };
        let hash = CString::new(hash)?;
        let mut error = [0_i8; 512];
        let ok = unsafe {
            rextto_lt_set_limits(
                session.0,
                hash.as_ptr(),
                download_limit,
                upload_limit,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok == 0 {
            bail!(
                "libtorrent apply saved limits failed: {}",
                native_error(&error)
            );
        }
        Ok(())
    }
    pub fn set_limits(
        &self,
        hash: &str,
        download_limit: i64,
        upload_limit: i64,
        seed_ratio: f64,
        seed_days: i64,
    ) -> Result<bool> {
        let Some(session) = &self.session else {
            return Ok(false);
        };
        if download_limit < -1
            || upload_limit < -1
            || download_limit > i32::MAX as i64
            || upload_limit > i32::MAX as i64
        {
            bail!("limits must be between -1 and {} bytes/s", i32::MAX);
        }
        if !seed_ratio.is_finite() || seed_ratio < -1.0 || seed_days < -1 {
            bail!("seed limits must be -1, 0, or a positive value");
        }
        let normalized = hash.to_ascii_lowercase();
        let hash = CString::new(normalized.as_str())?;
        let mut error = [0_i8; 512];
        let ok = unsafe {
            rextto_lt_set_limits(
                session.0,
                hash.as_ptr(),
                download_limit as i32,
                upload_limit as i32,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok == 0 {
            bail!("libtorrent set limits failed: {}", native_error(&error));
        }
        let conn = crate::config::open_config_db(&self.config_db)?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS torrent_limits (info_hash TEXT PRIMARY KEY, dl_bytes INTEGER NOT NULL DEFAULT -1, ul_bytes INTEGER NOT NULL DEFAULT -1, updated_at TEXT NOT NULL DEFAULT (datetime('now')));")?;
        conn.execute("INSERT INTO torrent_limits(info_hash,dl_bytes,ul_bytes,updated_at) VALUES (?1,?2,?3,datetime('now')) ON CONFLICT(info_hash) DO UPDATE SET dl_bytes=excluded.dl_bytes,ul_bytes=excluded.ul_bytes,updated_at=excluded.updated_at", params![normalized, download_limit, upload_limit])?;
        if seed_ratio >= 0.0 || seed_days >= 0 {
            self.save_seed_limit(&normalized, seed_ratio, seed_days)?;
        }
        Ok(true)
    }
    fn restore(&self, state_dir: &std::path::Path) -> Result<usize> {
        let Some(session) = &self.session else {
            return Ok(0);
        };
        let state_dir = CString::new(state_dir.to_string_lossy().as_bytes())?;
        let mut error = [0_i8; 512];
        let restored = unsafe {
            rextto_lt_restore(
                session.0,
                state_dir.as_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let warning = native_error(&error);
        if !warning.is_empty() {
            tracing::warn!(%warning, "some fastresume files were not restored");
        }
        if restored > 0 {
            tracing::info!(
                "♻️ libtorrent state restored: {} torrent(s) resume where they left off",
                restored
            );
        }
        Ok(restored)
    }
    pub fn shutdown(&self, cfg: &Config) -> Result<()> {
        let Some(session) = &self.session else {
            return Ok(());
        };
        let state_dir = CString::new(cfg.state_dir.to_string_lossy().as_bytes())?;
        let mut error = [0_i8; 512];
        let saved = unsafe {
            rextto_lt_save_resume(
                session.0,
                state_dir.as_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if saved == 0 {
            bail!(
                "libtorrent fastresume shutdown failed: {}",
                native_error(&error)
            );
        }
        tracing::info!("libtorrent shutdown: resume data saved");
        Ok(())
    }
    pub fn remove(&self, hash: &str, delete_files: bool) -> Result<bool> {
        if self.session.is_some() {
            self.control(hash, rextto_lt_remove, delete_files as i32)?;
        }
        let normalized = hash.to_ascii_lowercase();
        // Il resume va rimosso: altrimenti al riavvio libtorrent ripristina il
        // torrent e "riappare" in Scarico, annullando la rimozione.
        let _ = std::fs::remove_file(self.state_dir.join(format!("{normalized}.fastresume")));
        let _ = std::fs::remove_file(self.state_dir.join(format!("{normalized}.torrent")));
        Ok(self
            .torrents
            .write()
            .unwrap()
            .remove(&normalized)
            .is_some()
            || self.session.is_some())
    }
    pub fn stats(&self) -> serde_json::Value {
        let list = self.list();
        let native_count = self
            .session
            .as_ref()
            .map(|session| unsafe { rextto_lt_torrent_count(session.0) })
            .unwrap_or(0);
        serde_json::json!({"count": list.len(), "native_count": native_count, "downloading": list.iter().filter(|torrent| torrent.state == "downloading").count(), "seeding": list.iter().filter(|torrent| torrent.state == "seeding").count(), "queued": list.iter().filter(|torrent| torrent.state == "paused" && torrent.has_metadata).count(), "metadata_pending": list.iter().filter(|torrent| !torrent.has_metadata).count(), "integrated": self.session.is_some(), "dry_run": self.dry_run})
    }
    pub fn state_dir(&self, cfg: &Config) -> PathBuf {
        cfg.state_dir.clone()
    }
}

/// `listen_interfaces` effettivo. Se è configurata un'interfaccia di uscita
/// (killswitch VPN) e l'ascolto non specifica già una porta, l'ascolto viene
/// legato alla stessa scheda, come nel legacy EXTTO (`iface:porta_min`).
fn effective_listen_interfaces(lt: &LibtorrentSettings) -> String {
    let listen = lt.listen_interfaces.trim();
    let outgoing = lt.outgoing_interface.trim();
    if !listen.is_empty() {
        // Nome di interfaccia "nudo" senza porta (es. `eth0`, `wg0`, `tun0`).
        if !listen.contains(':') && !listen.contains(',') && !listen.contains('[') {
            return format!("{listen}:{}", lt.port_min);
        }
        return listen.to_owned();
    }
    if !outgoing.is_empty() {
        return format!("{outgoing}:{}", lt.port_min);
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn ramdisk_capacity_respects_threshold_margin_and_reservations() {
        let gib = 1024_u64 * 1024 * 1024;
        // 2 GB file with 6 GB free, 0.5 GB margin: fits.
        assert!(ramdisk_fits(5 * gib, gib / 2, 6 * gib, 0, 2 * gib).is_ok());
        // Same file above the per-torrent threshold: rejected.
        let too_big = ramdisk_fits(gib, gib / 2, 6 * gib, 0, 2 * gib);
        assert!(too_big.unwrap_err().contains("too large"));
        // Free space reserved by another download leaves too little: rejected.
        let reserved = ramdisk_fits(5 * gib, gib / 2, 6 * gib, 5 * gib, 2 * gib);
        assert!(reserved.unwrap_err().contains("insufficient"));
    }

    #[test]
    fn ramdisk_path_check_is_component_wise() {
        assert!(path_on_ramdisk(
            Path::new("/mnt/ramdisk/show/file.mkv"),
            Path::new("/mnt/ramdisk")
        ));
        assert!(path_on_ramdisk(
            Path::new("/mnt/ramdisk"),
            Path::new("/mnt/ramdisk")
        ));
        // A sibling directory with a shared prefix must not match.
        assert!(!path_on_ramdisk(
            Path::new("/mnt/ramdisk2/file.mkv"),
            Path::new("/mnt/ramdisk")
        ));
        assert!(!path_on_ramdisk(
            Path::new("/mnt/downloads/file.mkv"),
            Path::new("/mnt/ramdisk")
        ));
    }

    #[test]
    fn ramdisk_settings_are_read_from_the_config_db() {
        let mut cfg = Config::default();
        cfg.settings
            .insert("libtorrent_ramdisk_dir".into(), "/mnt/ramdisk".into());
        cfg.settings
            .insert("libtorrent_ramdisk_enabled".into(), "yes".into());
        cfg.settings
            .insert("libtorrent_ramdisk_threshold_gb".into(), "5.5".into());
        cfg.settings
            .insert("libtorrent_ramdisk_margin_gb".into(), "0.5".into());
        cfg.settings
            .insert("libtorrent_ramdisk_min_free_bytes".into(), "1024".into());
        assert!(cfg.ramdisk_enabled());
        assert_eq!(
            cfg.ramdisk_dir().as_deref(),
            Some(Path::new("/mnt/ramdisk"))
        );
        let gib = 1024_u64 * 1024 * 1024;
        assert_eq!(cfg.ramdisk_threshold_bytes(), (5.5 * gib as f64) as u64);
        assert_eq!(cfg.ramdisk_margin_bytes(), gib / 2);
        assert_eq!(cfg.ramdisk_min_free_bytes(), 1024);

        cfg.settings
            .insert("libtorrent_ramdisk_enabled".into(), "no".into());
        assert!(!cfg.ramdisk_enabled());
    }

    #[test]
    fn extra_settings_lines_parse_numbers_booleans_and_strings() {
        let raw = "# commento\nconnections_limit=200\nsmooth_connects=true\ndont_count_slow_torrents=no\nproxy_host=127.0.0.1\n\nbadline\n";
        let lines = extra_settings_lines(raw);
        assert_eq!(
            lines,
            vec![
                "i:connections_limit=200".to_string(),
                "b:smooth_connects=1".to_string(),
                "b:dont_count_slow_torrents=0".to_string(),
                "s:proxy_host=127.0.0.1".to_string(),
            ]
        );
        assert!(extra_settings_lines("   ").is_empty());
    }

    #[test]
    fn dry_run_never_creates_a_native_session_or_controls_torrents() {
        let cfg = Config::default();
        let client = LibtorrentClient::new(&cfg).unwrap();
        assert!(client.dry_run);
        assert!(client.list().is_empty());
        assert!(client.poll_events().is_empty());
        client.promote_metadata();
        client.adjust_queue(&cfg);
        assert!(!client
            .pause("0123456789012345678901234567890123456789")
            .unwrap());
        assert!(!client
            .restart("0123456789012345678901234567890123456789")
            .unwrap());
    }

    #[test]
    fn reads_imported_bandwidth_limits_without_changing_their_byte_units() {
        let path = std::env::temp_dir().join(format!(
            "rextto-limits-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let mut cfg = Config::default();
        cfg.data_dir = path.clone();
        let conn = rusqlite::Connection::open(path.join("rextto_config.db")).unwrap();
        conn.execute_batch("CREATE TABLE torrent_limits (info_hash TEXT PRIMARY KEY, dl_bytes INTEGER, ul_bytes INTEGER);").unwrap();
        conn.execute(
            "INSERT INTO torrent_limits VALUES (?1,?2,?3)",
            rusqlite::params!["abc123", 1_572_864_i64, -1_i64],
        )
        .unwrap();
        drop(conn);
        let client = LibtorrentClient::new(&cfg).unwrap();
        assert_eq!(
            client.stored_limits("abc123").unwrap(),
            Some((1_572_864, -1))
        );
        drop(client);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn per_torrent_seed_limits_are_persisted_with_lowercase_hashes() {
        let path = std::env::temp_dir().join(format!(
            "rextto-seed-limits-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let mut cfg = Config::default();
        cfg.data_dir = path.join("data");
        cfg.state_dir = path.join("state");
        let client = LibtorrentClient::new(&cfg).unwrap();
        client.save_seed_limit("ABCDEF", 2.5, 7).unwrap();
        assert_eq!(
            client
                .seed_limits()
                .unwrap()
                .get("abcdef")
                .map(|limit| (limit.ratio, limit.days)),
            Some((2.5, 7))
        );
        let saved = std::fs::read_to_string(client.seed_limits_path()).unwrap();
        assert!(saved.contains("abcdef"));
        assert!(!saved.contains("ABCDEF"));
        drop(client);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn killswitch_binds_listen_to_outgoing_interface() {
        let mut lt = LibtorrentSettings::default();
        lt.port_min = 6881;
        assert_eq!(effective_listen_interfaces(&lt), "");
        // Interfaccia nuda: viene completata con la porta minima.
        lt.listen_interfaces = "wg0".into();
        assert_eq!(effective_listen_interfaces(&lt), "wg0:6881");
        // Interfaccia di uscita senza listen esplicito: ascolto legato alla VPN.
        lt.listen_interfaces = String::new();
        lt.outgoing_interface = "tun0".into();
        assert_eq!(effective_listen_interfaces(&lt), "tun0:6881");
        // Listen esplicito con porta: resta invariato.
        lt.listen_interfaces = "0.0.0.0:6881-6891".into();
        assert_eq!(effective_listen_interfaces(&lt), "0.0.0.0:6881-6891");
    }
}
