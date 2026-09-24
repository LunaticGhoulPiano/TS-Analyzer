pub mod distribution;
pub mod paths;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatingSystem {
    Windows,
    Macos,
    Linux,
    Unsupported,
}

impl OperatingSystem {
    pub fn current() -> Self {
        Self::from_target(std::env::consts::OS)
    }

    pub fn from_target(target: &str) -> Self {
        match target {
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            "linux" => Self::Linux,
            _ => Self::Unsupported,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Macos => "macOS",
            Self::Linux => "Linux",
            Self::Unsupported => "this operating system",
        }
    }

    pub fn unavailable(self, feature: &str) -> String {
        format!("{feature} is not implemented for {} yet.", self.name())
    }
}
