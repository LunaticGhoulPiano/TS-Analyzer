use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use tsan_core::{TransportStreamFormat, probe_transport_stream};

#[derive(Clone, Debug)]
pub struct PacketRecord {
    pub index: u64,
    pub offset: u64,
    pub pid: u16,
    pub continuity_counter: u8,
    pub payload_unit_start: bool,
    pub transport_error: bool,
    pub scrambled: bool,
    pub adaptation: bool,
    pub payload: bool,
    pub random_access: bool,
    pub discontinuity: bool,
    pub pcr_27mhz: Option<u64>,
    pub bytes: [u8; 188],
}

#[derive(Clone, Debug)]
pub struct PacketWindow {
    pub packets: Vec<PacketRecord>,
    pub next_index: u64,
}

pub fn read_packet_window(
    path: &Path,
    start_index: u64,
    max_records: usize,
    pid_filter: Option<u16>,
) -> io::Result<PacketWindow> {
    let mut file = File::open(path)?;
    let mut sample = vec![0; 64 * 1024];
    let sample_size = file.read(&mut sample)?;
    let probe = probe_transport_stream(&sample[..sample_size])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let packet_size = probe.packet_size();
    let sync_offset = usize::from(probe.format() == TransportStreamFormat::M2ts192) * 4;
    let first_offset = probe.stream_start_offset() as u64;
    let start_offset = first_offset
        .checked_add(start_index.saturating_mul(packet_size as u64))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "packet index too large"))?;
    file.seek(SeekFrom::Start(start_offset))?;
    let mut bytes = [0; 204];
    let mut packets = Vec::new();
    let mut index = start_index;
    // A bounded search prevents an absent PID filter from scanning the whole file on the UI thread.
    let search_limit = max_records.max(1).saturating_mul(4096);
    for _ in 0..search_limit {
        if file.read_exact(&mut bytes[..packet_size]).is_err() {
            break;
        }
        let packet = &bytes[sync_offset..sync_offset + 188];
        if packet[0] != 0x47 {
            index += 1;
            continue;
        }
        let pid = (u16::from(packet[1] & 0x1f) << 8) | u16::from(packet[2]);
        if pid_filter.is_some_and(|wanted| wanted != pid) {
            index += 1;
            continue;
        }
        let adaptation = packet[3] & 0x20 != 0;
        let mut pcr_27mhz = None;
        let mut random_access = false;
        let mut discontinuity = false;
        if adaptation && packet[4] > 0 {
            discontinuity = packet[5] & 0x80 != 0;
            random_access = packet[5] & 0x40 != 0;
            if packet[4] >= 7 && packet[5] & 0x10 != 0 {
                let base = (u64::from(packet[6]) << 25)
                    | (u64::from(packet[7]) << 17)
                    | (u64::from(packet[8]) << 9)
                    | (u64::from(packet[9]) << 1)
                    | u64::from(packet[10] >> 7);
                let extension = (u64::from(packet[10] & 1) << 8) | u64::from(packet[11]);
                pcr_27mhz = Some(base * 300 + extension);
            }
        }
        let mut raw = [0; 188];
        raw.copy_from_slice(packet);
        packets.push(PacketRecord {
            index,
            offset: first_offset + index * packet_size as u64 + sync_offset as u64,
            pid,
            continuity_counter: packet[3] & 0x0f,
            payload_unit_start: packet[1] & 0x40 != 0,
            transport_error: packet[1] & 0x80 != 0,
            scrambled: packet[3] & 0xc0 != 0,
            adaptation,
            payload: packet[3] & 0x10 != 0,
            random_access,
            discontinuity,
            pcr_27mhz,
            bytes: raw,
        });
        index += 1;
        if packets.len() >= max_records {
            break;
        }
    }
    Ok(PacketWindow {
        packets,
        next_index: index,
    })
}
