use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::OperatingSystem;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserDirectories {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub updates: PathBuf,
    pub diagnostics: PathBuf,
}

fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|p| p.is_absolute())
}

// Separate environment resolution from I/O so every platform's fallback is testable.
pub fn user_directories_for(
    os: OperatingSystem,
    environment: impl Fn(&str) -> Option<OsString>,
    home: Option<&Path>,
) -> Result<UserDirectories, String> {
    let home = home.filter(|p| p.is_absolute());
    let home_path = |suffix: &str| home.map(|p| p.join(suffix));
    let (config, cache, updates) = match os {
        OperatingSystem::Windows => {
            let roaming = absolute(environment("APPDATA")).or_else(|| home_path("AppData/Roaming"));
            let local =
                absolute(environment("LOCALAPPDATA")).or_else(|| home_path("AppData/Local"));
            (
                roaming,
                local.as_ref().map(|p| p.join("TS-Analyzer/cache")),
                local.map(|p| p.join("TS-Analyzer/updates")),
            )
        }
        OperatingSystem::Macos => {
            let support = home_path("Library/Application Support");
            (
                support.clone(),
                home_path("Library/Caches/TS-Analyzer"),
                support.map(|p| p.join("TS-Analyzer/updates")),
            )
        }
        OperatingSystem::Linux => {
            let config = absolute(environment("XDG_CONFIG_HOME")).or_else(|| home_path(".config"));
            let cache = absolute(environment("XDG_CACHE_HOME")).or_else(|| home_path(".cache"));
            let state =
                absolute(environment("XDG_STATE_HOME")).or_else(|| home_path(".local/state"));
            (
                config,
                cache.map(|p| p.join("TS-Analyzer")),
                state.map(|p| p.join("TS-Analyzer/updates")),
            )
        }
        OperatingSystem::Unsupported => return Err(os.unavailable("User directories")),
    };
    let diagnostics = match os {
        OperatingSystem::Windows | OperatingSystem::Linux => updates
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.join("diagnostics")),
        OperatingSystem::Macos => home_path("Library/Logs/TS-Analyzer"),
        OperatingSystem::Unsupported => None,
    }
    .ok_or("No valid diagnostics directory or home directory")?;
    Ok(UserDirectories {
        diagnostics,
        config: config
            .ok_or("No valid configuration directory or home directory")?
            .join("TS-Analyzer"),
        cache: cache.ok_or("No valid cache directory or home directory")?,
        updates: updates.ok_or("No valid update directory or home directory")?,
    })
}

pub fn user_directories() -> Result<UserDirectories, String> {
    let home = std::env::home_dir();
    user_directories_for(
        OperatingSystem::current(),
        platform_environment,
        home.as_deref(),
    )
}

fn platform_environment(key: &str) -> Option<OsString> {
    let value = std::env::var_os(key);
    if absolute(value.clone()).is_some() {
        return value;
    }
    #[cfg(windows)]
    if matches!(key, "APPDATA" | "LOCALAPPDATA") {
        return known_folder(key).map(PathBuf::into_os_string);
    }
    None
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn known_folder(key: &str) -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_LocalAppData, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    };
    let id = if key == "APPDATA" {
        &FOLDERID_RoamingAppData
    } else {
        &FOLDERID_LocalAppData
    };
    // SAFETY: id is a valid known-folder GUID; the returned allocation is freed below.
    let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    // SAFETY: The successful API call returns a NUL-terminated UTF-16 allocation.
    let result = unsafe { raw.to_string() }.ok().map(PathBuf::from);
    // SAFETY: SHGetKnownFolderPath allocates using the COM task allocator.
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    result.filter(|p| p.is_absolute())
}

pub fn config_file() -> Result<PathBuf, String> {
    if let Some(path) = absolute(std::env::var_os("TSAN_CONFIG_PATH")) {
        return Ok(path);
    }
    Ok(user_directories()?.config.join("tsan-config.toml"))
}

