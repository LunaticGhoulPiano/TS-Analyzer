use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformUpdateBackend {
    pub platform: &'static str,
    pub target_os: &'static str,
    pub target_arch: &'static str,
    pub artifact_policy: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    pub version: String,
    pub page: String,
    pub asset: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateStatus {
    UpToDate,
    Available(Release),
    NoRelease,
    NoCompatiblePackage(String),
    Failed(String),
}

impl UpdateStatus {
    pub fn message(&self) -> String {
        match self {
            Self::UpToDate => format!(
                "{} {} is current.",
                std::env::consts::OS,
                crate::release::VERSION
            ),
            Self::Available(r) => format!(
                "Version {} is available ({:.1} MiB). Includes the app and its private runtimes.",
                r.version,
                r.bytes as f64 / 1_048_576.0
            ),
            Self::NoRelease => format!(
                "No public stable {} release has been published.",
                std::env::consts::OS
            ),
            Self::NoCompatiblePackage(v) => format!(
                "Release {v} has no verified package for this platform. Nothing was downloaded."
            ),
            Self::Failed(e) => format!("Update check failed: {e}"),
        }
    }
}

pub fn platform_backend() -> PlatformUpdateBackend {
    PlatformUpdateBackend {
        platform: if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        },
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
        artifact_policy: "Checks stable GitHub Releases. Packages must match the OS/architecture and GitHub SHA-256 digest.",
    }
}

fn version(value: &str) -> Option<[u32; 3]> {
    let parts = value
        .strip_prefix('v')
        .unwrap_or(value)
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    parts.try_into().ok()
}

fn asset_name(version: &str) -> String {
    crate::release::asset(std::env::consts::OS, std::env::consts::ARCH, version)
}

fn validate_release(release: &Release) -> Result<(), String> {
    if version(&release.version).is_none() || release.version.starts_with('v') {
        return Err("Invalid release version".into());
    }
    let asset = asset_name(&release.version);
    let tag = crate::release::tag(std::env::consts::OS, &release.version);
    let base = "https://github.com/LunaticGhoulPiano/TS-Analyzer/releases";
    if release.asset != asset
        || release.url != format!("{base}/download/{tag}/{asset}")
        || release.page != format!("{base}/tag/{tag}")
        || release.sha256.len() != 64
        || !release.sha256.bytes().all(|c| c.is_ascii_hexdigit())
        || release.bytes == 0
        || release.bytes > 4 * 1024 * 1024 * 1024
    {
        return Err("Release artifact metadata failed validation".into());
    }
    Ok(())
}

fn parse_response(text: &str) -> Result<UpdateStatus, String> {
    let lines = text.lines().map(str::trim).collect::<Vec<_>>();
    if lines.first() == Some(&"NO_RELEASE") {
        return Ok(UpdateStatus::NoRelease);
    }
    let tag = lines.first().ok_or("Empty release response")?;
    let platform_prefix = format!("{}-v", std::env::consts::OS);
    let number = tag
        .strip_prefix(&platform_prefix)
        .ok_or("Release belongs to a different platform")?;
    let remote = version(number).ok_or("Invalid release tag")?;
    let current = version(crate::release::VERSION).ok_or("Invalid application version")?;
    if remote <= current {
        return Ok(UpdateStatus::UpToDate);
    }
    if lines.len() == 2 && lines[1] == "NO_PACKAGE" {
        return Ok(UpdateStatus::NoCompatiblePackage((*tag).into()));
    }
    if lines.len() != 6 {
        return Err("Incomplete release response".into());
    }
    let release = Release {
        version: number.into(),
        page: lines[1].into(),
        asset: lines[2].into(),
        url: lines[3].into(),
        sha256: lines[4]
            .strip_prefix("sha256:")
            .ok_or("Missing GitHub SHA-256 digest")?
            .into(),
        bytes: lines[5].parse().map_err(|_| "Invalid package size")?,
    };
    validate_release(&release)?;
    Ok(UpdateStatus::Available(release))
}

pub fn check_for_updates() -> UpdateStatus {
    match platform::fetch_release().and_then(|s| parse_response(&s)) {
        Ok(status) => status,
        Err(e) => UpdateStatus::Failed(e),
    }
}

pub fn start_check() -> Receiver<UpdateStatus> {
    let (send, receive) = mpsc::channel();
    let fallback = send.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("tsan-update-check".into())
        .spawn(move || {
            let _ = send.send(check_for_updates());
        })
    {
        let _ = fallback.send(UpdateStatus::Failed(e.to_string()));
    }
    receive
}

pub fn start_download(release: Release) -> Receiver<Result<PathBuf, String>> {
    let (send, receive) = mpsc::channel();
    let fallback = send.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("tsan-update-download".into())
        .spawn(move || {
            let _ = send.send(validate_release(&release).and_then(|_| platform::stage(&release)));
        })
    {
        let _ = fallback.send(Err(e.to_string()));
    }
    receive
}

pub fn install_staged_update(package: &Path) -> Result<(), String> {
    platform::install(package)
}

