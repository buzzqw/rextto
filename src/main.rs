use anyhow::Result;
use clapless::Args;
use rextto::{
    archive::Archive,
    comics::ComicsDb,
    config::Config,
    database::Database,
    engine::Engine,
    importer,
    libtorrent::LibtorrentClient,
    models::CycleStats,
    notifier::Notifier,
    tmdb::TmdbClient,
    web::{self, AppState},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod clapless {
    pub enum Command {
        Serve {
            dry_run: bool,
            config: Option<String>,
        },
        Import {
            source: String,
            data_dir: String,
        },
    }
    pub struct Args;
    impl Args {
        pub fn parse() -> Command {
            let mut args = std::env::args().skip(1);
            if args.next().as_deref() == Some("import") {
                let mut source = std::env::var("REXTTO_IMPORT_SOURCE")
                    .unwrap_or_else(|_| "/path/to/legacy".to_string());
                let mut data_dir = "data".to_string();
                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--from-copy" => {
                            if let Some(v) = args.next() {
                                source = v
                            }
                        }
                        "--data-dir" => {
                            if let Some(v) = args.next() {
                                data_dir = v
                            }
                        }
                        _ => {}
                    }
                }
                Command::Import { source, data_dir }
            } else {
                let mut dry_run = false;
                let mut config = None;
                for arg in args {
                    match arg.as_str() {
                        "--dry-run" => dry_run = true,
                        "--config" => {
                            config = std::env::args()
                                .skip(1)
                                .skip_while(|v| v != "--config")
                                .nth(1)
                        }
                        _ => {}
                    }
                }
                Command::Serve { dry_run, config }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = Args::parse();
    if let clapless::Command::Import { source, data_dir } = command {
        let report = importer::import_extto(&PathBuf::from(source), &PathBuf::from(data_dir))?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let clapless::Command::Serve { dry_run, config } = command else {
        unreachable!()
    };
    let config_path = config
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rextto.json"));
    let mut cfg = Config::load(&config_path)?;
    if dry_run {
        cfg.dry_run = true;
    }
    cfg.prepare_dirs()?;
    // One-time cleanup of dirty/imported movie requirement placeholders.
    let _ = rextto::config::cleanup_movie_requirements(&cfg.data_dir);
    rextto::cache::init(&cfg.data_dir);
    let log_file = rextto::logging::RotatingWriter::new(
        cfg.data_dir.clone(),
        "rextto.log",
        5 * 1024 * 1024,
        4,
    );
    let (log_writer, _log_guard) = tracing_appender::non_blocking(log_file);
    let default_level = if cfg.debug_enabled() {
        "rextto=debug"
    } else {
        "rextto=info"
    };
    let initial_filter =
        EnvFilter::new(std::env::var("RUST_LOG").unwrap_or_else(|_| default_level.into()));
    let (reload_layer, log_reload) = tracing_subscriber::reload::Layer::new(initial_filter);
    tracing_subscriber::registry()
        .with(reload_layer)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .event_format(rextto::logging::ReadableFormat)
                .fmt_fields(rextto::logging::ReadableFields)
                .with_writer(log_writer),
        )
        .init();
    let db = Arc::new(Mutex::new(Database::open(
        &cfg.data_dir.join("rextto_series.db"),
    )?));
    let archive = Arc::new(Mutex::new(Archive::open(
        &cfg.data_dir.join("rextto_archive.db"),
    )?));
    let comics = Arc::new(ComicsDb::open(&cfg.data_dir.join("rextto_comics.db"))?);
    let engine = Arc::new(Engine::new());
    let torrents = Arc::new(LibtorrentClient::new(&cfg)?);
    let i18n = Arc::new(rextto::i18n::I18nDb::open(
        &cfg.data_dir.join("rextto_config.db"),
    )?);
    if let Ok(language) = i18n.language() {
        rextto::messages::set_language(&language);
    }
    let state = AppState {
        cfg: cfg.clone(),
        config_path,
        i18n,
        db,
        archive,
        comics,
        engine,
        torrents: torrents.clone(),
        torrent_events: Arc::new(Mutex::new(Vec::new())),
        notifier: Arc::new(Notifier::from_config(&cfg)),
        tmdb: Arc::new(TmdbClient::with_language(
            cfg.tmdb_api_key.clone(),
            cfg.tmdb_language(),
        )),
        last_cycle: Arc::new(Mutex::new(CycleStats::default())),
        cycle_lock: Arc::new(tokio::sync::Mutex::new(())),
        log_reload: Arc::new(Mutex::new(log_reload)),
    };
    if cfg.active && !cfg.dry_run {
        tracing::warn!(
            "active mode requested: make sure no legacy instance is using the same ports"
        );
    }
    let workers: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> =
        Arc::new(Mutex::new(Vec::new()));
    let result = tokio::select! {
        result = web::serve(state, workers.clone()) => result,
        _ = shutdown_signal() => Ok(()),
    };
    // Stop the background workers *before* touching the native session: a
    // still-running event worker polls libtorrent alerts and would consume the
    // `save_resume_data` alerts that `torrents.shutdown` is waiting for,
    // losing fastresume data (and racing native calls during teardown).
    {
        let handles = std::mem::take(&mut *workers.lock().unwrap());
        for handle in &handles {
            handle.abort();
        }
        for handle in handles {
            let _ = handle.await;
        }
    }
    tracing::info!("background workers stopped, saving libtorrent session");
    if let Err(error) = torrents.shutdown(&cfg) {
        tracing::error!(%error, "libtorrent shutdown did not save all fastresume data");
    }
    if let Err(error) = &result {
        tracing::error!(%error, "web server stopped with error");
    }
    // Flush dei log e uscita immediata: il teardown del runtime e della sessione
    // libtorrent può bloccarsi a lungo (osservato ~90 s in `stop-sigterm`),
    // rendendo lenti i riavvii. I resume data sono già stati salvati sopra.
    drop(_log_guard);
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .expect("install Ctrl-C handler");
}
