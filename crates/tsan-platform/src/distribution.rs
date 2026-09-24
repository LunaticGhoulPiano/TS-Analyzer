use std::fs;
use std::path::{Path, PathBuf};

pub const APP_ID: &str = "TSAnalyzer";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Installed,
    Portable,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Portable => "portable",
        }
    }

    pub fn asset(self, os: &str, arch: &str, version: &str) -> Result<String, String> {
        let platform = crate::OperatingSystem::from_target(os);
        if platform != crate::OperatingSystem::Windows {
            return Err(platform.unavailable("Package artifacts"));
        }
        if arch != "x86_64" {
            return Err(format!(
                "Windows package artifacts are not implemented for {arch} yet."
            ));
        }
        let suffix = match self {
            Self::Installed => "setup.exe",
            Self::Portable => "portable.zip",
        };
        Ok(format!("TS-Analyzer-{os}-v{version}-{arch}-{suffix}"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Distribution {
    pub root: PathBuf,
    pub mode: Mode,
}

fn marker_mode(text: &str) -> Result<Mode, String> {
    let mut fields = std::collections::BTreeMap::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or("Invalid deployment marker")?;
        if fields.insert(key.trim(), value.trim()).is_some() {
            return Err("Duplicate deployment field".into());
        }
    }
    if fields.len() != 3
        || fields.get("schema") != Some(&"1")
        || fields.get("app_id").copied() != Some(format!("\"{APP_ID}\"").as_str())
    {
        return Err("Unrecognized application deployment".into());
    }
    match fields.get("mode") {
        Some(&"\"installed\"") => Ok(Mode::Installed),
        Some(&"\"portable\"") => Ok(Mode::Portable),
        _ => Err("Unknown deployment mode".into()),
    }
}

pub fn at_root(root: &Path) -> Result<Distribution, String> {
    if crate::OperatingSystem::current() != crate::OperatingSystem::Windows {
        return Err(crate::OperatingSystem::current().unavailable("Package discovery"));
    }
    let text = fs::read_to_string(root.join("deployment.toml")).map_err(|e| e.to_string())?;
    let mode = marker_mode(&text)?;
    if !root.join("package.toml").is_file()
        || !root.join("TS-Analyzer.exe").is_file()
        || !root.join("app/tsan-gui.exe").is_file()
    {
        return Err("Incomplete application package".into());
    }
    Ok(Distribution {
        root: root.to_owned(),
        mode,
    })
}

// Detect from this executable, never from the working directory or an inherited variable.
pub fn for_gui(exe: &Path) -> Result<Option<Distribution>, String> {
    if crate::OperatingSystem::current() != crate::OperatingSystem::Windows {
        return Err(crate::OperatingSystem::current().unavailable("Package discovery"));
    }
    let Some(app) = exe
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "app"))
    else {
        return Ok(None);
    };
    let root = app.parent().ok_or("Package root is missing")?;
    if !root.join("deployment.toml").exists() {
        return Ok(None);
    }
    at_root(root).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_and_portable_have_distinct_assets() -> Result<(), String> {
        assert_eq!(
            Mode::Installed.asset("windows", "x86_64", "0.2.1")?,
            "TS-Analyzer-windows-v0.2.1-x86_64-setup.exe"
        );
        assert!(
            Mode::Portable
                .asset("windows", "x86_64", "0.2.1")?
                .ends_with("-portable.zip")
        );
        for os in ["macos", "linux", "unknown"] {
            for mode in [Mode::Installed, Mode::Portable] {
                assert_eq!(
                    mode.asset(os, "x86_64", "0.2.1"),
                    Err(crate::OperatingSystem::from_target(os).unavailable("Package artifacts"))
                );
            }
        }
        assert!(
            Mode::Installed
                .asset("windows", "aarch64", "0.2.1")
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn deployment_marker_is_explicit_and_strict() -> Result<(), String> {
        for mode in [Mode::Portable, Mode::Installed] {
            let text = format!(
                "schema = 1\napp_id = \"{APP_ID}\"\nmode = \"{}\"\n",
                mode.name()
            );
            assert_eq!(marker_mode(&text), Ok(mode));
            assert!(marker_mode(&text.replace(APP_ID, "OtherApp")).is_err());
            assert!(marker_mode(&(text + "mode = \"portable\"\n")).is_err());
        }
        let source = for_gui(Path::new("target/debug/tsan-gui.exe"));
        if cfg!(windows) {
            assert!(source?.is_none());
        } else {
            assert!(source.is_err());
        }
        assert!(marker_mode("mode = \"portable\"").is_err());
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn relocated_package_preserves_its_mode() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("tsan-layout-{}", std::process::id()));
        fs::create_dir_all(root.join("app"))?;
        for file in ["TS-Analyzer.exe", "package.toml", "app/tsan-gui.exe"] {
            fs::write(root.join(file), "")?;
        }
        fs::write(
            root.join("deployment.toml"),
            "schema = 1\napp_id = \"TSAnalyzer\"\nmode = \"portable\"\n",
        )?;
        let found = for_gui(&root.join("app/tsan-gui.exe"))?.ok_or("Package was not detected")?;
        assert_eq!(found.root, root);
        assert_eq!(found.mode, Mode::Portable);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
