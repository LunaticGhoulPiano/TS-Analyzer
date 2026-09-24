//! Shared persistent diagnostics. Native dump collection is isolated by OS.
use std::backtrace::Backtrace;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

mod retention;
#[cfg(not(windows))]
mod unsupported;
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;
#[cfg(not(windows))]
use unsupported as native;
#[cfg(windows)]
use windows as native;

const LOG_LIMIT: u64 = 2 * 1024 * 1024;
const LOG_BACKUPS: usize = 3;
const MAX_MESSAGE: usize = 16 * 1024;
const PANIC_LIMIT: u64 = 1024 * 1024;
const SESSION_MARKER: &str = "TS-Analyzer diagnostics schema 1\n";
static LOGGER: OnceLock<Logger> = OnceLock::new();
static PANICKED: AtomicBool = AtomicBool::new(false);

struct Logger {
    session: PathBuf,
    output: Mutex<RotatingLog>,
    panic: Mutex<File>,
    failure: Mutex<Option<String>>,
    native_status: String,
}

pub struct Session {
    directory: PathBuf,
    // Keep the shared lock held until the process and its dump helper finish.
    _lock: File,
    _native: Option<native::Guard>,
}

impl Session {
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        record(
            "INFO",
            "System",
            if PANICKED.load(Ordering::Relaxed) {
                "Session ended after a Rust panic; see panic.log"
            } else {
                "Session ended normally"
            },
        );
        if let Some(logger) = LOGGER.get()
            && let Ok(mut log) = logger.output.lock()
        {
            let _ = log.sync();
        }
        let status = if PANICKED.load(Ordering::Relaxed) {
            "rust-panic"
        } else {
            "normal"
        };
        let _ = fs::write(
            self.directory.join("session-end.txt"),
            format!("{status}\n"),
        );
    }
}

fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

fn bounded(value: &str, bytes: usize) -> &str {
    let mut end = value.len().min(bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

// A single log entry cannot inject additional apparent log records.
fn message(value: &str) -> String {
    let mut text = bounded(value, MAX_MESSAGE)
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    if value.len() > MAX_MESSAGE {
        text.push_str(" [truncated]");
    }
    text
}

fn candidate_roots(exe: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut candidates = Vec::new();
    let mut warnings = Vec::new();
    if let Some(value) = std::env::var_os("TSAN_DIAGNOSTICS_DIR") {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            candidates.push(path);
        } else {
            warnings
                .push("TSAN_DIAGNOSTICS_DIR must be absolute; using a platform fallback".into());
        }
    }
    let parent = exe.parent();
    let package = parent.and_then(|p| {
        if p.file_name().is_some_and(|n| n == "app") {
            p.parent()
        } else {
            Some(p)
        }
    });
    if let Some(root) = package.filter(|p| p.join("deployment.toml").is_file()) {
        match tsan_platform::distribution::at_root(root) {
            Ok(d) if d.mode == tsan_platform::distribution::Mode::Portable => {
                candidates.push(root.join("data/diagnostics"))
            }
            Ok(_) => (),
            Err(error) => warnings.push(format!("Deployment diagnostics path: {error}")),
        }
    }
    match tsan_platform::paths::user_directories() {
        Ok(paths) => candidates.push(paths.diagnostics),
        Err(error) => warnings.push(error),
    }
    let temporary = std::env::temp_dir();
    if temporary.is_absolute() {
        candidates.push(temporary.join("TS-Analyzer/diagnostics"));
    }
    candidates.dedup();
    (candidates, warnings)
}

pub(crate) fn session_lock(session: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(session.join("session.lock"))?;
    file.lock_shared()?;
    Ok(file)
}

fn create_session(root: &Path, name: &str) -> io::Result<(PathBuf, File, RotatingLog, File)> {
    fs::create_dir_all(root)?;
    let session = root.join(name);
    fs::create_dir(&session)?;
    let lock = session_lock(&session)?;
    fs::write(session.join("identity.txt"), SESSION_MARKER)?;
    let output = RotatingLog::new(&session, LOG_LIMIT)?;
    let panic = File::create(session.join("panic.log"))?;
    Ok((session, lock, output, panic))
}

/// Call once, before starting application workers. Failure does not prevent the app running.
pub fn initialize(component: &str, version: &str) -> Result<Session, String> {
    if LOGGER.get().is_some() {
        return Err("Diagnostics is already initialized".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (candidates, mut warnings) = candidate_roots(&exe);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let name = format!("session-{nanos:020}-{}", std::process::id());
    let mut selected = None;
    for root in candidates {
        match create_session(&root, &name) {
            Ok(value) => {
                selected = Some((root, value));
                break;
            }
            Err(error) => warnings.push(format!(
                "Cannot write diagnostics at {}: {error}",
                root.display()
            )),
        }
    }
    let (root, (directory, lock, output, panic)) = selected.ok_or_else(|| warnings.join("; "))?;
    if let Err(error) = retention::prune(&root, 20, 256 * 1024 * 1024) {
        warnings.push(format!("Diagnostics retention: {error}"));
    }
    let metadata = format!(
        "{SESSION_MARKER}component={}\nversion={}\nos={}\narch={}\npid={}\nstarted_unix_ms={}\nexecutable={}\ndebug_build={}\n",
        message(component),
        message(version),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::process::id(),
        now(),
        message(&exe.display().to_string()),
        cfg!(debug_assertions)
    );
    fs::write(directory.join("session.txt"), metadata).map_err(|e| e.to_string())?;
    let (native_guard, native_status) = match native::install(&directory) {
        Ok(guard) => (
            Some(guard),
            "Windows out-of-process minidump collector ready".to_owned(),
        ),
        Err(error) => {
            warnings.push(error.clone());
            (None, error)
        }
    };
    LOGGER
        .set(Logger {
            session: directory.clone(),
            output: Mutex::new(output),
            panic: Mutex::new(panic),
            failure: Mutex::new(None),
            native_status,
        })
        .map_err(|_| "Diagnostics was concurrently initialized".to_owned())?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        PANICKED.store(true, Ordering::Relaxed);
        let thread = std::thread::current();
        let text = format!(
            "unix_ms={} thread={} ({:?})\n{}\n{}\n\n",
            now(),
            thread.name().unwrap_or("unnamed"),
            thread.id(),
            info,
            Backtrace::force_capture()
        );
        if let Some(logger) = LOGGER.get() {
            if let Ok(mut file) = logger.panic.try_lock() {
                let length = file.metadata().map(|m| m.len()).unwrap_or(PANIC_LIMIT);
                let available = PANIC_LIMIT.saturating_sub(length).min(128 * 1024) as usize;
                if available > 0 {
                    let _ = file.write_all(bounded(&text, available).as_bytes());
                    let _ = file.sync_data();
                }
            }
            // Never wait on the normal log mutex from a panic hook.
            if let Ok(mut output) = logger.output.try_lock() {
                let _ = output.write(
                    &format!("{} [ERROR] [Panic] {}\n", now(), message(&info.to_string())),
                    true,
                );
            }
        }
        previous_hook(info);
    }));
    record(
        "INFO",
        "System",
        &format!("Diagnostics initialized: {}", directory.display()),
    );
    record("INFO", "System", &status());
    for warning in warnings {
        record("WARN", "Diagnostics", &warning);
        eprintln!("{warning}");
    }
    Ok(Session {
        directory,
        _lock: lock,
        _native: native_guard,
    })
}

