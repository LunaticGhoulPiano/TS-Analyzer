#![cfg_attr(windows, windows_subsystem = "windows")]
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;

use tsan_platform::{distribution, paths};

fn safe_version(value: &str) -> bool {
    !value.is_empty()
        && !value.contains("..")
        && value.bytes().all(|c| c.is_ascii_digit() || c == b'.')
}

fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
    })
}

fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn launch(
    package: &Path,
    args: &[std::ffi::OsString],
    wait: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let windows = paths::windows_directory()?;
    let package_version = field(
        &fs::read_to_string(package.join("package.toml"))?,
        "version",
    )
    .filter(|value| safe_version(value))
    .ok_or("Invalid package version")?;
    let mut runtime_id = std::hash::DefaultHasher::new();
    package.hash(&mut runtime_id);
    let deployment = distribution::at_root(package)?;
    let cache_root = match deployment.mode {
        distribution::Mode::Portable => package.join("data/cache"),
        distribution::Mode::Installed => paths::cache_directory()?,
    };
    let cache = cache_root
        .join(package_version)
        .join(format!("{:016x}", runtime_id.finish()));
    if !args.first().is_some_and(|a| a == "--build-info") {
        fs::create_dir_all(&cache)?;
    }
    let gst = package.join("runtime/gstreamer");
    let mut command = hidden_command(package.join("app/tsan-gui.exe"));
    command
        .args(args)
        .current_dir(package)
        .env(
            "PATH",
            env::join_paths([
                package.join("app"),
                package.join("runtime/msvc"),
                package.join("runtime/tsduck"),
                gst.join("bin"),
                windows.join("System32"),
                windows,
            ])?,
        )
        .env("TSAN_PACKAGE_ROOT", package)
        .env("TSAN_CACHE_ROOT", &cache_root)
        .env("TSAN_TEX_ROOT", package.join("runtime/tex"))
        .env("GST_PLUGIN_PATH", "")
        .env("GST_PLUGIN_PATH_1_0", "")
        .env("GST_PLUGIN_SYSTEM_PATH", gst.join("lib/gstreamer-1.0"))
        .env("GST_PLUGIN_SYSTEM_PATH_1_0", gst.join("lib/gstreamer-1.0"))
        .env(
            "GST_PLUGIN_SCANNER",
            gst.join("libexec/gstreamer-1.0/gst-plugin-scanner.exe"),
        )
        .env(
            "GST_PLUGIN_SCANNER_1_0",
            gst.join("libexec/gstreamer-1.0/gst-plugin-scanner.exe"),
        )
        .env("GST_REGISTRY", cache.join("gstreamer.bin"))
        .env("GST_REGISTRY_1_0", cache.join("gstreamer.bin"));
    if deployment.mode == distribution::Mode::Portable && env::var_os("TSAN_CONFIG_PATH").is_none()
    {
        command.env("TSAN_CONFIG_PATH", package.join("data/tsan-config.toml"));
    }
    let mut child = command.spawn()?;
    if wait && !child.wait()?.success() {
        return Err("TS Analyzer self-test failed".into());
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    if tsan_platform::OperatingSystem::current() != tsan_platform::OperatingSystem::Windows {
        return Err(tsan_platform::OperatingSystem::current()
            .unavailable("Package launcher")
            .into());
    }
    let exe = env::current_exe()?;
    let directory = exe.parent().ok_or("Missing executable directory")?;
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.iter().any(|a| a == "--install" || a == "--rollback") {
        return Err("Use the Windows installer to install or repair TS Analyzer.".into());
    }
    let package = directory.to_owned();
    let wait = args.iter().any(|a| a == "--wait");
    let args = args
        .into_iter()
        .filter(|a| a != "--wait")
        .collect::<Vec<_>>();
    launch(&package, &args, wait)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn report_error(message: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            window: *mut std::ffi::c_void,
            text: *const u16,
            title: *const u16,
            kind: u32,
        ) -> i32;
    }
    let text = message.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let title = "TS Analyzer\0".encode_utf16().collect::<Vec<_>>();
    // SAFETY: Both strings are NUL-terminated and valid for the synchronous call.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10);
    }
}
#[cfg(not(windows))]
fn report_error(message: &str) {
    eprintln!("{message}");
}

fn main() -> std::process::ExitCode {
    if let Some(exit) = tsan_diagnostics::run_helper_if_requested() {
        return exit;
    }
    let build_info = env::args_os().any(|s| s == "--build-info");
    let _diagnostics = if build_info {
        None
    } else {
        let version = env::current_exe()
            .ok()
            .and_then(|exe| fs::read_to_string(exe.parent()?.join("package.toml")).ok())
            .and_then(|text| field(&text, "version"))
            .filter(|version| safe_version(version))
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        match tsan_diagnostics::initialize("launcher", &version) {
            Ok(session) => Some(session),
            Err(error) => {
                eprintln!("Diagnostics initialization failed: {error}");
                None
            }
        }
    };
    if let Err(error) = run() {
        tsan_diagnostics::record("ERROR", "Launcher", &error.to_string());
        if env::var_os("TSAN_HEADLESS_TEST").is_some() {
            if let Some(path) = env::var_os("TSAN_ERROR_FILE") {
                let _ = fs::write(path, error.to_string());
            }
        } else {
            report_error(&error.to_string());
        }
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
