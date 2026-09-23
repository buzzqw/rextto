use anyhow::Result;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

pub fn create_snapshot(data_dir: &Path, backup_root: &Path, retain: usize) -> Result<PathBuf> {
    fs::create_dir_all(backup_root)?;
    // Nome leggibile e ordinabile cronologicamente (locale del server).
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let destination = backup_root.join(format!("rextto-backup-{stamp}.zip"));
    for name in [
        "rextto_series.db",
        "rextto_archive.db",
        "rextto_config.db",
        "rextto_comics.db",
    ] {
        let source = data_dir.join(name);
        if let Ok(connection) = rusqlite::Connection::open(source) {
            let _ = connection.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        }
    }
    let file = fs::File::create(&destination)?;
    let mut archive = ZipWriter::new(file);
    add_directory(&mut archive, data_dir, data_dir, backup_root)?;
    archive.finish()?.sync_all()?;
    let mut snapshots = fs::read_dir(backup_root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "zip")
        })
        .collect::<Vec<_>>();
    // Ordina per data di modifica: con nomi legacy e nuovi mescolati il solo
    // nome non riflette l'ordine cronologico reale.
    snapshots.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH)
    });
    let keep_from = snapshots.len().saturating_sub(retain.max(1));
    for entry in snapshots.into_iter().take(keep_from) {
        fs::remove_file(entry.path())?;
    }
    Ok(destination)
}

/// Copies a snapshot to an external directory (e.g. a cloud-synced folder or a
/// mounted remote), creating the destination if needed. Returns the copy path.
pub fn copy_snapshot(archive: &Path, target_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(target_dir)?;
    let name = archive
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid backup filename"))?;
    let destination = target_dir.join(name);
    fs::copy(archive, &destination)?;
    Ok(destination)
}

/// `FtpStream::connect` uses `ToSocketAddrs`, so a bare host/IP without a port
/// fails with "invalid socket address". Append the default FTP port when the
/// user did not specify one (an `[ipv6]:port` literal already has one).
fn ftp_endpoint(host: &str) -> String {
    let host = host.trim();
    let has_port = if let Some(rest) = host.strip_prefix('[') {
        rest.contains("]:")
    } else {
        host.contains(':')
    };
    if has_port || host.is_empty() {
        host.to_string()
    } else {
        format!("{host}:21")
    }
}

/// Outcome of a full FTP test (connect, login, CWD, upload and delete a probe
/// file). Every step is logged and reported so the UI can show what worked.
#[derive(Debug, Default, serde::Serialize)]
pub struct FtpTestReport {
    pub host: String,
    pub user: String,
    pub path: String,
    pub test_file: String,
    pub connected: bool,
    pub logged_in: bool,
    pub entered_path: bool,
    pub uploaded: bool,
    pub deleted: bool,
    pub error: Option<String>,
}

