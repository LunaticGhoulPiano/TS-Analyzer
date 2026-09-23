use std::io::{self, BufReader, Read};
use std::time::Duration;

use crate::{TransportStreamFormat, TransportStreamProbe};

const PTS_CLOCK_HZ: u64 = 90_000;
const PTS_WRAP: u64 = 1 << 33;
const MAX_FORWARD_GAP: u64 = 10 * PTS_CLOCK_HZ;
const MAX_REORDER: u64 = PTS_CLOCK_HZ;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportStreamSeekPoint {
    position: Duration,
    byte_offset: u64,
    decode_position: Duration,
}

impl TransportStreamSeekPoint {
    pub const fn position(self) -> Duration {
        self.position
    }

    pub const fn decode_position(self) -> Duration {
        self.decode_position
    }

    pub const fn byte_offset(self) -> u64 {
        self.byte_offset
    }
}

/// Video presentation timeline and packet locations derived without PCR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportStreamTimeline {
    video_pid: u16,
    duration: Duration,
    seek_points: Vec<TransportStreamSeekPoint>,
}

impl TransportStreamTimeline {
    pub const fn video_pid(&self) -> u16 {
        self.video_pid
    }

    pub const fn duration(&self) -> Duration {
        self.duration
    }

    pub fn seek_point(&self, position: Duration) -> Option<TransportStreamSeekPoint> {
        let index = self
            .seek_points
            .partition_point(|point| point.position <= position);
        self.seek_points
            .get(index.saturating_sub(1))
            .or_else(|| self.seek_points.first())
            .copied()
    }

    pub fn seek_point_count(&self) -> usize {
        self.seek_points.len()
    }
}

/// Scan PES headers in packet order. PTS supplies the stable time axis and TS
/// random-access indicators supply byte locations that can restart decoding.
pub fn index_transport_stream(
    reader: impl Read,
    probe: TransportStreamProbe,
) -> io::Result<Option<TransportStreamTimeline>> {
    let mut reader = BufReader::with_capacity(1024 * 1024, reader);
    if probe.stream_start_offset() != 0 {
        io::copy(
            &mut reader.by_ref().take(probe.stream_start_offset() as u64),
            &mut io::sink(),
        )?;
    }

    let packet_size = probe.packet_size();
    let sync_offset = match probe.format() {
        TransportStreamFormat::M2ts192 => 4,
        TransportStreamFormat::Packet188 | TransportStreamFormat::ReedSolomon204 => 0,
    };
    let mut packet = [0_u8; 204];
    let mut packet_offset = probe.stream_start_offset() as u64;
    let mut video_pid = None;
    let mut first_pts = None;
    let mut greatest_pts = None;
    let mut elapsed_ticks = 0_u64;
    let mut discontinuous = false;
    let mut pes_points = Vec::new();
    let mut random_access_points = Vec::new();

    loop {
        let current_offset = packet_offset;
        let mut read = 0;
        while read < packet_size {
            let count = reader.read(&mut packet[read..packet_size])?;
            if count == 0 {
                return Ok(if read == 0 && !discontinuous {
                    timeline(
                        video_pid,
                        first_pts,
                        elapsed_ticks,
                        pes_points,
                        random_access_points,
                    )
                } else {
                    None
                });
            }
            read += count;
        }
        packet_offset = packet_offset.saturating_add(packet_size as u64);

        let ts = &packet[sync_offset..sync_offset + 188];
        if ts[0] != 0x47 || ts[1] & 0x80 != 0 || ts[1] & 0x40 == 0 {
            continue;
        }
        let pid = (u16::from(ts[1] & 0x1f) << 8) | u16::from(ts[2]);
        if video_pid.is_some_and(|selected| selected != pid) || ts[3] & 0x10 == 0 {
            continue;
        }
        let has_adaptation = ts[3] & 0x20 != 0;
        let random_access =
            has_adaptation && ts[4] != 0 && ts.get(5).is_some_and(|flags| flags & 0x40 != 0);
        let mut payload_start = 4;
        if has_adaptation {
            payload_start += 1 + usize::from(ts[4]);
        }
        if payload_start + 14 > ts.len() {
            continue;
        }
        let payload = &ts[payload_start..];
        if payload[..3] != [0, 0, 1] || !(0xe0..=0xef).contains(&payload[3]) {
            continue;
        }
        if payload[6] & 0xc0 != 0x80 || payload[7] & 0x80 == 0 || payload[8] < 5 {
            continue;
        }
        let p = &payload[9..14];
        if p[0] & 1 == 0 || p[2] & 1 == 0 || p[4] & 1 == 0 {
            continue;
        }
        video_pid.get_or_insert(pid);
        let pts = (u64::from((p[0] >> 1) & 7) << 30)
            | (u64::from(p[1]) << 22)
            | (u64::from(p[2] >> 1) << 15)
            | (u64::from(p[3]) << 7)
            | u64::from(p[4] >> 1);
        let position_ticks = if let Some(greatest) = greatest_pts {
            let forward = (pts + PTS_WRAP - greatest) % PTS_WRAP;
            let backward = (greatest + PTS_WRAP - pts) % PTS_WRAP;
            if forward <= MAX_FORWARD_GAP {
                elapsed_ticks = elapsed_ticks.saturating_add(forward);
                greatest_pts = Some(pts);
                elapsed_ticks
            } else if backward <= MAX_REORDER {
                elapsed_ticks.saturating_sub(backward)
            } else {
                discontinuous = true;
                continue;
            }
        } else {
            first_pts = Some(pts);
            greatest_pts = Some(pts);
            0
        };
        let point = TransportStreamSeekPoint {
            position: ticks_to_duration(position_ticks),
            byte_offset: current_offset,
            decode_position: {
                let dts = if payload[7] & 0xc0 == 0xc0 && payload[8] >= 10 {
                    payload.get(14..19).map(|p| {
                        (u64::from((p[0] >> 1) & 7) << 30)
                            | (u64::from(p[1]) << 22)
                            | (u64::from(p[2] >> 1) << 15)
                            | (u64::from(p[3]) << 7)
                            | u64::from(p[4] >> 1)
                    })
                } else {
                    None
                };
                let delay = dts
                    .map(|d| (pts + PTS_WRAP - d) % PTS_WRAP)
                    .filter(|d| *d < PTS_CLOCK_HZ * 10)
                    .unwrap_or(0);
                ticks_to_duration(position_ticks.saturating_sub(delay))
            },
        };
        pes_points.push(point);
        if random_access {
            random_access_points.push(point);
        }
    }
}

