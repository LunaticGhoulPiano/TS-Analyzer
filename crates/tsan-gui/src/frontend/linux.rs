use super::{AnalysisService, Frontend, OperatingSystem};

pub struct LinuxFrontend;

impl Frontend for LinuxFrontend {
    fn run(_analysis: AnalysisService) -> Result<(), String> {
        Err(OperatingSystem::Linux.unavailable("GUI"))
    }
}
