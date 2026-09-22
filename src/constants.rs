/// Intervallo predefinito tra le ricerche automatiche di serie e film (6 ore).
/// I fumetti hanno invece il proprio intervallo settimanale.
pub const DEFAULT_REFRESH_SECS: u64 = 21_600;
pub const DEFAULT_LISTEN: &str = "127.0.0.1:5000";
pub const DEFAULT_ENGINE_PORT: u16 = 8889;
pub const DEFAULT_ENGINE_LISTEN: &str = "127.0.0.1:8889";
pub const DEFAULT_DB_FILE: &str = "rextto_series.db";
pub const DEFAULT_ARCHIVE_FILE: &str = "rextto_archive.db";
pub const DEFAULT_CONFIG_FILE: &str = "rextto_config.db";
pub const DEFAULT_STATE_DIR: &str = "rextto_torrents_state";
