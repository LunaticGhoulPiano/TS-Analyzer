use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogCategory {
    System,
    Input,
    Playback,
    Pipeline,
    Analysis,
    Configuration,
    ImportExport,
}

impl LogCategory {
    pub const fn name(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Input => "Input",
            Self::Playback => "Playback",
            Self::Pipeline => "Pipeline",
            Self::Analysis => "Analysis",
            Self::Configuration => "Configuration",
            Self::ImportExport => "Import/Export",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

impl LogLevel {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub sequence: u64,
    pub unix_milliseconds: u128,
    pub level: LogLevel,
    pub category: LogCategory,
    pub message: String,
}

impl LogEntry {
    pub fn new(
        sequence: u64,
        level: LogLevel,
        category: LogCategory,
        message: impl Into<String>,
    ) -> Self {
        let unix_milliseconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();

        Self {
            sequence,
            unix_milliseconds,
            level,
            category,
            message: message.into(),
        }
    }

    pub fn dump_line(&self) -> String {
        format!(
            "{} [{}] [{}] {}",
            self.unix_milliseconds,
            self.level.name(),
            self.category.name(),
            self.message
        )
    }
}