fn timeline(
    video_pid: Option<u16>,
    first_pts: Option<u64>,
    elapsed_ticks: u64,
    mut pes_points: Vec<TransportStreamSeekPoint>,
    mut random_access_points: Vec<TransportStreamSeekPoint>,
) -> Option<TransportStreamTimeline> {
    let video_pid = video_pid?;
    first_pts?;
    if elapsed_ticks == 0 {
        return None;
    }
    let seek_points = if random_access_points.is_empty() {
        &mut pes_points
    } else {
        &mut random_access_points
    };
    seek_points.sort_unstable_by_key(|point| (point.position, point.byte_offset));
    seek_points.dedup_by_key(|point| point.position);
    Some(TransportStreamTimeline {
        video_pid,
        duration: ticks_to_duration(elapsed_ticks),
        seek_points: std::mem::take(seek_points),
    })
}

fn ticks_to_duration(ticks: u64) -> Duration {
    let nanos = u128::from(ticks) * 1_000_000_000 / u128::from(PTS_CLOCK_HZ);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::probe_transport_stream;

    fn packet(pid: u16, pts: u64, random_access: bool) -> [u8; 188] {
        let mut bytes = [0xff; 188];
        bytes[0] = 0x47;
        bytes[1] = 0x40 | ((pid >> 8) as u8 & 0x1f);
        bytes[2] = pid as u8;
        bytes[3] = 0x30;
        bytes[4] = 1;
        bytes[5] = if random_access { 0x40 } else { 0 };
        bytes[6..15].copy_from_slice(&[0, 0, 1, 0xe0, 0, 0, 0x80, 0x80, 5]);
        bytes[15] = 0x20 | ((pts >> 29) as u8 & 0x0e) | 1;
        bytes[16] = (pts >> 22) as u8;
        bytes[17] = ((pts >> 14) as u8 & 0xfe) | 1;
        bytes[18] = (pts >> 7) as u8;
        bytes[19] = ((pts << 1) as u8 & 0xfe) | 1;
        bytes
    }

    #[test]
    fn duration_and_seek_points_come_from_video_pts() {
        let bytes = [
            packet(256, 90_000, true),
            packet(256, 180_000, false),
            packet(256, 135_000, false),
            packet(256, 225_000, true),
            packet(256, 270_000, true),
        ]
        .concat();
        let probe = probe_transport_stream(&bytes).unwrap();
        let timeline = index_transport_stream(bytes.as_slice(), probe)
            .unwrap()
            .unwrap();
        assert_eq!(timeline.video_pid(), 256);
        assert_eq!(timeline.duration(), Duration::from_secs(2));
        assert_eq!(timeline.seek_point_count(), 3);
        assert_eq!(
            timeline.seek_point(Duration::from_millis(750)),
            Some(TransportStreamSeekPoint {
                position: Duration::ZERO,
                byte_offset: 0,
                decode_position: Duration::ZERO,
            })
        );
    }

    #[test]
    fn discontinuity_is_not_silently_added_to_duration() {
        let bytes = [
            packet(256, 90_000, true),
            packet(256, 180_000, true),
            packet(256, 270_000, true),
            packet(256, 90_000, true),
            packet(256, 180_000, true),
        ]
        .concat();
        let probe = probe_transport_stream(&bytes).unwrap();
        assert!(
            index_transport_stream(bytes.as_slice(), probe)
                .unwrap()
                .is_none()
        );
    }
}
