//! Command-line parsing for the `rexttod` daemon.
//!
//! The daemon has no third-party argument parser: it is a self-contained
//! executable that must build anywhere with a Rust toolchain. This module keeps
//! the (small) grammar in one place so it can be unit-tested without spawning
//! the process.

/// Options accepted by `rexttod --update`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UpdateOptions {
    /// GitHub repository (`owner/name`). Defaults to [`crate::update::DEFAULT_REPO`].
    pub repo: Option<String>,
    /// Explicit release tag, for example `v0.2.0`.
    pub release: Option<String>,
    /// Release channel (`continuous` or `stable`). Ignored when `release` is set.
    pub channel: Option<String>,
    /// Installation directory. Defaults to the directory of the running binary.
    pub install_dir: Option<String>,
    /// Local archive to install instead of downloading one (offline/testing).
    pub archive: Option<String>,
    /// Restart the service after a successful update (default: true).
    pub restart: bool,
    /// Reinstall even when the installed marker already matches the release.
    pub force: bool,
}

/// Parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Run the daemon (default command).
    Serve {
        dry_run: bool,
        config: Option<String>,
    },
    /// Import data from a legacy `Extto` installation.
    Import { source: String, data_dir: String },
    /// Print the installed version and exit.
    Version,
    /// Print usage and exit.
    Help,
    /// Download and install the latest components, then exit.
    Update(UpdateOptions),
}

impl Default for Command {
    fn default() -> Self {
        Command::Serve {
            dry_run: false,
            config: None,
        }
    }
}

/// Parse the process arguments, excluding `argv[0]`.
pub fn parse<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();

    // `import` is a sub-command and owns the rest of the line.
    if args.first().map(String::as_str) == Some("import") {
        return parse_import(&args[1..]);
    }

    if has_flag(&args, &["--help", "-h"]) {
        return Command::Help;
    }
    if has_flag(&args, &["--version", "-V"]) {
        return Command::Version;
    }
    if has_flag(&args, &["--update"]) {
        return Command::Update(parse_update(&args));
    }

    let mut dry_run = false;
    let mut config = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => dry_run = true,
            "--config" => {
                config = args.get(index + 1).cloned();
                index += 1;
            }
            value if value.starts_with("--config=") => {
                config = Some(value["--config=".len()..].to_string());
            }
            _ => {}
        }
        index += 1;
    }
    Command::Serve { dry_run, config }
}

fn parse_import(args: &[String]) -> Command {
    let mut source =
        std::env::var("REXTTO_IMPORT_SOURCE").unwrap_or_else(|_| "/path/to/legacy".to_string());
    let mut data_dir = "data".to_string();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--from-copy" => {
                if let Some(value) = args.get(index + 1) {
                    source = value.clone();
                    index += 1;
                }
            }
            "--data-dir" => {
                if let Some(value) = args.get(index + 1) {
                    data_dir = value.clone();
                    index += 1;
                }
            }
            value if value.starts_with("--from-copy=") => {
                source = value["--from-copy=".len()..].to_string();
            }
            value if value.starts_with("--data-dir=") => {
                data_dir = value["--data-dir=".len()..].to_string();
            }
            _ => {}
        }
        index += 1;
    }
    Command::Import { source, data_dir }
}

fn parse_update(args: &[String]) -> UpdateOptions {
    let mut options = UpdateOptions {
        restart: true,
        ..UpdateOptions::default()
    };
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        let (key, inline) = match arg.split_once('=') {
            Some((key, value)) => (key, Some(value.to_string())),
            None => (arg, None),
        };
        let take_value = |index: &mut usize| -> Option<String> {
            if let Some(value) = inline.clone() {
                return Some(value);
            }
            let value = args.get(*index + 1).cloned();
            if value.is_some() {
                *index += 1;
            }
            value
        };
        match key {
            "--repo" => options.repo = take_value(&mut index),
            "--release" => options.release = take_value(&mut index),
            "--channel" => options.channel = take_value(&mut index),
            "--install-dir" => options.install_dir = take_value(&mut index),
            "--archive" => options.archive = take_value(&mut index),
            "--no-restart" => options.restart = false,
            "--force" => options.force = true,
            _ => {}
        }
        index += 1;
    }
    options
}

fn has_flag(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|arg| flags.contains(&arg.as_str()))
}

/// Usage text shown by `rexttod --help`.
pub fn usage() -> String {
    format!(
        "{name} {version} — {about}\n\
         \n\
         USAGE:\n\
         \x20   {name} [OPTIONS]\n\
         \x20   {name} import --from-copy <dir> [--data-dir <dir>]\n\
         \x20   {name} --update [OPTIONS]\n\
         \n\
         OPTIONS:\n\
         \x20   -h, --help            Show this help and exit\n\
         \x20   -V, --version         Show the installed version and exit\n\
         \x20   --config <file>       Configuration file (default: rextto.json)\n\
         \x20   --dry-run             Never start real downloads\n\
         \n\
         UPDATE OPTIONS:\n\
         \x20   --repo <owner/name>   GitHub repository (default: {repo})\n\
         \x20   --release <tag>       Install a specific release tag\n\
         \x20   --channel <name>      continuous (default) or stable\n\
         \x20   --install-dir <dir>   Installation directory (default: binary directory)\n\
         \x20   --archive <file>      Install from a local archive instead of downloading\n\
         \x20   --force               Reinstall even if the version is unchanged\n\
         \x20   --no-restart          Do not restart the service after updating\n",
        name = crate::constants::APP_NAME,
        version = crate::constants::VERSION,
        about = env!("CARGO_PKG_DESCRIPTION"),
        repo = crate::update::DEFAULT_REPO,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(line: &str) -> Command {
        parse(line.split_whitespace().map(str::to_string))
    }

    #[test]
    fn defaults_to_serve() {
        assert_eq!(parse_str(""), Command::default());
        assert_eq!(
            parse_str("--dry-run --config /tmp/rextto.json"),
            Command::Serve {
                dry_run: true,
                config: Some("/tmp/rextto.json".into()),
            }
        );
    }

    #[test]
    fn accepts_inline_config() {
        assert_eq!(
            parse_str("--config=/etc/rextto.json"),
            Command::Serve {
                dry_run: false,
                config: Some("/etc/rextto.json".into()),
            }
        );
    }

    #[test]
    fn recognises_version_and_help() {
        assert_eq!(parse_str("--version"), Command::Version);
        assert_eq!(parse_str("-V"), Command::Version);
        assert_eq!(parse_str("--help"), Command::Help);
        assert_eq!(parse_str("-h"), Command::Help);
    }

    #[test]
    fn parses_update_options() {
        assert_eq!(
            parse_str("--update --channel stable --install-dir /opt/rextto --no-restart"),
            Command::Update(UpdateOptions {
                channel: Some("stable".into()),
                install_dir: Some("/opt/rextto".into()),
                restart: false,
                ..UpdateOptions::default()
            })
        );
        assert_eq!(
            parse_str("--update --release=v1.2.3 --repo=me/rextto --force"),
            Command::Update(UpdateOptions {
                release: Some("v1.2.3".into()),
                repo: Some("me/rextto".into()),
                restart: true,
                force: true,
                ..UpdateOptions::default()
            })
        );
    }

    #[test]
    fn parses_import_subcommand() {
        assert_eq!(
            parse_str("import --from-copy /srv/extto --data-dir /srv/data"),
            Command::Import {
                source: "/srv/extto".into(),
                data_dir: "/srv/data".into(),
            }
        );
    }
}