impl FtpTestReport {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Connects, logs in, enters the remote directory, uploads a small probe file
/// and deletes it again — so a successful test proves the backup copy can work.
pub fn test_ftp(host: &str, user: &str, password: &str, remote_path: &str) -> FtpTestReport {
    let mut report = FtpTestReport {
        host: host.trim().to_string(),
        user: user.to_string(),
        path: remote_path.trim().to_string(),
        ..Default::default()
    };
    tracing::info!(host = %report.host, user = %report.user, endpoint = %ftp_endpoint(host), "FTP test: connecting");
    let mut ftp = match suppaftp::FtpStream::connect(ftp_endpoint(host)) {
        Ok(ftp) => {
            report.connected = true;
            ftp
        }
        Err(error) => {
            tracing::warn!(host = %report.host, %error, "FTP test: connection failed");
            report.error = Some(format!("connessione fallita: {error}"));
            return report;
        }
    };
    if let Err(error) = ftp.login(user, password) {
        tracing::warn!(host = %report.host, user = %report.user, %error, "FTP test: login failed");
        report.error = Some(format!("login fallito: {error}"));
        let _ = ftp.quit();
        return report;
    }
    report.logged_in = true;
    tracing::info!(host = %report.host, user = %report.user, "FTP test: login ok");
    if !report.path.is_empty() {
        if let Err(error) = ftp.cwd(&report.path) {
            tracing::warn!(host = %report.host, path = %report.path, %error, "FTP test: remote path not accessible");
            report.error = Some(format!("cartella '{}': {error}", report.path));
            let _ = ftp.quit();
            return report;
        }
        report.entered_path = true;
        tracing::info!(host = %report.host, path = %report.path, "FTP test: remote path ok");
    }
    let test_file = format!("rextto-test-{}.txt", uuid::Uuid::new_v4());
    report.test_file = test_file.clone();
    let payload = b"Rextto FTP test file\n";
    let mut reader = std::io::Cursor::new(payload.as_slice());
    match ftp.put_file(&test_file, &mut reader) {
        Ok(bytes) => {
            report.uploaded = true;
            tracing::info!(host = %report.host, path = %report.path, remote = %test_file, bytes, "FTP test: probe file uploaded");
        }
        Err(error) => {
            tracing::warn!(host = %report.host, path = %report.path, remote = %test_file, %error, "FTP test: probe upload failed");
            report.error = Some(format!("upload del file di prova fallito: {error}"));
            let _ = ftp.quit();
            return report;
        }
    }
    match ftp.rm(&test_file) {
        Ok(()) => {
            report.deleted = true;
            tracing::info!(host = %report.host, remote = %test_file, "FTP test: probe file removed");
        }
        Err(error) => {
            // The upload already proved write access; deletion is best-effort.
            tracing::warn!(host = %report.host, remote = %test_file, %error, "FTP test: could not remove probe file");
            report.error = Some(format!("file di prova caricato ma non rimosso: {error}"));
        }
    }
    let _ = ftp.quit();
    tracing::info!(host = %report.host, path = %report.path, ok = report.ok(), "FTP test completed");
    report
}

pub fn upload_ftp(
    archive: &Path,
    host: &str,
    user: &str,
    password: &str,
    remote_path: &str,
) -> Result<()> {
    use std::io::BufReader;
    let mut ftp = suppaftp::FtpStream::connect(ftp_endpoint(host))
        .map_err(|error| anyhow::anyhow!("connessione FTP fallita: {error}"))?;
    ftp.login(user, password)?;
    if !remote_path.trim().is_empty() {
        ftp.cwd(remote_path)?;
    }
    let name = archive
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid backup filename"))?;
    let mut reader = BufReader::new(fs::File::open(archive)?);
    ftp.put_file(name, &mut reader)?;
    ftp.quit()?;
    Ok(())
}

fn add_directory(
    writer: &mut ZipWriter<fs::File>,
    root: &Path,
    current: &Path,
    backup_root: &Path,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            continue;
        }
        let path = entry.path();
        if path.starts_with(backup_root) {
            continue;
        }
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value == "rextto_torrents_state")
        {
            continue;
        }
        let name = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            add_directory(writer, root, &path, backup_root)?;
            continue;
        }
        if name.ends_with("-wal")
            || name.ends_with("-shm")
            || name.ends_with(".log")
            || name.contains(".log.")
        {
            continue;
        }
        let mut source = fs::File::open(&path)?;
        writer.start_file(
            name,
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )?;
        let mut buffer = Vec::new();
        source.read_to_end(&mut buffer)?;
        writer.write_all(&buffer)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ftp_endpoint_defaults_to_port_21() {
        assert_eq!(ftp_endpoint("10.0.0.5"), "10.0.0.5:21");
        assert_eq!(ftp_endpoint("ftp.example.com"), "ftp.example.com:21");
        assert_eq!(ftp_endpoint("ftp.example.com:2121"), "ftp.example.com:2121");
        assert_eq!(ftp_endpoint(" 10.0.0.5 "), "10.0.0.5:21");
        assert_eq!(ftp_endpoint("[::1]:21"), "[::1]:21");
        assert_eq!(ftp_endpoint(""), "");
    }

    #[test]
    fn creates_compressed_snapshot_without_transient_files() {
        let root = std::env::temp_dir().join(format!("rextto-backup-{}", std::process::id()));
        let backup = root.join("backups");
        fs::create_dir_all(&backup).unwrap();
        fs::write(root.join("settings.json"), b"{}\n").unwrap();
        fs::write(root.join("settings.db-wal"), b"transient").unwrap();
        fs::write(root.join("rextto.log"), b"log").unwrap();
        let snapshot = create_snapshot(&root, &backup, 1).unwrap();
        let file = fs::File::open(snapshot).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.by_name("settings.json").is_ok());
        assert!(archive.by_name("settings.db-wal").is_err());
        assert!(archive.by_name("rextto.log").is_err());
        let _ = fs::remove_dir_all(root);
    }
}
