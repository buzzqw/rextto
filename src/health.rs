use serde::Serialize;
use std::{
    path::Path,
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub unix_time: u64,
    pub process_id: u32,
    pub resident_bytes: u64,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub data_dir_writable: bool,
    pub disk_total_bytes: u64,
    pub disk_free_bytes: u64,
    pub uptime_seconds: u64,
    pub load_average: Option<f64>,
    pub cpu_percent: Option<f64>,
    pub process_cpu_percent: Option<f64>,
    pub trash_file_count: u64,
    pub trash_bytes: u64,
    pub disks: Vec<DiskInfo>,
    pub paths: Vec<PathCheck>,
    pub ramdisk: Option<RamDiskInfo>,
    pub last_errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ProcessMetrics {
    pub resident_bytes: u64,
    pub process_cpu_percent: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct DiskInfo {
    pub mount: String,
    pub filesystem: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct PathCheck {
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub writable: bool,
}

#[derive(Debug, Serialize)]
pub struct RamDiskInfo {
    pub path: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

struct CpuSample {
    total: u64,
    idle: u64,
    at: Instant,
    percent: Option<f64>,
}

static CPU_SAMPLE: Mutex<Option<CpuSample>> = Mutex::new(None);

struct ProcessCpuSample {
    ticks: u64,
    at: Instant,
    percent: Option<f64>,
}

static PROCESS_CPU_SAMPLE: Mutex<Option<ProcessCpuSample>> = Mutex::new(None);

/// Legge `(total, idle)` da `/proc/stat`.
fn cpu_snapshot() -> Option<(u64, u64)> {
    let contents = std::fs::read_to_string("/proc/stat").ok()?;
    let line = contents.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = parts.filter_map(|value| value.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    let total: u64 = values.iter().sum();
    let idle: u64 = values[3] + values.get(4).copied().unwrap_or(0);
    Some((total, idle))
}

/// Percentuale di utilizzo CPU del sistema, calcolata sul delta di `/proc/stat`
/// tra due letture consecutive. Le chiamate ravvicinate riusano l'ultimo valore;
/// la primissima chiamata esegue una breve misurazione per non restituire `None`.
fn system_cpu_percent() -> Option<f64> {
    let (total, idle) = cpu_snapshot()?;
    let now = Instant::now();
    let mut guard = CPU_SAMPLE.lock().ok()?;
    if let Some(previous) = guard.as_ref() {
        let elapsed = now.duration_since(previous.at).as_secs_f64();
        let delta_total = total.saturating_sub(previous.total) as f64;
        let delta_idle = idle.saturating_sub(previous.idle) as f64;
        let result = if elapsed >= 0.2 && delta_total > 0.0 {
            Some(((delta_total - delta_idle) / delta_total * 100.0).clamp(0.0, 100.0))
        } else {
            previous.percent
        };
        *guard = Some(CpuSample {
            total,
            idle,
            at: now,
            percent: result,
        });
        return result;
    }
    drop(guard);
    // Primo campione: misura su una finestra breve per avere subito un valore.
    std::thread::sleep(std::time::Duration::from_millis(150));
    let (total_later, idle_later) = cpu_snapshot()?;
    let delta_total = total_later.saturating_sub(total) as f64;
    let delta_idle = idle_later.saturating_sub(idle) as f64;
    let result = if delta_total > 0.0 {
        Some(((delta_total - delta_idle) / delta_total * 100.0).clamp(0.0, 100.0))
    } else {
        None
    };
    let mut guard = CPU_SAMPLE.lock().ok()?;
    *guard = Some(CpuSample {
        total: total_later,
        idle: idle_later,
        at: Instant::now(),
        percent: result,
    });
    result
}

/// Percentuale CPU del processo Rextto, calcolata sui suoi tick utime+stime.
/// Il valore è riferito a un core (può quindi superare 100% con più thread).
fn process_cpu_percent() -> Option<f64> {
    let contents = std::fs::read_to_string("/proc/self/stat").ok()?;
    let command_end = contents.rfind(')')?;
    let fields: Vec<&str> = contents.get(command_end + 2..)?.split_whitespace().collect();
    // After pid and comm, fields[0] is state (field 3); utime/stime are
    // fields 14/15 in procfs, hence indexes 11/12 here.
    let ticks = fields.get(11)?.parse::<u64>().ok()?.saturating_add(fields.get(12)?.parse::<u64>().ok()?);
    let now = Instant::now();
    let clock_ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let mut guard = PROCESS_CPU_SAMPLE.lock().ok()?;
    let Some(previous) = guard.as_ref() else {
        *guard = Some(ProcessCpuSample {
            ticks,
            at: now,
            percent: None,
        });
        return None;
    };
    let elapsed = now.duration_since(previous.at).as_secs_f64();
    let percent = if elapsed > 0.0 {
        Some((ticks.saturating_sub(previous.ticks) as f64 / clock_ticks / elapsed * 100.0).max(0.0))
    } else {
        previous.percent
    };
    *guard = Some(ProcessCpuSample {
        ticks,
        at: now,
        percent,
    });
    percent
}

/// Contesto dei percorsi controllati dalla salute (permessi, spazio, ram disk).
pub struct HealthPaths<'a> {
    pub data_dir: &'a Path,
    pub trash_path: &'a Path,
    pub download_path: &'a Path,
    pub archive_root: Option<&'a Path>,
    pub ramdisk_path: Option<&'a Path>,
}

pub fn check(data_dir: &Path) -> Health {
    let trash = data_dir.join("trash");
    check_with_paths(&HealthPaths {
        data_dir,
        trash_path: &trash,
        download_path: data_dir,
        archive_root: None,
        ramdisk_path: None,
    })
}

/// Lightweight process-only metrics for the always-visible UI status bar.
/// Unlike `check_with_paths`, this does not inspect disks, paths or trash.
pub fn process_metrics() -> ProcessMetrics {
    let resident_bytes = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|value| value.split_whitespace().nth(1)?.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_mul(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as u64);
    ProcessMetrics {
        resident_bytes,
        process_cpu_percent: process_cpu_percent(),
    }
}

pub fn check_with_paths(paths: &HealthPaths) -> Health {
    let data_dir = paths.data_dir;
    let trash_path = paths.trash_path;
    let download_path = paths.download_path;
    let writable =
        data_dir.is_dir() && std::fs::metadata(data_dir).is_ok() && write_probe(data_dir);
    let (disk_total_bytes, disk_free_bytes) = disk_space(download_path);
    let resident_bytes = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|value| value.split_whitespace().nth(1)?.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_mul(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as u64);
    let uptime_seconds = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|value| value.split_whitespace().next()?.parse::<f64>().ok())
        .map(|value| value as u64)
        .unwrap_or(0);
    let load_average = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|value| value.split_whitespace().next()?.parse::<f64>().ok());
    let (trash_file_count, trash_bytes) = tree_stats(trash_path);
    let (memory_total_bytes, memory_available_bytes) = memory_info();
    Health {
        status: if writable { "ok" } else { "degraded" },
        unix_time: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        process_id: std::process::id(),
        resident_bytes,
        memory_total_bytes,
        memory_available_bytes,
        data_dir_writable: writable,
        disk_total_bytes,
        disk_free_bytes,
        uptime_seconds,
        load_average,
        cpu_percent: system_cpu_percent(),
        process_cpu_percent: process_cpu_percent(),
        trash_file_count,
        trash_bytes,
        disks: disks(),
        paths: path_checks(paths),
        ramdisk: paths.ramdisk_path.and_then(|path| {
            if path.is_dir() {
                let (total_bytes, free_bytes) = disk_space(path);
                Some(RamDiskInfo {
                    path: path.display().to_string(),
                    total_bytes,
                    free_bytes,
                })
            } else {
                None
            }
        }),
        last_errors: recent_errors(&data_dir.join("rextto.log"), 10),
    }
}

/// Controlla esistenza e scrivibilità dei percorsi operativi. La scrittura di
/// prova (`probe`) è limitata ai percorsi locali, per non disturbare i NAS.
fn path_checks(paths: &HealthPaths) -> Vec<PathCheck> {
    let mut checks = Vec::new();
    let mut add = |label: &str, path: &Path, probe: bool| {
        let exists = path.is_dir();
        let writable = exists && (!probe || write_probe(path));
        checks.push(PathCheck {
            label: label.to_string(),
            path: path.display().to_string(),
            exists,
            writable,
        });
    };
    add("Data", paths.data_dir, true);
    add("Download", paths.download_path, true);
    add("Trash", paths.trash_path, true);
    if let Some(root) = paths.archive_root {
        if !root.as_os_str().is_empty() {
            add("Archivio", root, false);
        }
    }
    if let Some(ramdisk) = paths.ramdisk_path {
        if !ramdisk.as_os_str().is_empty() {
            add("RAM disk", ramdisk, false);
        }
    }
    checks
}

/// Ultime righe di errore del log (`rextto.log`), più recenti in fondo.
fn recent_errors(log_path: &Path, limit: usize) -> Vec<String> {
    let Ok(contents) = std::fs::read_to_string(log_path) else {
        return Vec::new();
    };
    let mut errors: Vec<String> = contents
        .lines()
        .filter(|line| line.contains("ERROR"))
        .map(str::to_string)
        .collect();
    let start = errors.len().saturating_sub(limit);
    errors.split_off(start)
}

/// Elenca i filesystem reali montati (esclude i pseudo-fs), deduplicati per
/// device, con spazio totale/libero. Utile per la dashboard e la pagina Salute.
fn disks() -> Vec<DiskInfo> {
    const IGNORED: &[&str] = &[
        "proc",
        "sysfs",
        "devtmpfs",
        "devpts",
        "tmpfs",
        "ramfs",
        "cgroup",
        "cgroup2",
        "pstore",
        "securityfs",
        "debugfs",
        "tracefs",
        "configfs",
        "fusectl",
        "mqueue",
        "hugetlbfs",
        "binfmt_misc",
        "autofs",
        "squashfs",
        "efivarfs",
        "bpf",
        "overlay",
    ];
    let Ok(contents) = std::fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };
    let mut seen_devices: Vec<String> = Vec::new();
    let mut result = Vec::new();
    for line in contents.lines() {
        let mut parts = line.split_whitespace();
        let Some(device) = parts.next() else { continue };
        let Some(mount) = parts.next() else { continue };
        let Some(filesystem) = parts.next() else {
            continue;
        };
        if IGNORED.contains(&filesystem)
            || !mount.starts_with('/')
            || seen_devices.iter().any(|seen| seen == device)
        {
            continue;
        }
        let mount = mount.replace("\\040", " ");
        let (total_bytes, free_bytes) = disk_space(Path::new(&mount));
        if total_bytes == 0 {
            continue;
        }
        seen_devices.push(device.to_string());
        result.push(DiskInfo {
            mount,
            filesystem: filesystem.to_string(),
            total_bytes,
            free_bytes,
        });
        if result.len() >= 12 {
            break;
        }
    }
    result
}

fn memory_info() -> (u64, u64) {
    let Ok(contents) = std::fs::read_to_string("/proc/meminfo") else {
        return (0, 0);
    };
    let field = |name: &str| {
        contents
            .lines()
            .find(|line| line.starts_with(name))
            .and_then(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
            .map(|value| value.saturating_mul(1024))
            .unwrap_or(0)
    };
    (field("MemTotal:"), field("MemAvailable:"))
}

fn tree_stats(path: &Path) -> (u64, u64) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return (0, 0);
    };
    entries
        .filter_map(|entry| entry.ok())
        .fold((0, 0), |(files, bytes), entry| {
            let path = entry.path();
            if path.is_dir() {
                let (nested_files, nested_bytes) = tree_stats(&path);
                (files + nested_files, bytes.saturating_add(nested_bytes))
            } else if path.is_file() {
                (
                    files + 1,
                    bytes.saturating_add(entry.metadata().map(|value| value.len()).unwrap_or(0)),
                )
            } else {
                (files, bytes)
            }
        })
}

