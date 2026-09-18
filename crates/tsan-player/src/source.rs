use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpProtocol {
    Udp,
    Rtp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlayerSource {
    File(PathBuf),
    Ip {
        protocol: IpProtocol,
        listen_address: SocketAddr,
    },
}

impl PlayerSource {
    pub const fn is_live(&self) -> bool {
        matches!(self, Self::Ip { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TsRecordingOptions {
    pub output_path: PathBuf,
    pub max_duration: Option<Duration>,
}

impl TsRecordingOptions {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.output_path.as_os_str().is_empty() {
            return Err("recording output path is empty");
        }
        if self.max_duration.is_some_and(|duration| duration.is_zero()) {
            return Err("maximum recording duration must be positive");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerSession {
    pub source: PlayerSource,
    pub recording: Option<TsRecordingOptions>,
}

impl PlayerSession {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(recording) = &self.recording {
            if !self.source.is_live() {
                return Err("TS recording is available only for live IP sources");
            }
            recording.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_requires_live_source_and_valid_output() {
        let recording = TsRecordingOptions {
            output_path: PathBuf::from("capture.ts"),
            max_duration: Some(Duration::from_secs(30)),
        };
        let file = PlayerSession {
            source: PlayerSource::File(PathBuf::from("input.ts")),
            recording: Some(recording.clone()),
        };
        assert!(file.validate().is_err());

        let live = PlayerSession {
            source: PlayerSource::Ip {
                protocol: IpProtocol::Udp,
                listen_address: SocketAddr::from(([0, 0, 0, 0], 1234)),
            },
            recording: Some(recording),
        };
        assert!(live.validate().is_ok());
    }
}
