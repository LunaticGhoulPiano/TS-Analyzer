//! Isolated crash fixture; never included in user packages.
#![allow(clippy::panic)]
fn main() -> std::process::ExitCode {
    if let Some(exit) = tsan_diagnostics::run_helper_if_requested() {
        return exit;
    }
    let session = match tsan_diagnostics::initialize("diagnostics-probe", "0.0.0") {
        Ok(session) => session,
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    println!("SESSION={}", session.directory().display());
    tsan_diagnostics::record("INFO", "Test", "event-before-failure");
    match std::env::args().nth(1).as_deref() {
        Some("panic") => panic!("diagnostics-panic-fixture"),
        Some("worker-panic") => {
            let worker = std::thread::Builder::new()
                .name("probe-worker".into())
                .spawn(|| panic!("diagnostics-worker-panic-fixture"));
            if let Ok(worker) = worker {
                let _ = worker.join();
            }
        }
        Some("native") => native_crash(),
        Some("abrupt") => std::process::exit(42),
        Some("flood") => {
            for i in 0..1000 {
                tsan_diagnostics::record("INFO", "Test", &format!("{i}:{}", "x".repeat(16384)));
            }
        }
        _ => (),
    }
    drop(session);
    std::process::ExitCode::SUCCESS
}
#[cfg(windows)]
#[allow(unsafe_code)]
fn native_crash() {
    // SAFETY: Deliberately terminate ONLY this disposable fixture process with an SEH exception.
    unsafe {
        windows::Win32::System::Diagnostics::Debug::RaiseException(0xE0421001, 1, None);
    }
}
#[cfg(not(windows))]
fn native_crash() {
    eprintln!("Native crash fixture is not implemented on this OS");
}
