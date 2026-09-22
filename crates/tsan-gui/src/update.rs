use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformUpdateBackend {
    pub platform: &'static str,
    pub target_os: &'static str,
    pub target_arch: &'static str,
    pub artifact_policy: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateStatus {
    NotConfigured {
        current_version: &'static str,
        backend: PlatformUpdateBackend,
    },
}

impl UpdateStatus {
    pub fn message(&self) -> String {
        match self {
            Self::NotConfigured {
                current_version,
                backend,
            } => format!(
                "Update framework ready for {} {}/{} at version {}. No signed release manifest is configured yet; the manifest will select the matching artifact and installation action.",
                backend.platform, backend.target_os, backend.target_arch, current_version
            ),
        }
    }
}

pub fn platform_backend() -> PlatformUpdateBackend {
    platform::backend()
}

pub fn check_for_updates() -> UpdateStatus {
    // TODO(release): Configure a signed update manifest URL, public verification key,
    // release channels, artifact hashes, target selectors, installation actions and
    // rollback metadata before enabling downloads.
    UpdateStatus::NotConfigured {
        current_version: env!("CARGO_PKG_VERSION"),
        backend: platform_backend(),
    }
}

#[allow(dead_code)]
pub fn install_staged_update(package: &Path) -> Result<(), String> {
    platform::install_staged_update(package)
}

#[cfg(target_os = "windows")]
mod platform {
    use super::PlatformUpdateBackend;
    use std::path::Path;

    pub fn backend() -> PlatformUpdateBackend {
        PlatformUpdateBackend {
            platform: "Windows",
            target_os: std::env::consts::OS,
            target_arch: std::env::consts::ARCH,
            artifact_policy: "Signed manifest selects an artifact and Windows installation action.",
        }
    }

    pub fn install_staged_update(package: &Path) -> Result<(), String> {
        Err(format!(
            "The signed manifest has not provided a Windows installation action for {}.",
            package.display()
        ))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::PlatformUpdateBackend;
    use std::path::Path;

    pub fn backend() -> PlatformUpdateBackend {
        PlatformUpdateBackend {
            platform: "macOS",
            target_os: std::env::consts::OS,
            target_arch: std::env::consts::ARCH,
            artifact_policy: "Signed manifest selects an artifact and macOS installation action.",
        }
    }

    pub fn install_staged_update(package: &Path) -> Result<(), String> {
        Err(format!(
            "The signed manifest has not provided a macOS installation action for {}.",
            package.display()
        ))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::PlatformUpdateBackend;
    use std::path::Path;

    pub fn backend() -> PlatformUpdateBackend {
        PlatformUpdateBackend {
            platform: "Linux",
            target_os: std::env::consts::OS,
            target_arch: std::env::consts::ARCH,
            artifact_policy: "Signed manifest selects an artifact and Linux installation action.",
        }
    }

    pub fn install_staged_update(package: &Path) -> Result<(), String> {
        Err(format!(
            "The signed manifest has not provided a Linux installation action for {}.",
            package.display()
        ))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod platform {
    use super::PlatformUpdateBackend;
    use std::path::Path;

    pub fn backend() -> PlatformUpdateBackend {
        PlatformUpdateBackend {
            platform: "Unsupported platform",
            target_os: std::env::consts::OS,
            target_arch: std::env::consts::ARCH,
            artifact_policy: "No installation action is available for this target.",
        }
    }

    pub fn install_staged_update(package: &Path) -> Result<(), String> {
        Err(format!(
            "No updater backend is available for {}.",
            package.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{UpdateStatus, check_for_updates, platform_backend};

    #[test]
    fn framework_reports_the_compiled_target_without_fixing_a_package_format() {
        let backend = platform_backend();
        assert!(!backend.platform.is_empty());
        assert!(!backend.target_os.is_empty());
        assert!(!backend.target_arch.is_empty());
        assert!(backend.artifact_policy.contains("manifest"));
        assert!(matches!(
            check_for_updates(),
            UpdateStatus::NotConfigured { .. }
        ));
    }
}