fn write_probe(data_dir: &Path) -> bool {
    let path = data_dir.join(format!(".health-probe-{}", std::process::id()));
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|file| {
            file.sync_all()?;
            std::fs::remove_file(&path)
        });
    result.is_ok()
}

fn disk_space(path: &Path) -> (u64, u64) {
    let path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok();
    let Some(path) = path else {
        return (0, 0);
    };
    let mut stats = unsafe { std::mem::zeroed::<libc::statvfs>() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
        return (0, 0);
    }
    let block_size = stats.f_frsize as u64;
    (
        stats.f_blocks as u64 * block_size,
        stats.f_bavail as u64 * block_size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_errors_keeps_only_the_last_error_lines() {
        let path = std::env::temp_dir().join(format!("rextto-health-log-{}", std::process::id()));
        std::fs::write(&path, "INFO ok\nERROR first\nWARN ignore\nERROR second\n").unwrap();
        let errors = recent_errors(&path, 10);
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("first"));
        assert!(errors[1].contains("second"));
        assert_eq!(recent_errors(&path, 1), vec!["ERROR second".to_string()]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn path_checks_report_missing_paths() {
        let root = std::env::temp_dir().join(format!("rextto-health-paths-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let missing = root.join("nope");
        let checks = path_checks(&HealthPaths {
            data_dir: &root,
            trash_path: &missing,
            download_path: &root,
            archive_root: None,
            ramdisk_path: None,
        });
        let trash = checks.iter().find(|check| check.label == "Trash").unwrap();
        assert!(!trash.exists);
        assert!(!trash.writable);
        let data = checks.iter().find(|check| check.label == "Data").unwrap();
        assert!(data.exists && data.writable);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn health_checks_actual_write_access() {
        let path = std::env::temp_dir().join(format!("rextto-health-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let health = check(&path);
        assert_eq!(health.status, "ok");
        assert!(!path
            .join(format!(".health-probe-{}", std::process::id()))
            .exists());
        let _ = std::fs::remove_dir_all(path);
    }
}
