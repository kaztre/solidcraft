//! The app's log file: what SolidCraft logs (`log::warn!`, `log::info!`, and every command that
//! changes the design or fails), one line each, so a bug report can say what happened before it
//! (docs/crash-reports.md).
//!
//! - **Where**: `logs/<time>-<pid>.log` beside the crash folder ([`default_dir`]). Nothing until
//!   [`install`] is called: tests, the CLI and the web build never do.
//! - **Bounded**: one run's file stops at [`MAX_FILE_BYTES`] with a note saying so, and the newest
//!   [`MAX_FILES`] files are kept.
//! - **Unbuffered**: each line is appended as it is logged, so a crash loses nothing before it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// One run's log stops growing here.
pub const MAX_FILE_BYTES: u64 = 4 << 20;
/// Log files kept, this run's included.
pub const MAX_FILES: usize = 10;
/// A longer message is cut.
const MAX_MESSAGE: usize = 4000;

static PATH: OnceLock<PathBuf> = OnceLock::new();

struct FileLog {
    out: Mutex<Out>,
    /// The level for SolidCraft's crates.
    ours: LevelFilter,
    /// The level for every other crate.
    others: LevelFilter,
    cap: u64,
}

struct Out {
    file: std::fs::File,
    written: u64,
    full: bool,
}

impl Log for FileLog {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= if m.target().starts_with("solidcraft") { self.ours } else { self.others }
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let Ok(mut out) = self.out.lock() else { return };
        if out.full {
            return;
        }
        let ms = now_ms();
        let mut text = line(ms, r.level(), r.target(), &r.args().to_string());
        if out.written.saturating_add(text.len() as u64) > self.cap {
            out.full = true;
            text = line(ms, Level::Warn, "solidcraft_engine::session_log", "log full: nothing more is written this run");
        }
        // A full disk is no reason to stop the app.
        if out.file.write_all(text.as_bytes()).is_ok() {
            out.written = out.written.saturating_add(text.len() as u64);
        }
    }

    fn flush(&self) {
        if let Ok(mut out) = self.out.lock() {
            let _ = out.file.flush();
        }
    }
}

/// `2026-10-31T14:03:22.123Z warn solidcraft_kernel: message`, continuation lines indented.
fn line(ms: u64, level: Level, target: &str, message: &str) -> String {
    let message = if message.len() > MAX_MESSAGE {
        let mut end = MAX_MESSAGE;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        format!("{} […]", &message[..end])
    } else {
        message.to_string()
    };
    let time = crate::crash::utc(ms / 1000);
    format!(
        "{}.{:03}Z {} {target}: {}\n",
        time.trim_end_matches('Z'),
        ms % 1000,
        level.as_str().to_ascii_lowercase(),
        message.trim_end().replace('\n', "\n    ")
    )
}

/// The folder the app's logs go in: `logs/` beside the crash and recovery folders.
pub fn default_dir() -> Option<PathBuf> {
    crate::recovery::default_dir().and_then(|d| d.parent().map(|p| p.join("logs")))
}

/// Starts this run's log in `dir` (created if missing), after removing the oldest files beyond
/// [`MAX_FILES`]. `None` when the file can't be created or a logger is already installed; the app
/// then runs without a log. `RUST_LOG` raises the level (never below `info` for SolidCraft's own).
pub fn install(dir: &Path) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    prune(dir, MAX_FILES.saturating_sub(1));
    let path = dir.join(format!("{}-{}.log", crate::crash::stamp(now_ms() / 1000), std::process::id()));
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
    let env = std::env::var("RUST_LOG").ok().and_then(|v| v.trim().parse::<LevelFilter>().ok());
    let (ours, others) = env.map_or((LevelFilter::Info, LevelFilter::Warn), |all| (all.max(LevelFilter::Info), all));
    let logger = FileLog { out: Mutex::new(Out { file, written: 0, full: false }), ours, others, cap: MAX_FILE_BYTES };
    // Installed once, for the life of the process.
    if log::set_logger(Box::leak(Box::new(logger))).is_err() {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    log::set_max_level(ours.max(others));
    let _ = PATH.set(path.clone());
    Some(path)
}

/// This run's log file, once [`install`] has started one.
pub fn current() -> Option<&'static Path> {
    PATH.get().map(PathBuf::as_path)
}

/// Removes all but the newest `keep` logs in `dir`. Names start with the time, so they sort by age.
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut logs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "log")).collect();
    logs.sort();
    let extra = logs.len().saturating_sub(keep);
    for old in logs.iter().take(extra) {
        let _ = std::fs::remove_file(old);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("solidcraft-session-log-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn logger(path: &Path, cap: u64) -> FileLog {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        FileLog { out: Mutex::new(Out { file, written: 0, full: false }), ours: LevelFilter::Info, others: LevelFilter::Warn, cap }
    }

    fn emit(l: &FileLog, level: Level, target: &str, message: &str) {
        l.log(&Record::builder().level(level).target(target).args(format_args!("{message}")).build());
    }

    #[test]
    fn a_line_has_the_time_level_and_target_and_indents_continuations() {
        let l = line(1_793_455_402_123, Level::Warn, "solidcraft_kernel", "first\nsecond\n");
        assert_eq!(l, "2026-10-31T14:03:22.123Z warn solidcraft_kernel: first\n    second\n");
    }

    #[test]
    fn a_long_message_is_cut_on_a_character_boundary() {
        let l = line(0, Level::Info, "solidcraft_engine", &"é".repeat(MAX_MESSAGE));
        assert!(l.len() < MAX_MESSAGE + 100, "{}", l.len());
        assert!(l.trim_end().ends_with("[…]"));
    }

    #[test]
    fn our_info_is_kept_and_other_crates_only_warn() {
        let d = dir("levels");
        let path = d.join("run.log");
        let l = logger(&path, MAX_FILE_BYTES);
        emit(&l, Level::Info, "solidcraft_engine", "ours");
        emit(&l, Level::Debug, "solidcraft_engine", "our debug");
        emit(&l, Level::Info, "wgpu_core", "theirs");
        emit(&l, Level::Warn, "wgpu_core", "their warning");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("solidcraft_engine: ours"));
        assert!(!text.contains("our debug"));
        assert!(!text.contains("theirs"));
        assert!(text.contains("wgpu_core: their warning"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_full_log_says_so_once_and_stops_growing() {
        let d = dir("full");
        let path = d.join("run.log");
        let l = logger(&path, 300);
        for i in 0..50 {
            emit(&l, Level::Warn, "solidcraft_engine", &format!("message {i}"));
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("log full").count(), 1);
        assert!(text.len() < 300 + 200, "{}", text.len());
        assert!(!text.contains("message 49"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn prune_keeps_the_newest_logs_and_nothing_else_is_touched() {
        let d = dir("prune");
        for name in ["20261001-000000-1.log", "20261002-000000-1.log", "20261003-000000-1.log", "notes.txt"] {
            std::fs::write(d.join(name), "x").unwrap();
        }
        prune(&d, 1);
        let mut left: Vec<String> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["20261003-000000-1.log", "notes.txt"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
