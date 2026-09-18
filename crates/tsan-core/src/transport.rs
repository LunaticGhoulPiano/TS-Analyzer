use std::fmt;

const MINIMUM_PROBE_PACKETS: usize = 5;
const MAXIMUM_LEADING_BYTES: usize = 408;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportStreamFormat {
    Packet188,
    M2ts192,
    ReedSolomon204,
}

impl TransportStreamFormat {
    pub const fn packet_size(self) -> usize {
        match self {
            Self::Packet188 => 188,
            Self::M2ts192 => 192,
            Self::ReedSolomon204 => 204,
        }
    }

    const fn sync_byte_offset(self) -> usize {
        match self {
            Self::Packet188 | Self::ReedSolomon204 => 0,
            Self::M2ts192 => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportStreamProbe {
    format: TransportStreamFormat,
    stream_start_offset: usize,
    verified_packets: usize,
}

impl TransportStreamProbe {
    pub const fn format(self) -> TransportStreamFormat {
        self.format
    }

    pub const fn packet_size(self) -> usize {
        self.format.packet_size()
    }

    pub const fn stream_start_offset(self) -> usize {
        self.stream_start_offset
    }

    pub const fn verified_packets(self) -> usize {
        self.verified_packets
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportStreamProbeError {
    SampleTooShort,
    SynchronizationNotFound,
}

impl fmt::Display for TransportStreamProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleTooShort => write!(formatter, "input is too short to identify as MPEG-TS"),
            Self::SynchronizationNotFound => write!(
                formatter,
                "input does not contain a valid MPEG-TS packet sequence"
            ),
        }
    }
}

impl std::error::Error for TransportStreamProbeError {}

pub fn probe_transport_stream(
    sample: &[u8],
) -> Result<TransportStreamProbe, TransportStreamProbeError> {
    let smallest_required_sample =
        TransportStreamFormat::Packet188.packet_size() * MINIMUM_PROBE_PACKETS;
    if sample.len() < smallest_required_sample {
        return Err(TransportStreamProbeError::SampleTooShort);
    }

    let formats = [
        TransportStreamFormat::Packet188,
        TransportStreamFormat::M2ts192,
        TransportStreamFormat::ReedSolomon204,
    ];
    let mut best_match = None;

    for format in formats {
        let packet_size = format.packet_size();
        let search_end = sample
            .len()
            .saturating_sub(packet_size * MINIMUM_PROBE_PACKETS)
            .min(MAXIMUM_LEADING_BYTES);
        for stream_start_offset in 0..=search_end {
            let sync_offset = stream_start_offset + format.sync_byte_offset();
            let verified_packets = count_valid_packets(sample, sync_offset, packet_size);
            if verified_packets < MINIMUM_PROBE_PACKETS {
                continue;
            }

            let candidate = TransportStreamProbe {
                format,
                stream_start_offset,
                verified_packets,
            };
            if best_match.is_none_or(|current: TransportStreamProbe| {
                candidate.verified_packets > current.verified_packets
            }) {
                best_match = Some(candidate);
            }
        }
    }

    best_match.ok_or(TransportStreamProbeError::SynchronizationNotFound)
}

fn count_valid_packets(sample: &[u8], sync_offset: usize, packet_size: usize) -> usize {
    let mut verified_packets = 0;
    let mut offset = sync_offset;

    while offset + 4 <= sample.len() && is_valid_packet_header(&sample[offset..offset + 4]) {
        verified_packets += 1;
        offset += packet_size;
    }

    verified_packets
}

fn is_valid_packet_header(header: &[u8]) -> bool {
    header[0] == 0x47 && (header[3] & 0x30) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(format: TransportStreamFormat, continuity_counter: u8) -> Vec<u8> {
        let mut packet = vec![0xff; format.packet_size()];
        let sync_offset = format.sync_byte_offset();
        packet[sync_offset] = 0x47;
        packet[sync_offset + 1] = 0x1f;
        packet[sync_offset + 2] = 0xff;
        packet[sync_offset + 3] = 0x10 | (continuity_counter & 0x0f);
        packet
    }

    fn sample(format: TransportStreamFormat) -> Vec<u8> {
        (0..8).flat_map(|counter| packet(format, counter)).collect()
    }

    #[test]
    fn detects_supported_packet_formats() {
        for format in [
            TransportStreamFormat::Packet188,
            TransportStreamFormat::M2ts192,
            TransportStreamFormat::ReedSolomon204,
        ] {
            let probe = probe_transport_stream(&sample(format));
            assert_eq!(probe.map(TransportStreamProbe::format), Ok(format));
        }
    }

    #[test]
    fn detects_stream_after_leading_bytes() {
        let mut bytes = vec![0xaa; 17];
        bytes.extend(sample(TransportStreamFormat::Packet188));

        let probe = probe_transport_stream(&bytes);

        assert_eq!(probe.map(TransportStreamProbe::stream_start_offset), Ok(17));
    }

    #[test]
    fn rejects_typescript_source() {
        let source = b"export const syncByte = 0x47;\n".repeat(64);

        assert_eq!(
            probe_transport_stream(&source),
            Err(TransportStreamProbeError::SynchronizationNotFound)
        );
    }

    #[test]
    fn rejects_short_sample() {
        assert_eq!(
            probe_transport_stream(&[0x47; 188]),
            Err(TransportStreamProbeError::SampleTooShort)
        );
    }
}
