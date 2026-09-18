use tsan_core::probe_transport_stream;
pub use tsan_core::{TransportStreamProbe, TransportStreamProbeError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputSourceKind {
    TransportStreamFile,
    IpStreaming,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileSourceProbe {
    kind: InputSourceKind,
    transport_stream: TransportStreamProbe,
}

impl FileSourceProbe {
    pub const fn kind(self) -> InputSourceKind {
        self.kind
    }

    pub const fn transport_stream(self) -> TransportStreamProbe {
        self.transport_stream
    }
}

pub fn probe_file_source(sample: &[u8]) -> Result<FileSourceProbe, TransportStreamProbeError> {
    let transport_stream = probe_transport_stream(sample)?;
    Ok(FileSourceProbe {
        kind: InputSourceKind::TransportStreamFile,
        transport_stream,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_is_not_a_transport_stream_file() {
        let sample = b"export type Packet = { syncByte: number };\n".repeat(64);

        assert!(probe_file_source(&sample).is_err());
    }
}
