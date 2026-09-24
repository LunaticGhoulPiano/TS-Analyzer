use super::{Distribution, PathBuf, Release, StagedUpdate};
use tsan_platform::OperatingSystem;

pub fn fetch_release(_: &Distribution) -> Result<String, String> {
    Err(OperatingSystem::Linux.unavailable("Release checking"))
}

pub fn stage(_: &Release, _: &Distribution) -> Result<PathBuf, String> {
    Err(OperatingSystem::Linux.unavailable("Update staging"))
}

pub fn install(_: &StagedUpdate) -> Result<(), String> {
    Err(OperatingSystem::Linux.unavailable("Update installation"))
}
