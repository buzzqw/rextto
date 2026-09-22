use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
};

/// Formatter che scrive i timestamp in ora locale (non UTC).
pub struct LocalTime;

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