pub fn cache_directory() -> Result<PathBuf, String> {
    Ok(user_directories()?.cache)
}

pub fn update_directory() -> Result<PathBuf, String> {
    Ok(user_directories()?.updates)
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn windows_directory() -> Result<PathBuf, String> {
    use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
    if let Some(root) =
        absolute(std::env::var_os("SystemRoot")).filter(|p| p.join("System32").is_dir())
    {
        return Ok(root);
    }
    let mut buffer = vec![0u16; 32768];
    // SAFETY: The API receives a writable slice and its actual capacity.
    let length = unsafe { GetWindowsDirectoryW(Some(&mut buffer)) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("Windows could not report its system directory".into());
    }
    use std::os::windows::ffi::OsStringExt;
    let root = PathBuf::from(OsString::from_wide(&buffer[..length]));
    if !root.is_absolute() || !root.join("System32").is_dir() {
        return Err("Windows returned an invalid system directory".into());
    }
    Ok(root)
}

#[cfg(not(windows))]
pub fn windows_directory() -> Result<PathBuf, String> {
    Err("A Windows system directory is only available on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_relative_environment_values_use_each_os_home_layout() -> Result<(), String> {
        let home = std::env::temp_dir().join("tsan-home-fixture");
        for (os, config, cache, updates, diagnostics) in [
            (
                OperatingSystem::Windows,
                "AppData/Roaming/TS-Analyzer",
                "AppData/Local/TS-Analyzer/cache",
                "AppData/Local/TS-Analyzer/updates",
                "AppData/Local/TS-Analyzer/diagnostics",
            ),
            (
                OperatingSystem::Macos,
                "Library/Application Support/TS-Analyzer",
                "Library/Caches/TS-Analyzer",
                "Library/Application Support/TS-Analyzer/updates",
                "Library/Logs/TS-Analyzer",
            ),
            (
                OperatingSystem::Linux,
                ".config/TS-Analyzer",
                ".cache/TS-Analyzer",
                ".local/state/TS-Analyzer/updates",
                ".local/state/TS-Analyzer/diagnostics",
            ),
        ] {
            for value in [
                None,
                Some(OsString::from("")),
                Some(OsString::from("relative")),
            ] {
                let found = user_directories_for(os, |_| value.clone(), Some(&home))?;
                assert_eq!(found.config, home.join(config));
                assert_eq!(found.cache, home.join(cache));
                assert_eq!(found.updates, home.join(updates));
                assert_eq!(found.diagnostics, home.join(diagnostics));
            }
        }
        assert!(user_directories_for(OperatingSystem::Linux, |_| None, None).is_err());
        assert!(user_directories_for(OperatingSystem::Unsupported, |_| None, Some(&home)).is_err());
        Ok(())
    }

    #[test]
    fn valid_platform_overrides_are_preserved() -> Result<(), String> {
        let base = std::env::temp_dir().join("tsan-overrides");
        let env = |key: &str| Some(base.join(key).into_os_string());
        let windows = user_directories_for(OperatingSystem::Windows, env, None)?;
        assert_eq!(windows.config, base.join("APPDATA/TS-Analyzer"));
        assert_eq!(
            windows.diagnostics,
            base.join("LOCALAPPDATA/TS-Analyzer/diagnostics")
        );
        assert_eq!(
            windows.updates,
            base.join("LOCALAPPDATA/TS-Analyzer/updates")
        );
        let linux = user_directories_for(OperatingSystem::Linux, env, None)?;
        assert_eq!(linux.config, base.join("XDG_CONFIG_HOME/TS-Analyzer"));
        assert_eq!(linux.cache, base.join("XDG_CACHE_HOME/TS-Analyzer"));
        assert_eq!(
            linux.diagnostics,
            base.join("XDG_STATE_HOME/TS-Analyzer/diagnostics")
        );
        assert_eq!(
            linux.updates,
            base.join("XDG_STATE_HOME/TS-Analyzer/updates")
        );
        Ok(())
    }
}
