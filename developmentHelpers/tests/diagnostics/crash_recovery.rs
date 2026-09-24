use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
#[cfg(windows)]
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

fn run(root: &Path, mode: &str) -> io::Result<(Output, PathBuf)> {
    let test = std::env::current_exe()?;
    let profile = test
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("No Cargo test directory"))?;
    let probe = profile
        .join("examples")
        .join(format!("diagnostics_probe{}", std::env::consts::EXE_SUFFIX));
    let platform = root.with_file_name("platform-fallback");
    let output = Command::new(probe)
        .arg(mode)
        .env("TSAN_DIAGNOSTICS_DIR", root)
        .env("LOCALAPPDATA", &platform)
        .env("APPDATA", &platform)
        .env("XDG_STATE_HOME", &platform)
        .env("HOME", &platform)
        .output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let session = text
        .lines()
        .find_map(|line| line.strip_prefix("SESSION="))
        .ok_or_else(|| {
            io::Error::other(format!(
                "No session: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        })?;
    let path = PathBuf::from(session);
    Ok((output, path))
}
#[cfg(windows)]
fn await_file(path: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!("Timed out: {}", path.display())));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

#[test]
fn logs_and_panic_survive_process_exit() -> Result<(), Box<dyn std::error::Error>> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../outputs")
        .join(std::env::consts::OS)
        .join("diagnostics-tests")
        .join(format!("run-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&root)?;
    let root = fs::canonicalize(root)?;
    let (normal, normal_session) = run(&root, "normal")?;
    assert!(normal.status.success());
    assert!(
        fs::read_to_string(normal_session.join("application.log"))?
            .contains("event-before-failure")
    );
    assert_eq!(
        fs::read_to_string(normal_session.join("session-end.txt"))?,
        "normal\n"
    );
    let (panic, panic_session) = run(&root, "panic")?;
    assert!(!panic.status.success());
    let trace = fs::read_to_string(panic_session.join("panic.log"))?;
    assert!(trace.contains("diagnostics-panic-fixture"));
    assert!(trace.contains("diagnostics_probe"));
    assert!(
        fs::read_to_string(panic_session.join("application.log"))?.contains("event-before-failure")
    );
    let (worker, worker_session) = run(&root, "worker-panic")?;
    assert!(worker.status.success());
    assert!(fs::read_to_string(worker_session.join("panic.log"))?.contains("probe-worker"));
    let (abrupt, abrupt_session) = run(&root, "abrupt")?;
    assert_eq!(abrupt.status.code(), Some(42));
    assert!(
        fs::read_to_string(abrupt_session.join("application.log"))?
            .contains("event-before-failure")
    );
    assert!(!abrupt_session.join("session-end.txt").exists());
    #[cfg(windows)]
    {
        await_file(&abrupt_session.join("process-exit.txt"))?;
        assert!(
            fs::read_to_string(abrupt_session.join("process-exit.txt"))?.contains("0x0000002A")
        );
        let (native, native_session) = run(&root, "native")?;
        assert!(!native.status.success());
        let dump = fs::read(native_session.join("crash.dmp"))?;
        assert_eq!(&dump[..4], b"MDMP");
        // Validate ExceptionStream, including the original fault code and thread context.
        let u32_at = |offset: usize| {
            u32::from_le_bytes(dump[offset..offset + 4].try_into().unwrap_or_default())
        };
        let count = u32_at(8) as usize;
        let directory = u32_at(12) as usize;
        let stream = (0..count)
            .map(|i| directory + i * 12)
            .find(|offset| u32_at(*offset) == 6)
            .ok_or("No exception stream")?;
        let exception = u32_at(stream + 8) as usize;
        assert_ne!(u32_at(exception), 0); // faulting thread ID
        assert_eq!(u32_at(exception + 8), 0xE0421001);
        assert_ne!(u32_at(exception + 160), 0); // context byte length
        assert!(
            fs::read_to_string(native_session.join("native-crash.txt"))?.contains("Dump complete")
        );
    }
    // A preferred path that is a file must fall back to a writable platform location.
    let bad_root = root.join("not-a-directory");
    fs::write(&bad_root, "keep")?;
    let (fallback, fallback_session) = run(&bad_root, "normal")?;
    assert!(fallback.status.success());
    assert!(!fallback_session.starts_with(&bad_root));
    assert!(
        fs::read_to_string(fallback_session.join("application.log"))?
            .contains("Cannot write diagnostics")
    );
    assert_eq!(fs::read_to_string(&bad_root)?, "keep");
    // Keep these diagnostic fixtures for examination; the test never opens a GUI.
    println!("Diagnostic fixtures: {}", root.display());
    Ok(())
}
