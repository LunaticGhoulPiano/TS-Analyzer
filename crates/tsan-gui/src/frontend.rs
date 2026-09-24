use tsan_platform::OperatingSystem;
use tsan_runtime::AnalysisService;

mod linux;
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub trait Frontend {
    fn run(analysis: AnalysisService) -> Result<(), String>;
}

pub fn run(analysis: AnalysisService) -> Result<(), String> {
    run_for(OperatingSystem::current(), analysis)
}

fn run_for(os: OperatingSystem, analysis: AnalysisService) -> Result<(), String> {
    match os {
        OperatingSystem::Windows => {
            #[cfg(target_os = "windows")]
            {
                windows::WindowsFrontend::run(analysis)
            }
            #[cfg(not(target_os = "windows"))]
            {
                Err("The Windows frontend requires a Windows build".into())
            }
        }
        OperatingSystem::Macos => macos::MacosFrontend::run(analysis),
        OperatingSystem::Linux => linux::LinuxFrontend::run(analysis),
        OperatingSystem::Unsupported => Err(os.unavailable("GUI")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_frontends_report_their_own_platform() {
        for os in [
            OperatingSystem::Macos,
            OperatingSystem::Linux,
            OperatingSystem::Unsupported,
        ] {
            assert_eq!(run_for(os, AnalysisService), Err(os.unavailable("GUI")));
        }
    }
}
