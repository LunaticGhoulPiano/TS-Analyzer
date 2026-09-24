mod config;
use tsan_platform::distribution;
mod frontend;
mod os_integration;
#[cfg(target_os = "windows")]
mod plot_range;
mod release;
#[cfg(target_os = "windows")]
mod report_graphics;
mod update;
#[cfg(target_os = "windows")]
mod xlsx;

#[cfg(target_os = "windows")]
mod analysis_views;
#[cfg(target_os = "windows")]
mod app;
#[cfg(target_os = "windows")]
mod liquid_glass;
#[cfg(target_os = "windows")]
mod logging;
#[cfg(target_os = "windows")]
mod platform;
#[cfg(target_os = "windows")]
mod player_worker;
#[cfg(target_os = "windows")]
mod report_export;
#[cfg(target_os = "windows")]
mod ui_components;

fn main() -> std::process::ExitCode {
    if let Some(exit) = tsan_diagnostics::run_helper_if_requested() {
        return exit;
    }
    let build_info = std::env::args_os()
        .nth(1)
        .is_some_and(|s| s == "--build-info");
    let _diagnostics = if build_info {
        None
    } else {
        match tsan_diagnostics::initialize("gui", release::VERSION) {
            Ok(session) => Some(session),
            Err(error) => {
                eprintln!("Diagnostics initialization failed: {error}");
                None
            }
        }
    };
    match frontend::run(tsan_runtime::AnalysisService) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tsan_diagnostics::record("ERROR", "System", &error);
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
