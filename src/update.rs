//! Self-update support for Rextto.
//!
//! `rexttod --update` mirrors what `install.sh` does for the payload: it
//! downloads the release asset (`rextto-linux-<arch>.tar.gz`), verifies its
//! checksum when the release publishes one, and installs the daemon, the web UI
//! and any bundled library into the installation directory.
//!
//! The update is deliberately staged: the new payload is extracted and
//! validated in a temporary directory, then swapped into place with atomic
//! renames. A failure before the swap leaves the running installation
//! untouched; a failure during the swap is rolled back from the previous
//! version. Runtime data (`REXTTO_DATA_DIR`) is never touched.

use crate::cli::UpdateOptions;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// Repository used when none is configured.
pub const DEFAULT_REPO: &str = "buzzqw/rextto";
/// systemd unit updated/restarted after an install.
pub const SERVICE_NAME: &str = "rextto.service";
/// Prefix of the per-architecture release asset.
pub const ARCHIVE_STEM: &str = "rextto-linux";

/// Return the installed version, including the release marker written next to
/// the executable by the installer (when present) and the bundled libtorrent.
pub fn version_string() -> String {
    let mut value = format!(
        "{} {} (build {})",
        crate::constants::APP_NAME,
        crate::constants::VERSION,
        crate::constants::BUILD
    );
    if let Some(marker) = installed_marker() {
        value.push_str(&format!(" [{marker}]"));
    }
    let libtorrent = crate::libtorrent::libtorrent_version();
    if !libtorrent.is_empty() {
        value.push_str(&format!(" · libtorrent {libtorrent}"));
    }
    value
}

fn installed_marker() -> Option<String> {
    let executable = std::env::current_exe().ok()?;
    let dir = executable.parent()?;
    for candidate in [dir.join("VERSION"), dir.join("..").join("VERSION")] {
        if let Ok(text) = fs::read_to_string(&candidate) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// Resolve the release name to download.
pub fn release_name(options: &UpdateOptions) -> String {
    if let Some(release) = options.release.as_deref() {
        return release.to_string();
    }
    if let Some(channel) = options.channel.as_deref() {
        return channel.to_string();
    }
    std::env::var("REXTTO_VERSION")
        .or_else(|_| std::env::var("REXTTO_CHANNEL"))
        .unwrap_or_else(|_| "continuous".to_string())
}

/// Build the release asset URL for a repository, release and architecture.
pub fn archive_url(repo: &str, release: &str, arch: &str) -> String {
    let asset = format!("{ARCHIVE_STEM}-{arch}.tar.gz");
    if release == "latest" || release == "stable" {
        format!("https://github.com/{repo}/releases/latest/download/{asset}")
    } else {
        format!("https://github.com/{repo}/releases/download/{release}/{asset}")
    }
}

fn target_arch() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x86_64"),
        "aarch64" => Ok("aarch64"),
        other => bail!("unsupported CPU architecture for updates: {other}"),
    }
}

/// Run `rexttod --update`.
pub async fn run(options: &UpdateOptions) -> Result<()> {
    let repo = options
        .repo
        .clone()
        .or_else(|| std::env::var("REXTTO_REPO").ok())
        .unwrap_or_else(|| DEFAULT_REPO.to_string());
    let release = release_name(options);
    let install_dir = resolve_install_dir(options)?;

    println!("Rextto updater");
    println!("  release:    {release}");
    println!("  repository: {repo}");
    println!("  install:    {}", install_dir.display());

    if let Some(marker) = installed_marker() {
        if options.release.is_some() && marker == release && !options.force {
            println!("Already at release {release}; nothing to do (use --force to reinstall).");
            return Ok(());
        }
    }

    let work = WorkDir::new()?;
    let archive = match options.archive.as_deref() {
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_file() {
                bail!("local archive not found: {}", path.display());
            }
            println!("  source:     local archive {}", path.display());
            let checksum = PathBuf::from(format!("{}.sha256", path.display()));
            if checksum.is_file() {
                verify_local_checksum(&path, &checksum)?;
            } else {
                println!("  checksum:   no .sha256 next to the local archive");
            }
            path
        }
        None => {
            let arch = target_arch()?;
            let url = archive_url(&repo, &release, arch);
            let destination = work
                .path()
                .join(url.rsplit('/').next().unwrap_or("rextto-linux.tar.gz"));
            println!("  downloading {url}");
            download(&url, &destination).await?;
            verify_remote_checksum(&destination, &format!("{url}.sha256")).await?;
            destination
        }
    };

    let extract_dir = work.path().join("payload");
    fs::create_dir_all(&extract_dir)?;
    extract(&archive, &extract_dir)?;

    let root = find_release_root(&extract_dir)
        .context("release archive does not contain a rexttod executable")?;
    validate_release(&root)?;

    install(&root, &install_dir)?;
    fs::write(install_dir.join("VERSION"), format!("{release}\n"))?;
    println!("Rextto updated to {release} in {}", install_dir.display());

    restart_service(options)?;
    Ok(())
}

