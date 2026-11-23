//! File logging and a panic hook.
//!
//! A release build on Windows has no console (`windows_subsystem = "windows"`),
//! so a logger that only writes to stderr records nothing a user could send you
//! after a failure. Everything goes to `<data-dir>/freetokenmeter/logs/` as well.
//!
//! Deliberately dependency-free: `log` is already a dependency and the file
//! handling needed here is a few lines of `std`.
//!
//! # Level selection
//!
//! `FTM_LOG` (or `RUST_LOG`) selects the level by name: `error`, `warn`, `info`,
//! `debug`, `trace`. The default is `info`, which is what a support report needs
//! (startup, sync summaries, per-provider failures) without logging a line per
//! database row.

use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

struct FileLogger {
    file: Mutex<Option<std::fs::File>>,
    level: LevelFilter,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let line = format!(
            "{} {:<5} {}: {}\n",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            record.level(),
            record.target(),
            record.args()
        );

        // Harmless when there is no console; useful when running from a terminal.
        if record.level() <= Level::Warn {
            eprint!("{}", line);
        }

        if let Ok(mut guard) = self.file.lock() {
            if let Some(file) = guard.as_mut() {
                // A failed log write must never take down the application.
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = file.flush();
            }
        }
    }
}

fn level_from_env() -> LevelFilter {
    let raw = std::env::var("FTM_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_default()
        .to_ascii_lowercase();

    if raw.contains("trace") {
        LevelFilter::Trace
    } else if raw.contains("debug") {
        LevelFilter::Debug
    } else if raw.contains("error") {
        LevelFilter::Error
    } else if raw.contains("warn") {
        LevelFilter::Warn
    } else if raw.contains("off") {
        LevelFilter::Off
    } else {
        LevelFilter::Info
    }
}

/// Path of the log file inside `data_dir`.
pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("freetokenmeter.log")
}

/// Install the file logger. Returns the log path, or `None` when the log file
/// could not be created (logging must never prevent startup).
pub fn init(data_dir: &Path) -> Option<PathBuf> {
    let path = log_path(data_dir);
    let file = path.parent().and_then(|dir| {
        create_dir_all(dir).ok()?;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()
    });

    let level = level_from_env();
    let logger = FileLogger {
        file: Mutex::new(file),
        level,
    };

    let installed = log::set_boxed_logger(Box::new(logger)).is_ok();
    if installed {
        log::set_max_level(level);
    }

    if installed {
        Some(path)
    } else {
        None
    }
}

/// Write panics to the log file.
///
/// Installed even though the sync manager isolates provider panics: a panic that
/// escapes still needs a record, because in a GUI build there is nowhere else for
/// the message to go.
pub fn install_panic_hook(data_dir: &Path) {
    let path = log_path(data_dir);
    let previous = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "non-string panic payload".to_string()
        };

        let line = format!(
            "{} PANIC   {} at {} — {}\n",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            payload,
            location,
            info
        );

        if let Some(dir) = path.parent() {
            let _ = create_dir_all(dir);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        eprint!("{}", line);

        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_lives_under_the_data_dir() {
        let path = log_path(Path::new("data"));
        assert_eq!(
            path,
            Path::new("data").join("logs").join("freetokenmeter.log")
        );
    }

    #[test]
    fn init_creates_the_log_file() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-logging-{}-{}",
            std::process::id(),
            crate::test_support::unique_id()
        ));
        let path = init(&dir).expect("log file path");
        assert!(path.exists(), "the log file is created eagerly");

        // The logger is global, so this only asserts it is usable, not that a
        // second install replaces the first.
        log::info!("logging smoke test");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
