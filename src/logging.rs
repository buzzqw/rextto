use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{LazyLock, Mutex},
};
use tracing::{
    field::{Field, Visit},
    Event, Subscriber,
};
use tracing_subscriber::{
    field::RecordFields,
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields},
    registry::LookupSpan,
};

/// Formatter che scrive i timestamp in ora locale (non UTC).
pub struct LocalTime;

/// Human-readable byte size, e.g. `1.4 GB` / `742.3 MB` / `12.0 KB`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [(&str, f64); 5] = [
        ("TB", 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("GB", 1024.0 * 1024.0 * 1024.0),
        ("MB", 1024.0 * 1024.0),
        ("KB", 1024.0),
        ("B", 1.0),
    ];
    let bytes = bytes as f64;
    for (unit, size) in UNITS {
        if bytes >= size || unit == "B" {
            let value = bytes / size;
            return if unit == "B" {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            };
        }
    }
    unreachable!()
}

/// Same as [`human_bytes`] but tolerant of the `i64` values used across the DB.
pub fn human_bytes_i64(bytes: i64) -> String {
    if bytes <= 0 {
        "0 B".to_string()
    } else {
        human_bytes(bytes as u64)
    }
}

/// Human-readable duration from whole seconds, e.g. `2h 05m` / `3m 12s` / `8s`.
pub fn human_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {secs:02}s")
    } else {
        format!("{secs}s")
    }
}

/// Human-readable transfer rate, e.g. `8.3 MB/s` / `420.0 KB/s`.
pub fn human_rate(bytes_per_second: i64) -> String {
    if bytes_per_second <= 0 {
        "0 B/s".to_string()
    } else {
        format!("{}/s", human_bytes(bytes_per_second as u64))
    }
}

/// Per-source outcome accumulated during a scrape cycle. Sources are RSS/HTML
/// feeds, Torznab indexers and web search engines; the engine drains these at
/// the end of a cycle to print a single "source report" instead of flooding the
/// log with one line per attempt.
#[derive(Default, Clone)]
pub struct SourceStat {
    pub ok: usize,
    pub fail: usize,
    pub results: usize,
    pub last_error: Option<String>,
}

impl SourceStat {
    fn record_ok(&mut self, results: usize) {
        self.ok += 1;
        self.results += results;
    }

    fn record_fail(&mut self, error: &str) {
        self.fail += 1;
        self.last_error = Some(error.to_string());
    }
}

static SOURCE_STATS: LazyLock<Mutex<BTreeMap<String, SourceStat>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn source_key(kind: &str, name: &str) -> String {
    format!("{kind}\u{1}{name}")
}

/// Records a successful source attempt with the number of releases returned.
pub fn source_ok(kind: &str, name: &str, results: usize) {
    SOURCE_STATS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(source_key(kind, name))
        .or_default()
        .record_ok(results);
}

/// Records a failed source attempt with a redacted, human-readable reason.
pub fn source_fail(kind: &str, name: &str, error: &str) {
    SOURCE_STATS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(source_key(kind, name))
        .or_default()
        .record_fail(error);
}

/// Drains the accumulated per-source outcomes, sorted by kind then name.
pub fn take_source_stats() -> Vec<(String, String, SourceStat)> {
    let mut stats = SOURCE_STATS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *stats)
        .into_iter()
        .filter_map(|(key, stat)| {
            let (kind, name) = key.split_once('\u{1}')?;
            Some((kind.to_string(), name.to_string(), stat))
        })
        .collect()
}

/// Event formatter that produces one aligned, explicit line per log record:
///
/// ```text
/// 2026-09-23 12:00:00  INFO [engine] 🔎 Step 1/2: scanning 3 sources (HTML/RSS feeds)
/// 2026-09-23 12:00:01  WARN [web]    ❌ DOWNLOAD FAILED — stalled · hash: abc · title: X
/// ```
///
/// Timestamp uses local time, the level is right-aligned to a fixed width and
/// the source module is shortened to its last segment (`rextto::engine` →
/// `engine`), so the reader immediately sees *who* logged and *when*.
pub struct ReadableFormat;

impl<S, N> FormatEvent<S, N> for ReadableFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let metadata = event.metadata();
        let level = metadata.level();
        let component = short_component(metadata.target());
        write!(
            writer,
            "{} {level:>5} [{component}] ",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        )?;
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// `rextto::web::handlers` → `handlers`, `rextto::engine` → `engine`.
fn short_component(target: &str) -> &str {
    target
        .strip_prefix("rextto::")
        .unwrap_or(target)
        .rsplit("::")
        .next()
        .unwrap_or(target)
}

/// Field formatter used together with [`ReadableFormat`]: prints the message
/// first, then the structured fields as `key: value` separated by ` · `, which
/// keeps the *what* and the *why* on the same readable line.
pub struct ReadableFields;

