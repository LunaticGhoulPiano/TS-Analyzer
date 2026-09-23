#![cfg_attr(windows, windows_subsystem = "windows")]
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION: &str = env!("TSAN_PACKAGE_VERSION");

fn safe_version(value: &str) -> bool {
    !value.is_empty() && !value.contains("..")
        && value.bytes().all(|c| c.is_ascii_digit() || c == b'.')
}

fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
    })
}

fn atomic_write(path: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = fs::File::create(&temp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)
}

fn copy_tree(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() { return Err(io::Error::other("Package contains a symbolic link")); }
        if kind.is_dir() { copy_tree(&entry.path(), &target.join(entry.file_name()))?; }
        else if kind.is_file() { fs::copy(entry.path(), target.join(entry.file_name()))?; }
    }
    Ok(())
}

fn install(package: &Path, no_launch: bool) -> Result<(), Box<dyn std::error::Error>> {
    let custom = env::var_os("TSAN_INSTALL_ROOT");
    let install_root = custom.clone().map(PathBuf::from).unwrap_or(
        PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
            .join("Programs/TS-Analyzer"));
    let releases = install_root.join("releases");
    fs::create_dir_all(&releases)?;
    let destination = releases.join(format!("v{VERSION}"));
    let manifest = fs::read_to_string(package.join("package.toml"))?;
    if field(&manifest, "version").as_deref() != Some(VERSION) {
        return Err("Installer and package versions differ".into());
    }
    if destination.exists() {
        if fs::read_to_string(destination.join("package.toml"))? != manifest {
            return Err("This version is already installed with different contents; use a new release version".into());
        }
    } else {
        let staging = releases.join(format!(".staging-{}-{}", VERSION, std::process::id()));
        if staging.exists() { return Err("A previous installation staging directory needs inspection".into()); }
        copy_tree(package, &staging)?;
        fs::rename(staging, &destination)?;
    }
    let pointer = install_root.join("current-version.toml");
    let previous = fs::read_to_string(&pointer).ok().and_then(|s| {
        let current = field(&s, "version");
        if current.as_deref() == Some(VERSION) { field(&s, "previous_version") } else { current }
    });
    fs::copy(package.join("TS-Analyzer.exe"), install_root.join("TS-Analyzer.exe"))?;
    let previous = previous.filter(|v| safe_version(v)).unwrap_or_default();
    atomic_write(&pointer, &format!("version = \"{VERSION}\"\nprevious_version = \"{previous}\"\n"))?;
    if custom.is_none() {
        let script = r#"
$ErrorActionPreference = 'Stop'
$directory = Join-Path ([Environment]::GetFolderPath('Programs')) 'TS Analyzer'
New-Item -ItemType Directory -Force -Path $directory | Out-Null
$link = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path $directory 'TS Analyzer.lnk'))
$link.TargetPath = $env:TSAN_SHORTCUT_TARGET
$link.WorkingDirectory = [IO.Path]::GetDirectoryName($env:TSAN_SHORTCUT_TARGET)
$link.Save()
"#;
        let mut command = hidden_command(windows_powershell());
        command.env("PSModulePath", windows_powershell().parent().ok_or("Missing Windows PowerShell directory")?.join("Modules"));
        let status = command.args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("TSAN_SHORTCUT_TARGET", install_root.join("TS-Analyzer.exe")).status()?;
        if !status.success() { return Err("Installed, but could not create the Start menu shortcut".into()); }
    }
    if !no_launch { launch(&destination, &[], false)?; }
    Ok(())
}

fn windows_powershell() -> PathBuf {
    PathBuf::from(env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()))
        .join("System32/WindowsPowerShell/v1.0/powershell.exe")
}

fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn launch(package: &Path, args: &[std::ffi::OsString], wait: bool) -> Result<(), Box<dyn std::error::Error>> {
    let windows = PathBuf::from(env::var_os("SystemRoot").ok_or("SystemRoot is unavailable")?);
    let package_version = field(&fs::read_to_string(package.join("package.toml"))?, "version")
        .filter(|value| safe_version(value)).ok_or("Invalid package version")?;
    let mut runtime_id = std::hash::DefaultHasher::new();
    package.hash(&mut runtime_id);
    let cache = PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
        .join("TS-Analyzer/cache").join(package_version).join(format!("{:016x}", runtime_id.finish()));
    fs::create_dir_all(&cache)?;
    let gst = package.join("runtime/gstreamer");
    let mut command = hidden_command(package.join("app/tsan-gui.exe"));
    command.args(args).current_dir(package)
        .env("PATH", env::join_paths([
            package.join("app"), package.join("runtime/msvc"), package.join("runtime/tsduck"),
            gst.join("bin"), windows.join("System32"), windows,
        ])?)
        .env("TSAN_PACKAGE_ROOT", package)
        .env("TSAN_TEX_ROOT", package.join("runtime/tex"))
        .env("GST_PLUGIN_PATH", "")
        .env("GST_PLUGIN_PATH_1_0", "")
        .env("GST_PLUGIN_SYSTEM_PATH", gst.join("lib/gstreamer-1.0"))
        .env("GST_PLUGIN_SYSTEM_PATH_1_0", gst.join("lib/gstreamer-1.0"))
        .env("GST_PLUGIN_SCANNER", gst.join("libexec/gstreamer-1.0/gst-plugin-scanner.exe"))
        .env("GST_PLUGIN_SCANNER_1_0", gst.join("libexec/gstreamer-1.0/gst-plugin-scanner.exe"))
        .env("GST_REGISTRY", cache.join("gstreamer.bin"))
        .env("GST_REGISTRY_1_0", cache.join("gstreamer.bin"));
    let mut child = command.spawn()?;
    if wait && !child.wait()?.success() { return Err("TS Analyzer self-test failed".into()); }
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    if !safe_version(VERSION) { return Err("Invalid package version".into()); }
    let exe = env::current_exe()?;
    let directory = exe.parent().ok_or("Missing executable directory")?;
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if exe.file_stem().is_some_and(|s| s == "Setup") || args.iter().any(|a| a == "--install") {
        return install(directory, args.iter().any(|a| a == "--no-launch"));
    }
    let package = if directory.join("app/tsan-gui.exe").is_file() {
        directory.to_owned()
    } else {
        let pointer = directory.join("current-version.toml");
        let text = fs::read_to_string(&pointer)?;
        let mut current = field(&text, "version").ok_or("Missing installed version")?;
        if args.iter().any(|a| a == "--rollback") {
            let previous = field(&text, "previous_version").ok_or("No previous version")?;
            if !safe_version(&previous) || !directory.join(format!("releases/v{previous}/app/tsan-gui.exe")).is_file() {
                return Err("Previous version is unavailable".into());
            }
            atomic_write(&pointer, &format!("version = \"{previous}\"\nprevious_version = \"{current}\"\n"))?;
            current = previous;
        }
        if !safe_version(&current) { return Err("Invalid installed version".into()); }
        directory.join("releases").join(format!("v{current}"))
    };
    let wait = args.iter().any(|a| a == "--wait");
    let args = args.into_iter().filter(|a| a != "--wait" && a != "--rollback").collect::<Vec<_>>();
    launch(&package, &args, wait)
}

#[cfg(windows)]
fn report_error(message: &str) {
    #[link(name = "user32")]
    unsafe extern "system" { fn MessageBoxW(window: *mut std::ffi::c_void, text: *const u16, title: *const u16, kind: u32) -> i32; }
    let text = message.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let title = "TS Analyzer\0".encode_utf16().collect::<Vec<_>>();
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10); }
}
#[cfg(not(windows))]
fn report_error(message: &str) { eprintln!("{message}"); }

fn main() {
    if let Err(error) = run() {
        if env::var_os("TSAN_HEADLESS_TEST").is_some() {
            if let Some(path) = env::var_os("TSAN_ERROR_FILE") { let _ = fs::write(path, error.to_string()); }
        } else { report_error(&error.to_string()); }
        std::process::exit(1);
    }
}