pub fn directory() -> Option<&'static Path> {
    LOGGER.get().map(|l| l.session.as_path())
}

pub fn status() -> String {
    match LOGGER.get() {
        None => "Persistent diagnostics unavailable".into(),
        Some(logger) => {
            let failure = logger.failure.lock().ok().and_then(|f| f.clone());
            format!(
                "{}; {}{}",
                logger.session.display(),
                logger.native_status,
                failure
                    .map(|e| format!("; write failure: {e}"))
                    .unwrap_or_default()
            )
        }
    }
}

/// Records are written without a userspace buffer; errors are synced to storage.
pub fn record(level: &str, category: &str, text: &str) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    let line = format!(
        "{} [{}] [{}] {}\n",
        now(),
        message(level),
        message(category),
        message(text)
    );
    let result = logger
        .output
        .lock()
        .map_err(|_| "Log mutex poisoned".into())
        .and_then(|mut log| {
            log.write(&line, level == "ERROR")
                .map_err(|e| e.to_string())
        });
    if let Err(error) = result
        && let Ok(mut failure) = logger.failure.lock()
    {
        if failure.is_none() {
            eprintln!("Diagnostic log write failed: {error}");
        }
        *failure = Some(error);
    }
}

/// Invoke before initialize/frontend dispatch in every executable using diagnostics.
pub fn run_helper_if_requested() -> Option<std::process::ExitCode> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if !args.first().is_some_and(|s| s == "--tsan-crash-helper") {
        return None;
    }
    Some(match native::run_helper(&args[1..]) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Crash collector failed: {error}");
            std::process::ExitCode::FAILURE
        }
    })
}

struct RotatingLog {
    directory: PathBuf,
    file: Option<File>,
    length: u64,
    limit: u64,
}
impl RotatingLog {
    fn new(directory: &Path, limit: u64) -> io::Result<Self> {
        Ok(Self {
            directory: directory.into(),
            file: Some(File::create(directory.join("application.log"))?),
            length: 0,
            limit,
        })
    }
    fn sync(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("Log file unavailable"))?
            .sync_data()
    }
    fn write(&mut self, text: &str, critical: bool) -> io::Result<()> {
        if self.length > 0 && self.length + text.len() as u64 > self.limit {
            self.sync()?;
            self.file.take();
            let oldest = self
                .directory
                .join(format!("application.{LOG_BACKUPS}.log"));
            if oldest.is_file() {
                fs::remove_file(oldest)?;
            }
            for index in (1..LOG_BACKUPS).rev() {
                let previous = self.directory.join(format!("application.{index}.log"));
                if previous.is_file() {
                    fs::rename(
                        previous,
                        self.directory
                            .join(format!("application.{}.log", index + 1)),
                    )?;
                }
            }
            fs::rename(
                self.directory.join("application.log"),
                self.directory.join("application.1.log"),
            )?;
            self.file = Some(File::create(self.directory.join("application.log"))?);
            self.length = 0;
        }
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("Log file unavailable"))?
            .write_all(text.as_bytes())?;
        self.length += text.len() as u64;
        if critical {
            self.sync()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotation_retains_newest_records_with_bounded_files() -> io::Result<()> {
        let root =
            std::env::temp_dir().join(format!("tsan-log-test-{}-{}", std::process::id(), now()));
        fs::create_dir(&root)?;
        {
            let mut log = RotatingLog::new(&root, 10)?;
            for index in 0..20 {
                log.write(&format!("record-{index:02}\n"), true)?;
            }
        }
        assert_eq!(
            fs::read_to_string(root.join("application.log"))?,
            "record-19\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("application.3.log"))?,
            "record-16\n"
        );
        assert_eq!(fs::read_dir(&root)?.count(), 4);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
