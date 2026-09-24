use std::ffi::OsString;
use std::path::Path;

pub struct Guard;
pub fn install(_session: &Path) -> Result<Guard, String> {
    Err(tsan_platform::OperatingSystem::current().unavailable("Native crash dumps"))
}
pub fn run_helper(_args: &[OsString]) -> Result<(), String> {
    Err(tsan_platform::OperatingSystem::current().unavailable("Native crash dumps"))
}