impl<'writer> FormatFields<'writer> for ReadableFields {
    fn format_fields<R>(&self, mut writer: Writer<'writer>, fields: R) -> std::fmt::Result
    where
        R: RecordFields,
    {
        let mut visitor = ReadableVisitor {
            message: String::new(),
            fields: Vec::new(),
        };
        fields.record(&mut visitor);
        let mut line = visitor.message;
        for (index, (key, value)) in visitor.fields.iter().enumerate() {
            if index == 0 {
                if !line.is_empty() {
                    line.push_str(" · ");
                }
            } else {
                line.push_str(" · ");
            }
            line.push_str(key);
            line.push_str(": ");
            line.push_str(value);
        }
        write!(writer, "{line}")
    }
}

struct ReadableVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl ReadableVisitor {
    fn push(&mut self, name: &str, value: String) {
        if name == "message" {
            self.message = value;
        } else {
            self.fields.push((name.to_owned(), value));
        }
    }
}

impl Visit for ReadableVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.push(field.name(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field.name(), value.to_owned());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field.name(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field.name(), value.to_string());
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        self.push(field.name(), value.to_string());
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        self.push(field.name(), value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field.name(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field.name(), value.to_string());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.push(field.name(), value.to_string());
    }
}

impl tracing_subscriber::fmt::time::FormatTime for LocalTime {
    fn format_time(
        &self,
        writer: &mut tracing_subscriber::fmt::format::Writer<'_>,
    ) -> std::fmt::Result {
        write!(
            writer,
            "{}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        )
    }
}

/// Size-based rotating log writer: keeps `max_files` files of at most `max_bytes`
/// each (`rextto.log`, `rextto.log.1`, ... `rextto.log.(max_files-1)`).
pub struct RotatingWriter {
    dir: PathBuf,
    base: String,
    max_bytes: u64,
    max_files: usize,
    file: Option<File>,
    size: u64,
}

impl RotatingWriter {
    pub fn new(dir: PathBuf, base: &str, max_bytes: u64, max_files: usize) -> Self {
        Self {
            dir,
            base: base.to_string(),
            max_bytes: max_bytes.max(1024),
            max_files: max_files.max(1),
            file: None,
            size: 0,
        }
    }

    fn path(&self, index: usize) -> PathBuf {
        if index == 0 {
            self.dir.join(&self.base)
        } else {
            self.dir.join(format!("{}.{}", self.base, index))
        }
    }

    fn open(&mut self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let path = self.path(0);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        self.file = Some(file);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        if self.max_files <= 1 {
            let _ = fs::remove_file(self.path(0));
            return self.open();
        }
        let _ = fs::remove_file(self.path(self.max_files - 1));
        for index in (1..self.max_files - 1).rev() {
            let from = self.path(index);
            if from.exists() {
                let _ = fs::rename(&from, self.path(index + 1));
            }
        }
        let current = self.path(0);
        if current.exists() {
            let _ = fs::rename(&current, self.path(1));
        }
        self.open()
    }
}

impl Write for RotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.file.is_none() {
            self.open()?;
        }
        if self.size >= self.max_bytes {
            self.rotate()?;
        }
        let file = self.file.as_mut().expect("log file open");
        let written = file.write(buf)?;
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_human_readable_values() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(human_bytes_i64(-5), "0 B");
        assert_eq!(human_duration(0), "0s");
        assert_eq!(human_duration(45), "45s");
        assert_eq!(human_duration(192), "3m 12s");
        assert_eq!(human_duration(7500), "2h 05m");
        assert_eq!(human_rate(0), "0 B/s");
        assert_eq!(human_rate(1024 * 1024), "1.0 MB/s");
    }

    #[test]
    fn accumulates_and_drains_source_stats() {
        // Isolate from other tests that may run in parallel.
        let _ = take_source_stats();
        source_ok("feed", "example.org", 3);
        source_ok("feed", "example.org", 2);
        source_fail("indexer", "Prowlarr", "HTTP 500");
        let stats = take_source_stats();
        assert_eq!(stats.len(), 2);
        let (kind, name, stat) = &stats[0];
        assert_eq!(kind, "feed");
        assert_eq!(name, "example.org");
        assert_eq!(stat.ok, 2);
        assert_eq!(stat.results, 5);
        assert_eq!(stat.fail, 0);
        assert_eq!(stats[1].2.fail, 1);
        assert_eq!(stats[1].2.last_error.as_deref(), Some("HTTP 500"));
        assert!(take_source_stats().is_empty());
    }

    #[test]
    fn readable_formatter_prints_aligned_and_explicit_lines() {
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
            type Writer = Capture;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        use tracing_subscriber::layer::SubscriberExt;

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .event_format(ReadableFormat)
                .fmt_fields(ReadableFields)
                .with_writer(Capture(buffer.clone())),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(hash = "abc", title = "Example", "📥 DOWNLOAD STARTED");
        });
        let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(
            output.contains("INFO [tests] 📥 DOWNLOAD STARTED · hash: abc · title: Example"),
            "unexpected layout: {output}"
        );
    }

    #[test]
    fn rotates_at_max_bytes_and_keeps_all_lines() {
        let dir = std::env::temp_dir().join(format!("rextto-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // 1 KB cap (minimum accepted), 3 files total. Enough writes to rotate
        // at least twice.
        let mut writer = RotatingWriter::new(dir.clone(), "test.log", 1024, 3);
        for index in 0..400 {
            // One write per line so rotation never splits a line (as tracing
            // writes each record as a single chunk).
            let line = format!("line-{index:04}\n");
            writer.write_all(line.as_bytes()).unwrap();
        }
        writer.flush().unwrap();
        assert!(dir.join("test.log").exists());
        assert!(dir.join("test.log.1").exists());
        assert!(dir.join("test.log.2").exists());
        // max_files = 3: the oldest rotation is dropped, never kept as .3.
        assert!(!dir.join("test.log.3").exists());
        // Rotated content is intact and the active file holds the newest line.
        for name in ["test.log.1", "test.log.2"] {
            let content = fs::read_to_string(dir.join(name)).unwrap();
            assert!(content.lines().all(|line| line.starts_with("line-")));
        }
        let active = fs::read_to_string(dir.join("test.log")).unwrap();
        assert!(active.contains("line-0399"));
        // Each retained file stays within the cap (plus the last line).
        for name in ["test.log", "test.log.1", "test.log.2"] {
            let size = fs::metadata(dir.join(name)).unwrap().len();
            assert!(size <= 1024 + 16, "{name} too big: {size}");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
