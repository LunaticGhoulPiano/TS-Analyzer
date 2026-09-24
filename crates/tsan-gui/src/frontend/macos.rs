use super::{AnalysisService, Frontend, OperatingSystem};

pub struct MacosFrontend;

impl Frontend for MacosFrontend {
    fn run(_analysis: AnalysisService) -> Result<(), String> {
        Err(OperatingSystem::Macos.unavailable("GUI"))
    }
}
