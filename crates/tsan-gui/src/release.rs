pub const VERSION: &str = env!("TSAN_PLATFORM_VERSION");
pub const STATUS: &str = env!("TSAN_PLATFORM_STATUS");
pub const MINIMUM_OS: &str = env!("TSAN_PLATFORM_MINIMUM_OS");

pub fn tag(os: &str, version: &str) -> String {
    format!("{os}-v{version}")
}

pub fn asset(os: &str, arch: &str, version: &str) -> String {
    format!("TS-Analyzer-{os}-v{version}-{arch}-portable.zip")
}

#[cfg(test)]
mod tests {
    #[test]
    fn platforms_have_separate_release_names() {
        assert_eq!(super::tag("windows", "0.2.0"), "windows-v0.2.0");
        assert_ne!(
            super::asset("windows", "x86_64", "0.2.0"),
            super::asset("linux", "x86_64", "0.2.0")
        );
        assert_ne!(super::tag("macos", "0.1.0"), super::tag("linux", "0.1.0"));
    }
}