#[cfg(windows)]
mod platform {
    use super::*;
    fn powershell() -> std::process::Command {
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let mut command = crate::os_integration::background_command(
            root.join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        );
        command.args(["-NoProfile", "-NonInteractive", "-Command"]);
        command.env(
            "PSModulePath",
            root.join("System32/WindowsPowerShell/v1.0/Modules"),
        );
        command
    }
    pub fn fetch_release() -> Result<String, String> {
        let script = r#"
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$candidates = @()
$pattern = '^' + [regex]::Escape($env:TSAN_UPDATE_OS) + '-v(\d+\.\d+\.\d+)$'
for ($page = 1; $page -le 20; $page++) {
  try {
    $items = @(Invoke-RestMethod -Uri ("https://api.github.com/repos/LunaticGhoulPiano/TS-Analyzer/releases?per_page=100&page=$page") -Headers @{'User-Agent'='TS-Analyzer';'Accept'='application/vnd.github+json'} -TimeoutSec 20)
  } catch {
    if ($_.Exception.Response.StatusCode.value__ -eq 404) { 'NO_RELEASE'; exit 0 }
    throw
  }
  $candidates += @($items | Where-Object { -not $_.draft -and -not $_.prerelease -and $_.tag_name -match $pattern })
  if ($items.Count -lt 100) { break }
  if ($page -eq 20) { throw 'Release history exceeds the check limit; cannot establish the newest platform version.' }
}
$r = $candidates | Sort-Object { [version]($_.tag_name -replace $pattern,'$1') } -Descending | Select-Object -First 1
if (-not $r) { 'NO_RELEASE'; exit 0 }
$r.tag_name
$name = 'TS-Analyzer-' + $r.tag_name + '-' + $env:TSAN_UPDATE_ARCH + '-portable.zip'
$a = @($r.assets | Where-Object { $_.name -eq $name })
if ($a.Count -ne 1 -or -not $a[0].digest) { 'NO_PACKAGE'; exit 0 }
$r.html_url
$a[0].name
$a[0].browser_download_url
$a[0].digest
$a[0].size
"#;
        let output = powershell()
            .arg(script)
            .env("TSAN_UPDATE_OS", std::env::consts::OS)
            .env("TSAN_UPDATE_ARCH", std::env::consts::ARCH)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    }
    pub fn stage(release: &Release) -> Result<PathBuf, String> {
        let root = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis();
        let directory = PathBuf::from(root)
            .join("TS-Analyzer/updates")
            .join(format!("{}-{nonce}", release.version));
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let output = powershell()
            .arg(include_str!("../../../packaging/windows/stage-update.ps1"))
            .env("TSAN_UPDATE_URL", &release.url)
            .env("TSAN_UPDATE_SHA256", &release.sha256)
            .env("TSAN_UPDATE_BYTES", release.bytes.to_string())
            .env("TSAN_UPDATE_STAGE", &directory)
            .env("TSAN_UPDATE_VERSION", &release.version)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().into());
        }
        Ok(directory.join("package/Setup.exe"))
    }
    pub fn install(package: &Path) -> Result<(), String> {
        if package.file_name().and_then(|v| v.to_str()) != Some("Setup.exe") || !package.is_file() {
            return Err("The staged installer is missing".into());
        }
        crate::os_integration::background_command(package)
            .arg("--install")
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn fetch_release() -> Result<String, String> {
        Err("This platform's release backend has not been implemented or verified.".into())
    }
    pub fn stage(_: &Release) -> Result<PathBuf, String> {
        Err("No verified installer backend exists for this platform.".into())
    }
    pub fn install(_: &Path) -> Result<(), String> {
        Err("No verified installer backend exists for this platform.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_cross_target_and_untrusted_release_metadata() -> Result<(), String> {
        let version = "99.0.0";
        let asset = asset_name(version);
        let tag = crate::release::tag(std::env::consts::OS, version);
        let base = "https://github.com/LunaticGhoulPiano/TS-Analyzer/releases";
        let mut release = Release {
            version: version.into(),
            asset: asset.clone(),
            page: format!("{base}/tag/{tag}"),
            url: format!("{base}/download/{tag}/{asset}"),
            sha256: "a".repeat(64),
            bytes: 123,
        };
        validate_release(&release)?;
        release.url = "https://example.com/update.zip".into();
        assert!(validate_release(&release).is_err());
        release.url = format!("{base}/download/{tag}/{asset}");
        release.asset = "other-platform.zip".into();
        assert!(validate_release(&release).is_err());
        assert_eq!(parse_response("NO_RELEASE")?, UpdateStatus::NoRelease);
        assert!(matches!(
            parse_response(&format!("{tag}\nNO_PACKAGE"))?,
            UpdateStatus::NoCompatiblePackage(_)
        ));
        assert!(parse_response("v99.0.0\n").is_err());
        assert!(parse_response("otheros-v99.0.0\nNO_PACKAGE").is_err());
        Ok(())
    }
    #[test]
    #[ignore = "contacts the public GitHub release API"]
    fn github_release_check() {
        let status = check_for_updates();
        println!("{}", status.message());
        assert!(!matches!(status, UpdateStatus::Failed(_)));
    }
}
