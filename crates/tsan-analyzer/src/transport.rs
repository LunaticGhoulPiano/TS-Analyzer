use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use tsan_core::{TransportStreamFormat, probe_transport_stream};

use crate::video::{VideoMetadata, VideoProbe};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BroadcastStandard {
    Unknown,
    AtscPsip,
    AtscCablePsip,
    DvbSi,
    Isdb,
    Scte,
    Dtmb,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SignalledModulation {
    #[default]
    Unknown,
    Vsb8,
    Qam64,
    Qam256,
}

impl SignalledModulation {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Vsb8 => "8-VSB",
            Self::Qam64 => "64-QAM",
            Self::Qam256 => "256-QAM",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnalysisReport {
    pub format: TransportStreamFormat,
    pub stream_start_offset: u64,
    pub standard: BroadcastStandard,
    pub signalled_modulation: SignalledModulation,
    pub packets: u64,
    pub malformed_packets: u64,
    pub trailing_bytes: u64,
    pub null_packets: u64,
    pub valid_sections: u64,
    pub section_crc_errors: u64,
    pub pids: BTreeMap<u16, PidReport>,
    pub programs: BTreeMap<u16, ProgramReport>,
    pub video_metadata: BTreeMap<u16, VideoMetadata>,
    pub native: Option<NativeSummary>,
    pub clock_points: Vec<ClockPoint>,
    pub random_access_points: Vec<(u64, u16)>,
    pub tables: BTreeMap<(u16, u8), TableSummary>,
    pub bitrate_windows: Vec<BitrateWindow>,
    pub section_events: Vec<SectionEvent>,
    pub sync_byte_errors: u64,
    pub sync_loss_events: u64,
    pub tr_events: Vec<TrEvent>,
}

#[derive(Clone, Debug)]
pub struct TrEvent {
    pub packet_index: u64,
    pub pid: u16,
    pub indicator: &'static str,
    pub detail: String,
    pub exact_packet: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct SectionEvent {
    pub packet_index: u64,
    pub pid: u16,
    pub table_id: u8,
    pub extension: u16,
    pub version: u8,
    pub section_number: u8,
    pub last_section_number: u8,
}

pub const BITRATE_WINDOW_PACKETS: u64 = 1024;

#[derive(Clone, Debug, Default)]
pub struct BitrateWindow {
    pub first_packet: u64,
    pub packet_count: u32,
    pub pid_packets: BTreeMap<u16, u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockKind {
    Pcr,
    Pts,
    Dts,
}

#[derive(Clone, Copy, Debug)]
pub struct ClockPoint {
    pub packet_index: u64,
    pub pid: u16,
    pub kind: ClockKind,
    pub ticks: u64,
}

#[derive(Clone, Debug, Default)]
pub struct TableSummary {
    pub sections: u64,
    pub crc_errors: u64,
    pub version: Option<u8>,
    pub section_number: Option<u8>,
    pub last_section_number: Option<u8>,
    pub first_section: Vec<u8>,
    pub instances: BTreeMap<(u16, u8, u8), Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeSummary {
    pub packets: u64,
    pub continuity_errors: u64,
    pub valid_sections: u64,
    pub pat_sections: u64,
    pub pmt_sections: u64,
    pub standards: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaInformation {
    pub video_codecs: Vec<&'static str>,
    pub audio_codecs: Vec<&'static str>,
    pub video_services: usize,
    pub audio_tracks: usize,
    pub video_width: Option<u32>,
    pub video_height: Option<u32>,
    pub video_frame_rate: Option<(u32, u32)>,
}

impl AnalysisReport {
    pub fn packet_offset(&self, packet_index: u64) -> u64 {
        self.stream_start_offset
            + packet_index * self.format.packet_size() as u64
            + u64::from(self.format == TransportStreamFormat::M2ts192) * 4
    }

    pub fn media_information(&self) -> MediaInformation {
        let mut video_codecs = BTreeSet::new();
        let mut audio_codecs = BTreeSet::new();
        let mut audio_pids = BTreeSet::new();
        let mut video_services = 0;
        let mut video_width = None;
        let mut video_height = None;
        let mut video_frame_rate = None;
        for program in self.programs.values() {
            let mut has_video = false;
            for (pid, stream) in &program.streams {
                if stream.is_video() {
                    has_video = true;
                    video_codecs.insert(stream.name_for_standard(self.standard));
                    if let Some(details) = self.video_metadata.get(pid) {
                        video_width = video_width.or(details.width);
                        video_height = video_height.or(details.height);
                        video_frame_rate = video_frame_rate.or(details.frame_rate);
                    }
                } else if stream.is_audio_for_standard(self.standard) {
                    audio_codecs.insert(stream.name_for_standard(self.standard));
                    audio_pids.insert(*pid);
                }
            }
            video_services += usize::from(has_video);
        }
        MediaInformation {
            video_codecs: video_codecs.into_iter().collect(),
            audio_codecs: audio_codecs.into_iter().collect(),
            video_services,
            audio_tracks: audio_pids.len(),
            video_width,
            video_height,
            video_frame_rate,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PidReport {
    pub packets: u64,
    pub first_packet: Option<u64>,
    pub payload_packets: u64,
    pub transport_errors: u64,
    pub continuity_errors: u64,
    pub duplicates: u64,
    pub scrambled_packets: u64,
    pub pcr_samples: u64,
    pub max_pcr_gap_27mhz: Option<u64>,
    pub pcr_repetition_errors: u64,
    pub pcr_accuracy_errors: u64,
    pub pcr_discontinuity_errors: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ProgramReport {
    pub pmt_pid: u16,
    pub pcr_pid: Option<u16>,
    pub streams: BTreeMap<u16, StreamReport>,
}

#[derive(Clone, Copy, Debug)]
pub struct StreamReport {
    pub stream_type: u8,
}

impl StreamReport {
    pub const fn is_video(self) -> bool {
        matches!(self.stream_type, 0x01 | 0x02 | 0x1b | 0x24)
    }

    pub const fn is_audio(self) -> bool {
        matches!(self.stream_type, 0x03 | 0x04 | 0x0f | 0x11)
    }

    pub const fn is_audio_for_standard(self, standard: BroadcastStandard) -> bool {
        self.is_audio()
            || (matches!(
                standard,
                BroadcastStandard::AtscPsip | BroadcastStandard::AtscCablePsip
            ) && matches!(self.stream_type, 0x81 | 0x87))
    }

    pub const fn name_for_standard(self, standard: BroadcastStandard) -> &'static str {
        match (standard, self.stream_type) {
            (BroadcastStandard::AtscPsip | BroadcastStandard::AtscCablePsip, 0x81) => "AC-3",
            (BroadcastStandard::AtscPsip | BroadcastStandard::AtscCablePsip, 0x87) => "E-AC-3",
            _ => self.name(),
        }
    }

    pub const fn name(self) -> &'static str {
        match self.stream_type {
            0x01 => "MPEG-1 video",
            0x02 => "MPEG-2 video",
            0x03 | 0x04 => "MPEG audio",
            0x0f => "AAC",
            0x11 => "AAC LATM",
            0x1b => "H.264/AVC",
            0x24 => "H.265/HEVC",
            0x81 | 0x87 => "Private stream (descriptor required)",
            _ => "Other / descriptor-defined",
        }
    }
}

#[derive(Default)]
struct PidState {
    previous_counter: Option<u8>,
    previous_packet: Vec<u8>,
    previous_pcr: Option<u64>,
    previous_pcr_packet: Option<u64>,
    previous_pcr_interval: Option<(u64, u64)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TableKey {
    pid: u16,
    extension: u16,
    version: u8,
    last_section: u8,
}

impl TableKey {
    fn from_section(pid: u16, section: &[u8]) -> Option<(Self, u8)> {
        let number = section[6];
        let last_section = section[7];
        (number <= last_section).then_some((
            Self {
                pid,
                extension: u16::from_be_bytes([section[3], section[4]]),
                version: (section[5] >> 1) & 0x1f,
                last_section,
            },
            number,
        ))
    }
}

#[derive(Default)]
struct PatAssembly {
    key: Option<TableKey>,
    sections: BTreeMap<u8, BTreeMap<u16, u16>>,
}

impl PatAssembly {
    fn push(
        &mut self,
        key: TableKey,
        number: u8,
        programs: BTreeMap<u16, u16>,
    ) -> Option<BTreeMap<u16, u16>> {
        if self.key != Some(key) {
            self.key = Some(key);
            self.sections.clear();
        }
        self.sections.insert(number, programs);
        if self.sections.len() != usize::from(key.last_section) + 1 {
            return None;
        }
        Some(
            self.sections
                .values()
                .flat_map(|section| section.iter().map(|(&program, &pid)| (program, pid)))
                .collect(),
        )
    }
}

#[derive(Default)]
struct PmtTable {
    pcr_pid: Option<u16>,
    streams: BTreeMap<u16, StreamReport>,
}

#[derive(Default)]
struct PmtAssembly {
    key: Option<TableKey>,
    sections: BTreeMap<u8, PmtTable>,
}

impl PmtAssembly {
    fn push(&mut self, key: TableKey, number: u8, table: PmtTable) -> Option<PmtTable> {
        if self.key != Some(key) {
            self.key = Some(key);
            self.sections.clear();
        }
        self.sections.insert(number, table);
        if self.sections.len() != usize::from(key.last_section) + 1 {
            return None;
        }
        let pcr_pid = self.sections.values().next()?.pcr_pid;
        let mut streams = BTreeMap::new();
        for section in self.sections.values() {
            if section.pcr_pid != pcr_pid {
                return None;
            }
            streams.extend(section.streams.iter().map(|(&pid, &stream)| (pid, stream)));
        }
        Some(PmtTable { pcr_pid, streams })
    }
}

pub fn analyze_file(path: &Path) -> io::Result<AnalysisReport> {
    let mut file = File::open(path)?;
    let mut sample = vec![0; 64 * 1024];
    let sample_size = file.read(&mut sample)?;
    let probe = probe_transport_stream(&sample[..sample_size])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    reader.seek(SeekFrom::Start(probe.stream_start_offset() as u64))?;
    let mut analyzer = Analyzer::new(probe.format());
    analyzer.report.stream_start_offset = probe.stream_start_offset() as u64;
    #[cfg(feature = "native-tsduck")]
    let mut native = tsan_tsduck_sys::Session::new()
        .map_err(|error| io::Error::other(format!("TSDuck initialization failed: {error:?}")))?;
    let packet_size = probe.packet_size();
    let sync_offset = usize::from(probe.format() == TransportStreamFormat::M2ts192) * 4;
    let mut bytes = [0; 204];
    loop {
        let mut filled = 0;
        while filled < packet_size {
            let count = reader.read(&mut bytes[filled..packet_size])?;
            if count == 0 {
                break;
            }
            filled += count;
        }
        if filled == 0 {
            break;
        }
        if filled < packet_size {
            analyzer.report.trailing_bytes = filled as u64;
            break;
        }
        let packet = &bytes[sync_offset..sync_offset + 188];
        analyzer.packet(packet);
        #[cfg(feature = "native-tsduck")]
        if packet[0] == 0x47 {
            native.feed_aligned(packet).map_err(|error| {
                io::Error::other(format!("TSDuck packet analysis failed: {error:?}"))
            })?;
        }
    }
    let report = analyzer.finish();
    #[cfg(feature = "native-tsduck")]
    let mut report = report;
    #[cfg(feature = "native-tsduck")]
    {
        let snapshot = native
            .snapshot()
            .map_err(|error| io::Error::other(format!("TSDuck snapshot failed: {error:?}")))?;
        report.native = Some(NativeSummary {
            packets: snapshot.packets,
            continuity_errors: snapshot.continuity_errors,
            valid_sections: snapshot.valid_sections,
            pat_sections: snapshot.pat_sections,
            pmt_sections: snapshot.pmt_sections,
            standards: snapshot.standards,
        });
        report.standard = match snapshot.standards {
            flags if flags & 0x08 != 0 => match report.standard {
                BroadcastStandard::AtscCablePsip => BroadcastStandard::AtscCablePsip,
                _ => BroadcastStandard::AtscPsip,
            },
            flags if flags & 0x10 != 0 => BroadcastStandard::Isdb,
            flags if flags & 0x02 != 0 => BroadcastStandard::DvbSi,
            flags if flags & 0x80 != 0 => BroadcastStandard::Dtmb,
            flags if flags & 0x04 != 0 => BroadcastStandard::Scte,
            _ => report.standard,
        };
    }
    Ok(report)
}

struct Analyzer {
    report: AnalysisReport,
    states: BTreeMap<u16, PidState>,
    sections: BTreeMap<u16, SectionAssembler>,
    video_probes: BTreeMap<u16, VideoProbe>,
    active_pat: Option<TableKey>,
    pending_pat: PatAssembly,
    active_pmts: BTreeMap<u16, TableKey>,
    pending_pmts: BTreeMap<u16, PmtAssembly>,
    previous_sync_error: bool,
}

fn signalled_atsc_modulation(section: &[u8]) -> SignalledModulation {
    if section.len() < 14 {
        return SignalledModulation::Unknown;
    }
    let channels = usize::from(section[9]);
    let payload_end = section.len().saturating_sub(4);
    let mut detected = SignalledModulation::Unknown;
    for index in 0..channels {
        let modulation_offset = 10 + index * 32 + 17;
        if modulation_offset >= payload_end {
            break;
        }
        let candidate = match section[modulation_offset] {
            0x02 => SignalledModulation::Qam64,
            0x03 => SignalledModulation::Qam256,
            0x04 => SignalledModulation::Vsb8,
            _ => continue,
        };
        if detected != SignalledModulation::Unknown && detected != candidate {
            return SignalledModulation::Unknown;
        }
        detected = candidate;
    }
    detected
}

impl Analyzer {
    fn finish(mut self) -> AnalysisReport {
        for (pid, probe) in self.video_probes {
            let active = self.report.programs.values().any(|program| {
                program
                    .streams
                    .get(&pid)
                    .is_some_and(|stream| stream.is_video())
            });
            if active && let Some(metadata) = probe.finish() {
                self.report.video_metadata.insert(pid, metadata);
            }
        }
        self.report
    }

    fn new(format: TransportStreamFormat) -> Self {
        let mut sections = BTreeMap::new();
        sections.insert(0, SectionAssembler::default());
        sections.insert(1, SectionAssembler::default());
        sections.insert(2, SectionAssembler::default());
        sections.insert(0x0013, SectionAssembler::default());
        sections.insert(0x001e, SectionAssembler::default());
        sections.insert(0x001f, SectionAssembler::default());
        sections.insert(0x0010, SectionAssembler::default());
        sections.insert(0x0011, SectionAssembler::default());
        sections.insert(0x0012, SectionAssembler::default());
        sections.insert(0x0014, SectionAssembler::default());
        sections.insert(0x0023, SectionAssembler::default());
        sections.insert(0x0024, SectionAssembler::default());
        sections.insert(0x0029, SectionAssembler::default());
        sections.insert(0x1ffb, SectionAssembler::default());
        Self {
            report: AnalysisReport {
                format,
                stream_start_offset: 0,
                standard: BroadcastStandard::Unknown,
                signalled_modulation: SignalledModulation::Unknown,
                packets: 0,
                malformed_packets: 0,
                trailing_bytes: 0,
                null_packets: 0,
                valid_sections: 0,
                section_crc_errors: 0,
                pids: BTreeMap::new(),
                programs: BTreeMap::new(),
                video_metadata: BTreeMap::new(),
                native: None,
                clock_points: Vec::new(),
                random_access_points: Vec::new(),
                tables: BTreeMap::new(),
                bitrate_windows: Vec::new(),
                section_events: Vec::new(),
                sync_byte_errors: 0,
                sync_loss_events: 0,
                tr_events: Vec::new(),
            },
            states: BTreeMap::new(),
            sections,
            video_probes: BTreeMap::new(),
            active_pat: None,
            pending_pat: PatAssembly::default(),
            active_pmts: BTreeMap::new(),
            pending_pmts: BTreeMap::new(),
            previous_sync_error: false,
        }
    }

    fn packet(&mut self, ts: &[u8]) {
        self.report.packets += 1;
        let index = self.report.packets - 1;
        if index.is_multiple_of(BITRATE_WINDOW_PACKETS) {
            self.report.bitrate_windows.push(BitrateWindow {
                first_packet: index,
                ..BitrateWindow::default()
            });
        }
        if let Some(window) = self.report.bitrate_windows.last_mut() {
            window.packet_count += 1;
        }
        if ts[0] != 0x47 || ts[3] & 0x30 == 0 {
            if ts[0] != 0x47 {
                if self.previous_sync_error {
                    self.report.sync_loss_events += 1;
                }
                self.previous_sync_error = true;
                self.report.sync_byte_errors += 1;
            }
            self.report.malformed_packets += 1;
            return;
        }
        self.previous_sync_error = false;
        let pid = (u16::from(ts[1] & 0x1f) << 8) | u16::from(ts[2]);
        let counter = ts[3] & 0x0f;
        let has_payload = ts[3] & 0x10 != 0;
        let has_adaptation = ts[3] & 0x20 != 0;
        if let Some(window) = self.report.bitrate_windows.last_mut() {
            *window.pid_packets.entry(pid).or_default() += 1;
        }
        if pid == 0x1fff {
            self.report.null_packets += 1;
        }
        let is_program_pcr = self
            .report
            .programs
            .values()
            .any(|program| program.pcr_pid == Some(pid));
        let entry = self.report.pids.entry(pid).or_default();
        let state = self.states.entry(pid).or_default();
        entry.packets += 1;
        entry.first_packet.get_or_insert(index);
        if ts[1] & 0x80 != 0 {
            entry.transport_errors += 1;
        }
        if ts[3] & 0xc0 != 0 {
            entry.scrambled_packets += 1;
        }
        let mut payload_offset = 4;
        if has_adaptation {
            let length = usize::from(ts[4]);
            payload_offset = 5 + length;
            if payload_offset > 188 {
                self.report.malformed_packets += 1;
                return;
            }
            if length > 0 {
                if ts[5] & 0x40 != 0 {
                    let point = (self.report.packets - 1, pid);
                    if self.report.random_access_points.last().copied() != Some(point) {
                        self.report.random_access_points.push(point);
                    }
                }
                if ts[5] & 0x80 != 0 {
                    state.previous_counter = None;
                    state.previous_pcr = None;
                    state.previous_pcr_packet = None;
                    state.previous_pcr_interval = None;
                }
                if ts[5] & 0x10 != 0 {
                    if length < 7 {
                        self.report.malformed_packets += 1;
                        return;
                    }
                    let base = (u64::from(ts[6]) << 25)
                        | (u64::from(ts[7]) << 17)
                        | (u64::from(ts[8]) << 9)
                        | (u64::from(ts[9]) << 1)
                        | u64::from(ts[10] >> 7);
                    let extension = (u64::from(ts[10] & 1) << 8) | u64::from(ts[11]);
                    let pcr = base * 300 + extension;
                    entry.pcr_samples += 1;
                    self.report.clock_points.push(ClockPoint {
                        packet_index: self.report.packets - 1,
                        pid,
                        kind: ClockKind::Pcr,
                        ticks: pcr,
                    });
                    if !is_program_pcr {
                        state.previous_pcr = None;
                        state.previous_pcr_packet = None;
                        state.previous_pcr_interval = None;
                    }
                    if let (Some(previous), Some(previous_packet)) =
                        (state.previous_pcr, state.previous_pcr_packet)
                    {
                        const WRAP: u64 = (1_u64 << 33) * 300;
                        let gap = (pcr + WRAP - previous) % WRAP;
                        let packet_gap = self.report.packets - 1 - previous_packet;
                        if gap < WRAP / 2 && packet_gap > 0 {
                            entry.max_pcr_gap_27mhz =
                                Some(entry.max_pcr_gap_27mhz.unwrap_or(0).max(gap));
                            if gap >= 1_080_000 {
                                entry.pcr_repetition_errors += 1;
                                self.report.tr_events.push(TrEvent {
                                    packet_index: index,
                                    pid,
                                    indicator: "PCR_repetition_error",
                                    detail: format!(
                                        "PCR interval {:.3} ms exceeds 40 ms",
                                        gap as f64 / 27_000.0
                                    ),
                                    exact_packet: true,
                                });
                            }
                            if gap > 2_700_000 {
                                entry.pcr_discontinuity_errors += 1;
                                self.report.tr_events.push(TrEvent {
                                    packet_index: index,
                                    pid,
                                    indicator: "PCR_discontinuity_indicator_error",
                                    detail: "PCR gap exceeds 100 ms without discontinuity flag"
                                        .to_owned(),
                                    exact_packet: true,
                                });
                            }
                            if let Some((last_gap, last_packets)) = state.previous_pcr_interval {
                                let predicted =
                                    last_gap as f64 * packet_gap as f64 / last_packets as f64;
                                if (gap as f64 - predicted).abs() >= 13.5 {
                                    entry.pcr_accuracy_errors += 1;
                                    self.report.tr_events.push(TrEvent {
                                        packet_index: index,
                                        pid,
                                        indicator: "PCR_accuracy_error",
                                        detail: format!(
                                            "PCR deviation {:+.1} ns (limit +/-500 ns)",
                                            (gap as f64 - predicted) * 1_000.0 / 27.0
                                        ),
                                        exact_packet: true,
                                    });
                                }
                            }
                            state.previous_pcr_interval = Some((gap, packet_gap));
                        } else {
                            entry.pcr_discontinuity_errors += 1;
                            self.report.tr_events.push(TrEvent {
                                packet_index: index,
                                pid,
                                indicator: "PCR_discontinuity_indicator_error",
                                detail: "PCR moved backward without discontinuity flag".to_owned(),
                                exact_packet: true,
                            });
                            state.previous_pcr_interval = None;
                        }
                    }
                    if is_program_pcr {
                        state.previous_pcr_packet = Some(self.report.packets - 1);
                        state.previous_pcr = Some(pcr);
                    }
                }
            }
        }
        if !has_payload || payload_offset == 188 {
            return;
        }
        entry.payload_packets += 1;
        if pid != 0x1fff
            && let Some(previous) = state.previous_counter
        {
            if counter == previous && state.previous_packet == ts {
                entry.duplicates += 1;
                return;
            }
            if counter != (previous + 1) & 0x0f {
                entry.continuity_errors += 1;
            }
        }
        state.previous_counter = Some(counter);
        state.previous_packet.clear();
        state.previous_packet.extend_from_slice(ts);
        if ts[1] & 0x80 != 0 {
            return;
        }
        if ts[3] & 0xc0 == 0 {
            if ts[1] & 0x40 != 0 {
                let (pts, dts) = pes_timestamps(&ts[payload_offset..]);
                for (kind, value) in [(ClockKind::Pts, pts), (ClockKind::Dts, dts)] {
                    if let Some(ticks) = value {
                        self.report.clock_points.push(ClockPoint {
                            packet_index: self.report.packets - 1,
                            pid,
                            kind,
                            ticks,
                        });
                    }
                }
            }
            let video_type = self
                .report
                .programs
                .values()
                .find_map(|program| program.streams.get(&pid))
                .filter(|stream| stream.is_video())
                .map(|stream| stream.stream_type);
            if let Some(stream_type) = video_type {
                let probe = self
                    .video_probes
                    .entry(pid)
                    .or_insert_with(|| VideoProbe::new(stream_type));
                if probe.stream_type() != stream_type {
                    *probe = VideoProbe::new(stream_type);
                }
                let nal_random_access = probe.push_packet(&ts[payload_offset..], ts[1] & 0x40 != 0);
                if nal_random_access {
                    let point = (self.report.packets - 1, pid);
                    if self.report.random_access_points.last().copied() != Some(point) {
                        self.report.random_access_points.push(point);
                    }
                }
            }
        }
        if let Some(assembler) = self.sections.get_mut(&pid) {
            for section in assembler.push(&ts[payload_offset..], ts[1] & 0x40 != 0, counter) {
                self.section(pid, &section);
            }
        }
    }
}

#[derive(Default)]
struct SectionAssembler {
    partial: Vec<u8>,
    previous_counter: Option<u8>,
}

impl SectionAssembler {
    fn push(&mut self, payload: &[u8], start: bool, counter: u8) -> Vec<Vec<u8>> {
        let mut sections = Vec::new();
        if self
            .previous_counter
            .is_some_and(|last| counter != (last + 1) & 0x0f)
        {
            self.partial.clear();
        }
        self.previous_counter = Some(counter);
        if start {
            let Some((&pointer, data)) = payload.split_first() else {
                self.partial.clear();
                return sections;
            };
            let pointer = usize::from(pointer);
            if pointer > data.len() {
                self.partial.clear();
                return sections;
            }
            if !self.partial.is_empty() {
                self.extend(&data[..pointer], &mut sections);
            }
            self.partial.clear();
            self.extend(&data[pointer..], &mut sections);
        } else if !self.partial.is_empty() {
            self.extend(payload, &mut sections);
        }
        sections
    }

    fn extend(&mut self, mut data: &[u8], sections: &mut Vec<Vec<u8>>) {
        while !data.is_empty() {
            if self.partial.is_empty() && data[0] == 0xff {
                return;
            }
            let wanted = if self.partial.len() < 3 {
                3 - self.partial.len()
            } else {
                let length =
                    (usize::from(self.partial[1] & 0x0f) << 8) | usize::from(self.partial[2]);
                if !(4..=4093).contains(&length) {
                    self.partial.clear();
                    return;
                }
                3 + length - self.partial.len()
            };
            let take = wanted.min(data.len());
            self.partial.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.partial.len() < 3 {
                continue;
            }
            let length = (usize::from(self.partial[1] & 0x0f) << 8) | usize::from(self.partial[2]);
            if !(4..=4093).contains(&length) {
                self.partial.clear();
                return;
            }
            if self.partial.len() == 3 + length {
                sections.push(std::mem::take(&mut self.partial));
            }
        }
    }
}
fn pes_timestamps(payload: &[u8]) -> (Option<u64>, Option<u64>) {
    if payload.len() < 14 || !payload.starts_with(&[0, 0, 1]) {
        return (None, None);
    }
    let flags = (payload[7] >> 6) & 0x03;
    if flags & 0x02 == 0 {
        return (None, None);
    }
    let pts = decode_pes_time(&payload[9..14]);
    let dts = if flags == 0x03 && payload.len() >= 19 {
        decode_pes_time(&payload[14..19])
    } else {
        None
    };
    (pts, dts)
}

fn decode_pes_time(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 5 || bytes[0] & 1 == 0 || bytes[2] & 1 == 0 || bytes[4] & 1 == 0 {
        return None;
    }
    Some(
        (u64::from((bytes[0] >> 1) & 0x07) << 30)
            | (u64::from(bytes[1]) << 22)
            | (u64::from(bytes[2] >> 1) << 15)
            | (u64::from(bytes[3]) << 7)
            | u64::from(bytes[4] >> 1),
    )
}

fn mpeg_crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

impl Analyzer {
    fn section(&mut self, pid: u16, section: &[u8]) {
        if section.len() < 3 {
            return;
        }
        let table = self.report.tables.entry((pid, section[0])).or_default();
        table.sections += 1;
        let syntax = section[1] & 0x80 != 0;
        let crc_present = syntax || section[0] == 0x73;
        if crc_present
            && ((syntax && section.len() < 12)
                || (!syntax && section.len() < 11)
                || mpeg_crc32(section) != 0)
        {
            table.crc_errors += 1;
            self.report.section_crc_errors += 1;
            return;
        }
        if table.first_section.is_empty() {
            table.first_section.extend_from_slice(section);
        }
        let (extension, version, number) = if syntax {
            (
                u16::from_be_bytes([section[3], section[4]]),
                (section[5] >> 1) & 0x1f,
                section[6],
            )
        } else {
            (0, 0, 0)
        };
        table
            .instances
            .insert((extension, version, number), section.to_vec());
        if syntax {
            table.version = Some(version);
            table.section_number = Some(number);
            table.last_section_number = Some(section[7]);
        }
        if pid == 0x0010 && !matches!(section[0], 0x40 | 0x41 | 0x72) {
            self.report.tr_events.push(TrEvent {
                packet_index: self.report.packets - 1,
                pid,
                indicator: "NIT_actual_error",
                detail: format!("Unexpected table ID 0x{:02X} on NIT PID", section[0]),
                exact_packet: true,
            });
        }
        self.report.section_events.push(SectionEvent {
            packet_index: self.report.packets - 1,
            pid,
            table_id: section[0],
            extension,
            version,
            section_number: number,
            last_section_number: if syntax { section[7] } else { 0 },
        });
        if !syntax {
            return;
        }
        self.report.valid_sections += 1;
        if section[5] & 1 == 0 {
            return;
        }
        match (pid, section[0]) {
            (0, 0x00) => self.pat(section),
            (_, 0x02) => self.pmt(pid, section),
            (0x1ffb, 0xc8) => {
                self.report.standard = BroadcastStandard::AtscPsip;
                self.report.signalled_modulation = signalled_atsc_modulation(section);
            }
            (0x1ffb, 0xc9) => {
                self.report.standard = BroadcastStandard::AtscCablePsip;
                self.report.signalled_modulation = signalled_atsc_modulation(section);
            }
            (0x1ffb, 0xc7 | 0xca..=0xcd) if self.report.standard == BroadcastStandard::Unknown => {
                self.report.standard = BroadcastStandard::AtscPsip;
            }
            (0x0023, 0xc3) | (0x0024, 0xc4) | (0x0029, 0xc8) => {
                self.report.standard = BroadcastStandard::Isdb;
            }
            (0x0010, 0x40 | 0x41) | (0x0011, 0x42 | 0x46)
                if self.report.standard == BroadcastStandard::Unknown =>
            {
                self.report.standard = BroadcastStandard::DvbSi;
            }
            _ => {}
        }
    }

    fn pat(&mut self, section: &[u8]) {
        if !(section.len() - 12).is_multiple_of(4) {
            return;
        }
        let Some((key, number)) = TableKey::from_section(0, section) else {
            return;
        };
        if self.active_pat == Some(key) {
            return;
        }
        let programs = section[8..section.len() - 4]
            .chunks_exact(4)
            .filter_map(|entry| {
                let number = u16::from_be_bytes([entry[0], entry[1]]);
                (number != 0).then_some((
                    number,
                    (u16::from(entry[2] & 0x1f) << 8) | u16::from(entry[3]),
                ))
            })
            .collect();
        let Some(programs) = self.pending_pat.push(key, number, programs) else {
            return;
        };
        let mut previous = std::mem::take(&mut self.report.programs);
        let mut current = BTreeMap::new();
        for (number, pmt_pid) in programs {
            let mut program = previous
                .remove(&number)
                .filter(|program| program.pmt_pid == pmt_pid)
                .unwrap_or_default();
            program.pmt_pid = pmt_pid;
            current.insert(number, program);
            self.sections.entry(pmt_pid).or_default();
        }
        self.sections.retain(|pid, _| {
            matches!(
                *pid,
                0 | 1 | 2 | 0x0010..=0x0014 | 0x001e | 0x001f | 0x0023 | 0x0024 | 0x0029 | 0x1ffb
            ) || current.values().any(|program| program.pmt_pid == *pid)
        });
        self.active_pmts.retain(|number, key| {
            current
                .get(number)
                .is_some_and(|program| program.pmt_pid == key.pid)
        });
        self.pending_pmts.retain(|number, assembly| {
            current.get(number).is_some_and(|program| {
                assembly
                    .key
                    .is_some_and(|pending| program.pmt_pid == pending.pid)
            })
        });
        self.report.programs = current;
        self.active_pat = Some(key);
    }

    fn pmt(&mut self, pid: u16, section: &[u8]) {
        if section.len() < 16 {
            return;
        }
        let Some((key, section_number)) = TableKey::from_section(pid, section) else {
            return;
        };
        let number = key.extension;
        if !self
            .report
            .programs
            .get(&number)
            .is_some_and(|program| program.pmt_pid == pid)
            || self.active_pmts.get(&number) == Some(&key)
        {
            return;
        }
        let pcr_pid = (u16::from(section[8] & 0x1f) << 8) | u16::from(section[9]);
        let info_length = (usize::from(section[10] & 0x0f) << 8) | usize::from(section[11]);
        let end = section.len() - 4;
        let mut cursor = 12 + info_length;
        if cursor > end {
            return;
        }
        let mut streams = BTreeMap::new();
        while cursor + 5 <= end {
            let stream_type = section[cursor];
            let stream_pid =
                (u16::from(section[cursor + 1] & 0x1f) << 8) | u16::from(section[cursor + 2]);
            let descriptors =
                (usize::from(section[cursor + 3] & 0x0f) << 8) | usize::from(section[cursor + 4]);
            cursor += 5 + descriptors;
            if cursor > end {
                return;
            }
            streams.insert(stream_pid, StreamReport { stream_type });
        }
        if cursor != end {
            return;
        }
        let table = PmtTable {
            pcr_pid: (pcr_pid != 0x1fff).then_some(pcr_pid),
            streams,
        };
        let Some(table) =
            self.pending_pmts
                .entry(number)
                .or_default()
                .push(key, section_number, table)
        else {
            return;
        };
        let Some(program) = self.report.programs.get_mut(&number) else {
            return;
        };
        program.pcr_pid = table.pcr_pid;
        program.streams = table.streams;
        self.active_pmts.insert(number, key);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn with_crc(mut bytes: Vec<u8>) -> Vec<u8> {
        let crc = mpeg_crc32(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        assert_eq!(mpeg_crc32(&bytes), 0);
        bytes
    }

    fn packet(pid: u16, counter: u8, section: &[u8]) -> [u8; 188] {
        let mut bytes = [0xff; 188];
        bytes[0] = 0x47;
        bytes[1] = 0x40 | (pid >> 8) as u8;
        bytes[2] = pid as u8;
        bytes[3] = 0x10 | counter;
        bytes[4] = 0;
        bytes[5..5 + section.len()].copy_from_slice(section);
        bytes
    }

    fn pat_section(version: u8, number: u8, last: u8, programs: &[(u16, u16)]) -> Vec<u8> {
        let length = 9 + 4 * programs.len();
        let mut section = vec![
            0x00,
            0xb0 | ((length >> 8) as u8 & 0x0f),
            length as u8,
            0,
            1,
            0xc1 | ((version & 0x1f) << 1),
            number,
            last,
        ];
        for &(program, pid) in programs {
            section.extend_from_slice(&[
                (program >> 8) as u8,
                program as u8,
                0xe0 | (pid >> 8) as u8,
                pid as u8,
            ]);
        }
        with_crc(section)
    }

    fn pmt_section(
        version: u8,
        number: u8,
        last: u8,
        program: u16,
        pcr_pid: u16,
        streams: &[(u8, u16)],
    ) -> Vec<u8> {
        let length = 13 + 5 * streams.len();
        let mut section = vec![
            0x02,
            0xb0 | ((length >> 8) as u8 & 0x0f),
            length as u8,
            (program >> 8) as u8,
            program as u8,
            0xc1 | ((version & 0x1f) << 1),
            number,
            last,
            0xe0 | (pcr_pid >> 8) as u8,
            pcr_pid as u8,
            0xf0,
            0,
        ];
        for &(stream_type, pid) in streams {
            section.extend_from_slice(&[stream_type, 0xe0 | (pid >> 8) as u8, pid as u8, 0xf0, 0]);
        }
        with_crc(section)
    }

    #[test]
    fn parses_pat_and_pmt_independently_of_playback() {
        let pat = with_crc(vec![
            0x00, 0xb0, 0x0d, 0x00, 0x01, 0xc1, 0, 0, 0, 1, 0xe1, 0x00,
        ]);
        let pmt = with_crc(vec![
            0x02, 0xb0, 0x17, 0, 1, 0xc1, 0, 0, 0xe1, 0x01, 0xf0, 0, 0x24, 0xe1, 0x01, 0xf0, 0,
            0x81, 0xe1, 0x02, 0xf0, 0,
        ]);
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        analyzer.packet(&packet(0, 0, &pat));
        analyzer.packet(&packet(0x100, 0, &pmt));
        let program = &analyzer.report.programs[&1];
        assert_eq!(program.pmt_pid, 0x100);
        assert_eq!(program.pcr_pid, Some(0x101));
        assert_eq!(program.streams[&0x101].name(), "H.265/HEVC");
        assert_eq!(analyzer.report.valid_sections, 2);
        analyzer.report.standard = BroadcastStandard::AtscPsip;
        assert_eq!(
            analyzer.report.media_information(),
            MediaInformation {
                video_codecs: vec!["H.265/HEVC"],
                audio_codecs: vec!["AC-3"],
                video_services: 1,
                audio_tracks: 1,
                video_width: None,
                video_height: None,
                video_frame_rate: None,
            }
        );
        analyzer.report.standard = BroadcastStandard::DvbSi;
        assert_eq!(analyzer.report.media_information().audio_tracks, 0);
        assert_eq!(
            program.streams[&0x102].name_for_standard(BroadcastStandard::DvbSi),
            "Private stream (descriptor required)"
        );
    }

    #[test]
    fn detects_duplicate_and_discontinuous_packets() {
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        let first = packet(0x101, 0, &[]);
        analyzer.packet(&first);
        analyzer.packet(&first);
        analyzer.packet(&packet(0x101, 2, &[]));
        let pid = &analyzer.report.pids[&0x101];
        assert_eq!(pid.duplicates, 1);
        assert_eq!(pid.continuity_errors, 1);
    }

    #[test]
    fn assembles_sections_across_packets_and_rejects_bad_crc() {
        let section = with_crc(vec![0x00, 0xb0, 0x0d, 0, 1, 0xc1, 0, 0, 0, 1, 0xe1, 0x00]);
        let mut assembler = SectionAssembler::default();
        assert!(
            assembler
                .push(&[0, section[0], section[1]], true, 0)
                .is_empty()
        );
        assert_eq!(
            assembler.push(&section[2..], false, 1),
            vec![section.clone()]
        );
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        let mut bad = section;
        bad[8] ^= 1;
        analyzer.packet(&packet(0, 0, &bad));
        assert_eq!(analyzer.report.section_crc_errors, 1);
        assert!(analyzer.report.programs.is_empty());
    }

    #[test]
    fn pat_version_switch_waits_for_all_sections_and_removes_old_programs() {
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        analyzer.packet(&packet(0, 0, &pat_section(0, 0, 0, &[(1, 0x100)])));
        analyzer.packet(&packet(
            0x100,
            0,
            &pmt_section(0, 0, 0, 1, 0x101, &[(0x24, 0x101)]),
        ));
        analyzer.packet(&packet(0, 1, &pat_section(1, 0, 1, &[(2, 0x200)])));
        assert!(analyzer.report.programs.contains_key(&1));
        assert!(!analyzer.report.programs.contains_key(&2));

        analyzer.packet(&packet(0, 2, &pat_section(1, 1, 1, &[(3, 0x300)])));
        assert!(!analyzer.report.programs.contains_key(&1));
        assert_eq!(analyzer.report.programs[&2].pmt_pid, 0x200);
        assert_eq!(analyzer.report.programs[&3].pmt_pid, 0x300);
        assert!(!analyzer.sections.contains_key(&0x100));
    }

    #[test]
    fn changing_pmt_pid_discards_old_program_details() {
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        analyzer.packet(&packet(0, 0, &pat_section(0, 0, 0, &[(1, 0x100)])));
        analyzer.packet(&packet(
            0x100,
            0,
            &pmt_section(0, 0, 0, 1, 0x101, &[(0x24, 0x101)]),
        ));
        analyzer.packet(&packet(0, 1, &pat_section(1, 0, 0, &[(1, 0x200)])));
        let program = &analyzer.report.programs[&1];
        assert_eq!(program.pmt_pid, 0x200);
        assert_eq!(program.pcr_pid, None);
        assert!(program.streams.is_empty());

        analyzer.packet(&packet(
            0x100,
            1,
            &pmt_section(1, 0, 0, 1, 0x101, &[(0x24, 0x101)]),
        ));
        assert!(analyzer.report.programs[&1].streams.is_empty());
        analyzer.packet(&packet(
            0x200,
            0,
            &pmt_section(1, 0, 0, 1, 0x201, &[(0x1b, 0x201)]),
        ));
        assert_eq!(analyzer.report.programs[&1].pcr_pid, Some(0x201));
        assert!(analyzer.report.programs[&1].streams.contains_key(&0x201));
    }

    #[test]
    fn pmt_version_switch_replaces_streams_only_after_last_section() {
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        analyzer.packet(&packet(0, 0, &pat_section(0, 0, 0, &[(1, 0x100)])));
        analyzer.packet(&packet(
            0x100,
            0,
            &pmt_section(0, 0, 0, 1, 0x101, &[(0x24, 0x101), (0x81, 0x102)]),
        ));
        analyzer.packet(&packet(
            0x100,
            1,
            &pmt_section(1, 0, 1, 1, 0x103, &[(0x1b, 0x103)]),
        ));
        let program = &analyzer.report.programs[&1];
        assert_eq!(program.pcr_pid, Some(0x101));
        assert!(program.streams.contains_key(&0x101));
        assert!(!program.streams.contains_key(&0x103));

        analyzer.packet(&packet(
            0x100,
            2,
            &pmt_section(1, 1, 1, 1, 0x103, &[(0x0f, 0x104)]),
        ));
        let program = &analyzer.report.programs[&1];
        assert_eq!(program.pcr_pid, Some(0x103));
        assert!(!program.streams.contains_key(&0x101));
        assert!(!program.streams.contains_key(&0x102));
        assert_eq!(program.streams[&0x103].name(), "H.264/AVC");
        assert_eq!(program.streams[&0x104].name(), "AAC");
    }

    #[test]
    fn identifies_atsc_only_from_valid_psip_section() {
        let psip = with_crc(vec![0xc7, 0xb0, 0x09, 0, 0, 0xc1, 0, 0]);
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        analyzer.packet(&packet(0x1ffb, 0, &psip));
        assert_eq!(analyzer.report.standard, BroadcastStandard::AtscPsip);

        let mut corrupt = psip;
        corrupt[4] ^= 1;
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        analyzer.packet(&packet(0x1ffb, 0, &corrupt));
        assert_eq!(analyzer.report.standard, BroadcastStandard::Unknown);
        assert_eq!(analyzer.report.section_crc_errors, 1);
    }
    fn vct_section(table_id: u8, modulation_mode: u8) -> Vec<u8> {
        let mut section = vec![table_id, 0xb0, 0, 0, 1, 0xc1, 0, 0, 0, 1];
        let mut channel = vec![0; 32];
        channel[17] = modulation_mode;
        section.extend(channel);
        section.extend([0xf0, 0]);
        let section_length = section.len() + 4 - 3;
        section[1] |= ((section_length >> 8) as u8) & 0x0f;
        section[2] = section_length as u8;
        with_crc(section)
    }

    #[test]
    fn distinguishes_terrestrial_and_cable_psip_from_vct_signalling() {
        for (table_id, modulation_mode, standard, modulation, profile) in [
            (
                0xc8,
                0x04,
                BroadcastStandard::AtscPsip,
                SignalledModulation::Vsb8,
                crate::ComplianceProfile::Atsc1A65_2013,
            ),
            (
                0xc9,
                0x02,
                BroadcastStandard::AtscCablePsip,
                SignalledModulation::Qam64,
                crate::ComplianceProfile::J83BQam64AtscCablePsip,
            ),
            (
                0xc9,
                0x03,
                BroadcastStandard::AtscCablePsip,
                SignalledModulation::Qam256,
                crate::ComplianceProfile::J83BQam256AtscCablePsip,
            ),
        ] {
            let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
            analyzer.packet(&packet(0x1ffb, 0, &vct_section(table_id, modulation_mode)));
            let report = analyzer.finish();
            assert_eq!(report.standard, standard);
            assert_eq!(report.signalled_modulation, modulation);
            assert_eq!(crate::ComplianceProfile::suggested(&report), profile);
        }
    }

    fn pcr_packet(pid: u16, counter: u8, pcr: u64) -> [u8; 188] {
        let mut bytes = [0xff; 188];
        let base = pcr / 300;
        let extension = pcr % 300;
        bytes[0] = 0x47;
        bytes[1] = (pid >> 8) as u8 & 0x1f;
        bytes[2] = pid as u8;
        bytes[3] = 0x30 | counter;
        bytes[4] = 7;
        bytes[5] = 0x10;
        bytes[6] = (base >> 25) as u8;
        bytes[7] = (base >> 17) as u8;
        bytes[8] = (base >> 9) as u8;
        bytes[9] = (base >> 1) as u8;
        bytes[10] = ((base & 1) << 7) as u8 | 0x7e | (extension >> 8) as u8;
        bytes[11] = extension as u8;
        bytes
    }

    #[test]
    fn repetition_checks_are_not_reported_as_pass_without_a_timebase() {
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        analyzer.packet(&packet(0, 0, &pat_section(0, 0, 0, &[(1, 0x100)])));
        analyzer.packet(&packet(
            0x100,
            0,
            &pmt_section(0, 0, 0, 1, 0x101, &[(0x24, 0x101)]),
        ));
        let report = analyzer.finish();
        let compliance =
            crate::compliance::tr101290_report(&report, crate::ComplianceProfile::DvbSi);
        assert_eq!(compliance.count("PAT_error"), None);
        assert_eq!(compliance.count("PMT_error"), None);
        assert_eq!(compliance.count("NIT_actual_error"), None);
    }

    #[test]
    fn tr_pcr_checks_start_after_pmt_and_null_pid_has_no_cc_error() {
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        analyzer.packet(&pcr_packet(0x101, 0, 100_000));
        analyzer.packet(&packet(0, 0, &pat_section(0, 0, 0, &[(1, 0x100)])));
        analyzer.packet(&packet(
            0x100,
            0,
            &pmt_section(0, 0, 0, 1, 0x101, &[(0x24, 0x101)]),
        ));
        for (counter, ticks) in [(1, 1_000_000), (2, 1_270_000), (3, 1_540_014)] {
            for _ in 0..100 {
                analyzer.packet(&packet(0x1fff, 0, &[]));
            }
            analyzer.packet(&pcr_packet(0x101, counter, ticks));
        }
        let report = analyzer.finish();
        let tr = crate::compliance::tr101290_report(&report, crate::ComplianceProfile::DvbSi);
        assert_eq!(report.pids[&0x1fff].continuity_errors, 0);
        assert_eq!(tr.count("Continuity_count_error"), Some(0));
        assert_eq!(tr.count("PCR_accuracy_error"), Some(1));
        assert_eq!(tr.count("PCR_repetition_error"), Some(0));
        let event = tr
            .events
            .iter()
            .find(|event| event.indicator == "PCR_accuracy_error");
        assert!(event.is_some());
    }

    #[test]
    fn pmt_on_dvb_nit_pid_is_reported_as_table_id_error() {
        let mut analyzer = Analyzer::new(TransportStreamFormat::Packet188);
        analyzer.packet(&packet(0, 0, &pat_section(0, 0, 0, &[(1, 0x10)])));
        analyzer.packet(&packet(
            0x10,
            0,
            &pmt_section(0, 0, 0, 1, 0x101, &[(0x24, 0x101)]),
        ));
        let report = analyzer.finish();
        let tr = crate::compliance::tr101290_report(&report, crate::ComplianceProfile::DvbSi);
        assert_eq!(tr.count("NIT_actual_error"), Some(1));
        assert_eq!(tr.events[0].packet_index, 1);
        assert_eq!(report.packet_offset(1), 188);
    }
}
