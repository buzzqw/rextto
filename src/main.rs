use anyhow::Result;
use rextto::{
    archive::Archive,
    cli::{self, Command},
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

#[tokio::main]
async fn main() -> Result<()> {
    let command = cli::parse(std::env::args().skip(1));
    match command {
        Command::Version => {
            println!("{}", rextto::update::version_string());
            return Ok(());
        }
        Command::Help => {
            print!("{}", cli::usage());
            return Ok(());
        }
        Command::Update(options) => {
            rextto::update::run(&options).await?;
            return Ok(());
        }
        Command::Import { source, data_dir } => {
            let report = importer::import_extto(&PathBuf::from(source), &PathBuf::from(data_dir))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        Command::Serve { dry_run, config } => run_daemon(dry_run, config).await,
    }
}

async fn run_daemon(dry_run: bool, config: Option<String>) -> Result<()> {
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
    let engine = Arc::new(Engine::with_db(db.clone()));
    let torrents = Arc::new(LibtorrentClient::new(&cfg)?);
    let i18n = Arc::new(rextto::i18n::I18nDb::open(
        &cfg.data_dir.join("rextto_config.db"),
    )?);
    // Merge the bundled translations (never overwrites user edits).
    let _ = i18n.seed_default_translations();
    if let Ok(language) = i18n.language() {
        rextto::messages::set_language(&language);
    }
    // Reclaim any WAL left oversized by a previous run: SQLite's default
    // autocheckpoint is PASSIVE and never shrinks the `-wal` file, and the fast
    // `process::exit` path skips the clean close that would truncate it.
    let _ = db.lock().unwrap().checkpoint();
    let _ = archive.lock().unwrap().checkpoint();
    let _ = comics.checkpoint();
    let _ = i18n.checkpoint();
    // Verifica l'integrità di ogni database all'avvio: crash, cadute di corrente
    // o spazio esaurito possono lasciare problemi che è meglio rilevare subito.
    for (name, result) in [
        ("rextto_series.db", db.lock().unwrap().quick_check()),
        ("rextto_archive.db", archive.lock().unwrap().quick_check()),
        ("rextto_comics.db", comics.quick_check()),
        ("rextto_config.db", i18n.quick_check()),
    ] {
        match result {
            Ok(rows) if rows.len() == 1 && rows[0] == "ok" => {
                tracing::info!(database = name, "integrity check: ok")
            }
            Ok(rows) => tracing::error!(
                database = name,
                detail = %rows.join(" | "),
                "integrity check: problemi rilevati"
            ),
            Err(error) => tracing::warn!(database = name, %error, "integrity check non eseguito"),
        }
    }
    let state = AppState {
        cfg: cfg.clone(),
        config_path,
        i18n: i18n.clone(),
        db: db.clone(),
        archive: archive.clone(),
        comics: comics.clone(),
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
        rename_progress: Arc::new(Mutex::new(rextto::web::RenameProgress::default())),
        config_cache: Arc::new(Mutex::new(None)),
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
        // Wait briefly so `torrents.shutdown` is not racing a native call, but
        // never block shutdown forever: a worker stuck in a blocking syscall
        // (e.g. a hung NFS mount) cannot be aborted, and an unbounded await here
        // would freeze the process so it never saves resume data nor exits.
        let stop = async {
            for handle in handles {
                let _ = handle.await;
            }
        };
        if tokio::time::timeout(std::time::Duration::from_secs(5), stop)
            .await
            .is_err()
        {
            tracing::warn!(
                "background workers did not stop within 5s (blocked I/O?); continuing shutdown"
            );
        }
    }
    tracing::info!("background workers stopped, saving libtorrent session");
    if let Err(error) = torrents.shutdown(&cfg) {
        tracing::error!(%error, "libtorrent shutdown did not save all fastresume data");
    }
    if let Err(error) = &result {
        tracing::error!(%error, "web server stopped with error");
    }
    // Truncate the WALs before the abrupt exit, otherwise the `-wal` files keep
    // their high-water mark until the next startup checkpoint.
    let _ = db.lock().unwrap().checkpoint();
    let _ = archive.lock().unwrap().checkpoint();
    let _ = comics.checkpoint();
    let _ = i18n.checkpoint();
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
