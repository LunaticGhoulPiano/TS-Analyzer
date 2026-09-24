use crate::distribution::{Distribution, Mode};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformUpdateBackend {
    pub platform: &'static str,
    pub target_os: &'static str,
    pub target_arch: &'static str,
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

#[derive(Clone, Debug)]
pub struct StagedUpdate {
    release: Release,
    distribution: Distribution,
    directory: PathBuf,
}

fn current_distribution() -> Result<Option<Distribution>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    crate::distribution::for_gui(&exe)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateStatus {
    DevelopmentBuild,
    UpToDate,
    Available(Release),
    NoRelease,
    NoCompatiblePackage(String),
    Failed(String),
}

impl UpdateStatus {
    pub fn message(&self) -> String {
        match self {
            Self::DevelopmentBuild => "This executable has no deployment marker. Update source builds through Git, or download a complete release package.".into(),
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
        platform: tsan_platform::OperatingSystem::current().name(),
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
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

fn asset_name(version: &str, mode: Mode) -> Result<String, String> {
    mode.asset(std::env::consts::OS, std::env::consts::ARCH, version)
}

fn validate_release(release: &Release, mode: Mode) -> Result<(), String> {
    if version(&release.version).is_none() || release.version.starts_with('v') {
        return Err("Invalid release version".into());
    }
    let asset = asset_name(&release.version, mode)?;
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

fn parse_response(text: &str, mode: Mode) -> Result<UpdateStatus, String> {
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
    validate_release(&release, mode)?;
    Ok(UpdateStatus::Available(release))
}

pub fn check_for_updates() -> UpdateStatus {
    let distribution = match current_distribution() {
        Ok(Some(deployment)) => deployment,
        Ok(None) => return UpdateStatus::DevelopmentBuild,
        Err(error) => return UpdateStatus::Failed(error),
    };
    match platform::fetch_release(&distribution).and_then(|s| parse_response(&s, distribution.mode))
    {
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

pub fn start_download(release: Release) -> Receiver<Result<StagedUpdate, String>> {
    let (send, receive) = mpsc::channel();
    let fallback = send.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("tsan-update-download".into())
        .spawn(move || {
            let result = current_distribution()
                .and_then(|d| d.ok_or("Source builds cannot install updates".into()))
                .and_then(|distribution| {
                    validate_release(&release, distribution.mode)?;
                    platform::stage(&release, &distribution).map(|directory| StagedUpdate {
                        release,
                        distribution,
                        directory,
                    })
                });
            let _ = send.send(result);
        })
    {
        let _ = fallback.send(Err(e.to_string()));
    }
    receive
}

pub fn install_staged_update(package: &StagedUpdate) -> Result<(), String> {
    if current_distribution()?.as_ref() != Some(&package.distribution) {
        return Err("Application deployment changed; check for updates again.".into());
    }
    validate_release(&package.release, package.distribution.mode)?;
    platform::install(package)
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod unsupported;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
use unsupported as platform;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_update_backends_fail_without_running_windows_tools() {
        let distribution = Distribution {
            root: std::env::temp_dir().join("tsan-unimplemented-update"),
            mode: Mode::Installed,
        };
        let release = Release {
            version: "0.0.0".into(),
            page: String::new(),
            asset: String::new(),
            url: String::new(),
            sha256: String::new(),
            bytes: 0,
        };
        let staged = StagedUpdate {
            release,
            distribution,
            directory: std::env::temp_dir(),
        };
        for (os, check, stage, install) in [
            (
                tsan_platform::OperatingSystem::Macos,
                macos::fetch_release(&staged.distribution),
                macos::stage(&staged.release, &staged.distribution),
                macos::install(&staged),
            ),
            (
                tsan_platform::OperatingSystem::Linux,
                linux::fetch_release(&staged.distribution),
                linux::stage(&staged.release, &staged.distribution),
                linux::install(&staged),
            ),
        ] {
            assert_eq!(check, Err(os.unavailable("Release checking")));
            assert_eq!(stage, Err(os.unavailable("Update staging")));
            assert_eq!(install, Err(os.unavailable("Update installation")));
        }
    }
    #[cfg(windows)]
    #[test]
    fn release_script_receives_literal_path_and_separate_arguments()
    -> Result<(), Box<dyn std::error::Error>> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "tsan release ' $; [args] {}-{nonce}",
            std::process::id()
        ));
        let script = root.join("scripts/windows/fetch-release.ps1");
        std::fs::create_dir_all(script.parent().ok_or("Missing script parent")?)?;
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let mut distribution = Distribution {
                root: root.clone(),
                mode: Mode::Portable,
            };
            assert!(platform::fetch_release(&distribution).is_err());
            std::fs::write(
                &script,
                "param($TargetOs, $TargetArch, $Mode)\n\"$TargetOs/$TargetArch/$Mode\"\n",
            )?;
            for mode in [Mode::Portable, Mode::Installed] {
                distribution.mode = mode;
                assert_eq!(
                    platform::fetch_release(&distribution)?,
                    format!(
                        "{}/{}/{}",
                        std::env::consts::OS,
                        std::env::consts::ARCH,
                        mode.name()
                    )
                );
            }
            std::fs::write(&script, "throw 'release-script-failure'\n")?;
            assert!(
                platform::fetch_release(&distribution)
                    .is_err_and(|error| error.contains("release-script-failure"))
            );
            Ok(())
        })();
        std::fs::remove_dir_all(root)?;
        result
    }

    #[test]
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    fn rejects_cross_target_and_untrusted_release_metadata() -> Result<(), String> {
        let version = "99.0.0";
        let mode = Mode::Portable;
        let asset = asset_name(version, mode)?;
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
        validate_release(&release, mode)?;
        assert!(validate_release(&release, Mode::Installed).is_err());
        release.url = "https://example.com/update.zip".into();
        assert!(validate_release(&release, mode).is_err());
        release.url = format!("{base}/download/{tag}/{asset}");
        release.asset = "other-platform.zip".into();
        assert!(validate_release(&release, mode).is_err());
        assert_eq!(parse_response("NO_RELEASE", mode)?, UpdateStatus::NoRelease);
        assert!(matches!(
            parse_response(&format!("{tag}\nNO_PACKAGE"), mode)?,
            UpdateStatus::NoCompatiblePackage(_)
        ));
        assert!(parse_response("v99.0.0\n", mode).is_err());
        assert!(parse_response("otheros-v99.0.0\nNO_PACKAGE", mode).is_err());
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
