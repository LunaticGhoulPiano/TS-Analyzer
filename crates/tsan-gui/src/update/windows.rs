use super::*;
fn powershell() -> Result<std::process::Command, String> {
    let root = tsan_platform::paths::windows_directory()?;
    let mut command = crate::os_integration::background_command(
        root.join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    );
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
    ]);
    command.env(
        "PSModulePath",
        root.join("System32/WindowsPowerShell/v1.0/Modules"),
    );
    Ok(command)
}
pub fn fetch_release(distribution: &Distribution) -> Result<String, String> {
    let script = distribution.root.join("scripts/windows/fetch-release.ps1");
    if !script.is_file() {
        return Err(format!(
            "Update check script is missing: {}",
            script.display()
        ));
    }
    let output = powershell()?
        .arg("-File")
        .arg(&script)
        .args([
            "-TargetOs",
            std::env::consts::OS,
            "-TargetArch",
            std::env::consts::ARCH,
            "-Mode",
            distribution.mode.name(),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}
pub fn stage(release: &Release, distribution: &Distribution) -> Result<PathBuf, String> {
    let updates = match distribution.mode {
        Mode::Installed => tsan_platform::paths::update_directory()?,
        Mode::Portable => distribution.root.join("data/updates"),
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let directory = updates.join(format!(
        "{}-{nonce}-{}",
        release.version,
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let output = powershell()?
        .arg("-Command")
        .arg(include_str!("../../../../scripts/windows/stage-update.ps1"))
        .env("TSAN_UPDATE_URL", &release.url)
        .env("TSAN_UPDATE_SHA256", &release.sha256)
        .env("TSAN_UPDATE_BYTES", release.bytes.to_string())
        .env("TSAN_UPDATE_STAGE", &directory)
        .env("TSAN_UPDATE_VERSION", &release.version)
        .env("TSAN_UPDATE_MODE", distribution.mode.name())
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }
    Ok(directory)
}

pub fn install(update: &StagedUpdate) -> Result<(), String> {
    for (name, script) in [
        (
            "stage-update.ps1",
            include_str!("../../../../scripts/windows/stage-update.ps1"),
        ),
        (
            "update-files.ps1",
            include_str!("../../../../scripts/windows/update-files.ps1"),
        ),
    ] {
        std::fs::write(update.directory.join(name), script).map_err(|e| e.to_string())?;
    }
    powershell()?
        .arg("-Command")
        .arg(include_str!("../../../../scripts/windows/apply-update.ps1"))
        .env("TSAN_UPDATE_MODE", update.distribution.mode.name())
        .env("TSAN_UPDATE_TARGET", &update.distribution.root)
        .env("TSAN_UPDATE_STAGE", &update.directory)
        .env("TSAN_UPDATE_VERSION", &update.release.version)
        .env("TSAN_UPDATE_SHA256", &update.release.sha256)
        .env("TSAN_UPDATE_BYTES", update.release.bytes.to_string())
        .env("TSAN_UPDATE_PARENT", std::process::id().to_string())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
