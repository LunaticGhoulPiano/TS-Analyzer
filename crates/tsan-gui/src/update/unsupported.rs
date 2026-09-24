use super::{Distribution, PathBuf, Release, StagedUpdate};
use tsan_platform::OperatingSystem;

pub fn fetch_release(_: &Distribution) -> Result<String, String> {
    Err(OperatingSystem::Unsupported.unavailable("Release checking"))
}

pub fn stage(_: &Release, _: &Distribution) -> Result<PathBuf, String> {
    Err(OperatingSystem::Unsupported.unavailable("Update staging"))
}

pub fn install(_: &StagedUpdate) -> Result<(), String> {
    Err(OperatingSystem::Unsupported.unavailable("Update installation"))
}