fn resolve_install_dir(options: &UpdateOptions) -> Result<PathBuf> {
    if let Some(dir) = options.install_dir.as_deref() {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("REXTTO_INSTALL_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let executable = std::env::current_exe().context("cannot locate the running executable")?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .context("cannot determine the installation directory from the executable path")
}

async fn download(url: &str, destination: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("rexttod/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(1800))
        .build()?;
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("cannot reach {url}"))?;
    let status = response.status();
    if !status.is_success() {
        bail!("download failed for {url}: HTTP {status}");
    }
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("cannot read the download from {url}"))?;
    fs::write(destination, &bytes)
        .with_context(|| format!("cannot write {}", destination.display()))?;
    Ok(())
}

/// Verify the archive against a `.sha256` asset when the release publishes one.
/// A missing checksum is tolerated because not every build publishes it.
async fn verify_remote_checksum(archive: &Path, checksum_url: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("rexttod/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let response = client.get(checksum_url).send().await;
    let Ok(response) = response else {
        println!("  checksum:   not published by this release (continuing)");
        return Ok(());
    };
    if !response.status().is_success() {
        println!("  checksum:   not published by this release (continuing)");
        return Ok(());
    }
    let body = response.text().await?;
    let expected = body
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if expected.len() != 64 {
        println!("  checksum:   not published by this release (continuing)");
        return Ok(());
    }
    compare_checksum(archive, &expected)
}

fn verify_local_checksum(archive: &Path, checksum: &Path) -> Result<()> {
    let body = fs::read_to_string(checksum)?;
    let expected = body.split_whitespace().next().unwrap_or_default();
    if expected.len() != 64 {
        bail!("malformed checksum file: {}", checksum.display());
    }
    compare_checksum(archive, expected)
}

fn compare_checksum(archive: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(archive)?;
    if actual != expected.to_ascii_lowercase() {
        bail!(
            "checksum mismatch for {}: expected {expected}, got {actual}",
            archive.display()
        );
    }
    println!("  checksum:   verified");
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn extract(archive: &Path, destination: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .status()
        .context("cannot run tar (is it installed?)")?;
    if !status.success() {
        bail!("tar failed to extract {}", archive.display());
    }
    Ok(())
}

/// Find the directory that contains the `rexttod` executable.
fn find_release_root(directory: &Path) -> Option<PathBuf> {
    let direct = directory.join(crate::constants::APP_NAME);
    if direct.is_file() {
        return Some(directory.to_path_buf());
    }
    for entry in fs::read_dir(directory).ok()?.flatten() {
        if entry.file_type().ok()?.is_dir() {
            if let Some(found) = find_release_root(&entry.path()) {
                return Some(found);
            }
        }
    }
    None
}

fn validate_release(root: &Path) -> Result<()> {
    let binary = root.join(crate::constants::APP_NAME);
    if !binary.is_file() {
        bail!("release is missing the {} executable", binary.display());
    }
    let ui = root.join("ui").join("pkg").join("ui.js");
    if !ui.is_file() {
        bail!("release is missing the web UI ({})", ui.display());
    }
    Ok(())
}

/// Install the staged payload. Everything is copied next to the final
/// destination first; the live installation is only touched once every piece is
/// staged, and any failure during the swap restores the previous binary and
/// directories.
fn install(root: &Path, install_dir: &Path) -> Result<()> {
    fs::create_dir_all(install_dir)
        .with_context(|| format!("cannot create {}", install_dir.display()))?;
    let suffix = format!(".{}.new", std::process::id());

    // Stage the executable and the directories before touching what is live.
    let binary = install_dir.join(crate::constants::APP_NAME);
    let new_binary = install_dir.join(format!("{}{}", crate::constants::APP_NAME, suffix));
    fs::copy(root.join(crate::constants::APP_NAME), &new_binary)
        .context("cannot stage the new executable")?;
    set_executable(&new_binary)?;

    let mut staged_dirs: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    for directory in ["ui", "lib"] {
        let source = root.join(directory);
        if !source.is_dir() {
            continue;
        }
        let staged = install_dir.join(format!(".{directory}{suffix}"));
        remove_path(&staged);
        copy_dir(&source, &staged)
            .with_context(|| format!("cannot stage the {directory} directory"))?;
        let previous = install_dir.join(format!(".{directory}.old{}", std::process::id()));
        remove_path(&previous);
        staged_dirs.push((directory.to_string(), staged, previous));
    }

    // Snapshot the current executable so the whole swap can be undone.
    let backup = install_dir.join(format!("{}.bak", crate::constants::APP_NAME));
    let had_binary = binary.is_file();
    if had_binary {
        fs::copy(&binary, &backup).context("cannot back up the current executable")?;
    }
    let rollback = |committed: &[(PathBuf, PathBuf)]| {
        for (destination, previous) in committed.iter().rev() {
            if previous.exists() {
                remove_path(destination);
                let _ = fs::rename(previous, destination);
            }
        }
        if had_binary {
            let _ = fs::copy(&backup, &binary);
        }
        remove_path(&backup);
    };

    if let Err(error) = fs::rename(&new_binary, &binary) {
        remove_path(&new_binary);
        rollback(&[]);
        return Err(error).context("cannot replace the executable");
    }

    let mut committed: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (directory, staged, previous) in staged_dirs {
        let destination = install_dir.join(&directory);
        if destination.exists() {
            if let Err(error) = fs::rename(&destination, &previous) {
                remove_path(&staged);
                rollback(&committed);
                return Err(error)
                    .with_context(|| format!("cannot move the current {directory} aside"));
            }
        }
        if let Err(error) = fs::rename(&staged, &destination) {
            if previous.exists() {
                let _ = fs::rename(&previous, &destination);
            }
            remove_path(&staged);
            rollback(&committed);
            return Err(error)
                .with_context(|| format!("cannot install the {directory} directory"));
        }
        committed.push((destination, previous));
    }

    // Success: the previous versions are no longer needed.
    for (_, previous) in committed {
        remove_path(&previous);
    }
    remove_path(&backup);

    // Keep the launcher and the release notes alongside the payload when shipped.
    for file in ["run.sh", "README.md"] {
        let source = root.join(file);
        if source.is_file() {
            let destination = install_dir.join(file);
            fs::copy(&source, &destination).with_context(|| format!("cannot install {file}"))?;
            if file == "run.sh" {
                set_executable(&destination)?;
            }
        }
    }
    Ok(())
}

fn restart_service(options: &UpdateOptions) -> Result<()> {
    if !options.restart {
        println!("Service not restarted (--no-restart).");
        return Ok(());
    }
    if unsafe { libc::geteuid() } != 0 {
        println!("Run `sudo systemctl restart {SERVICE_NAME}` to start the new version.");
        return Ok(());
    }
    match Command::new("systemctl")
        .args(["restart", SERVICE_NAME])
        .status()
    {
        Ok(status) if status.success() => println!("Service {SERVICE_NAME} restarted."),
        _ => eprintln!(
            "Could not restart {SERVICE_NAME}; run `systemctl restart {SERVICE_NAME}` manually."
        ),
    }
    Ok(())
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let destination = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &destination)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(entry.path())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, destination)?;
        } else {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

fn remove_path(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else if path.exists() {
        let _ = fs::remove_file(path);
    }
}

/// Temporary working directory removed on drop.
struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    fn new() -> Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or_default();
        let path =
            std::env::temp_dir().join(format!("rextto-update-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        remove_path(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_uses_the_tagged_download_path() {
        assert_eq!(
            archive_url("me/rextto", "continuous", "x86_64"),
            "https://github.com/me/rextto/releases/download/continuous/rextto-linux-x86_64.tar.gz"
        );
    }

    #[test]
    fn stable_uses_the_latest_download_path() {
        assert_eq!(
            archive_url("me/rextto", "stable", "aarch64"),
            "https://github.com/me/rextto/releases/latest/download/rextto-linux-aarch64.tar.gz"
        );
    }

    #[test]
    fn release_name_prefers_explicit_values() {
        let mut options = UpdateOptions::default();
        assert_eq!(release_name(&options), "continuous");
        options.channel = Some("stable".into());
        assert_eq!(release_name(&options), "stable");
        options.release = Some("v1.2.3".into());
        assert_eq!(release_name(&options), "v1.2.3");
    }

    #[test]
    fn find_release_root_handles_nested_archives() {
        let base = std::env::temp_dir().join(format!("rextto-test-{}", std::process::id()));
        let nested = base.join("rextto-linux-x86_64");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("rexttod"), b"binary").unwrap();
        assert_eq!(find_release_root(&base), Some(nested.clone()));
        let _ = fs::remove_dir_all(&base);
    }
}
