use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::ui_components::{ArrowScrollArea, ResizeHandle};
use eframe::egui;
use tsan_analyzer::{
    AnalysisReport, BITRATE_WINDOW_PACKETS, ClockKind, PacketWindow, read_packet_window,
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum HorizontalUnit {
    Packet,
    #[default]
    Seconds,
}

#[derive(Clone, Copy)]
struct GraphView {
    x_from_percent: f64,
    x_to_percent: f64,
    y_bounds: Option<(f64, f64)>,
}

impl Default for GraphView {
    fn default() -> Self {
        Self {
            x_from_percent: 0.0,
            x_to_percent: 100.0,
            y_bounds: None,
        }
    }
}

#[derive(Default)]
pub struct ViewState {
    pub psi_filter: String,
    pub packet_pid_filter: String,
    pub packet_start: u64,
    pub selected_packet: Option<u64>,
    pub tr101290_filter: String,
    pub graph_pid_filter: String,
    pub horizontal_unit: HorizontalUnit,
    pub show_pcr: bool,
    pub show_pts: bool,
    pub show_dts: bool,
    pub graph_show_legend: bool,
    pub bitrate_difference_percent: bool,
    graph_views: BTreeMap<String, GraphView>,
    graph_drag_start: Option<(String, egui::Pos2)>,
    graph_pan_start: Option<(String, egui::Pos2, GraphView)>,
    pub graph_series_enabled: BTreeMap<(u16, u8), bool>,
    pub graph_service: Option<u16>,
    pub graph_relative_clock: bool,
    pub tr101290_profile: Option<tsan_analyzer::ComplianceProfile>,
    packet_split_ratio: f32,
    tr101290_summary_ratio: f32,
    packet_cache: Option<(PathBuf, u64, Option<u16>, PacketWindow)>,
}

impl ViewState {
    pub fn new() -> Self {
        Self {
            show_pcr: true,
            show_pts: true,
            show_dts: true,
            graph_show_legend: true,
            bitrate_difference_percent: true,
            packet_split_ratio: 0.48,
            tr101290_summary_ratio: 0.58,
            ..Self::default()
        }
    }
}

fn pid_filter(text: &str) -> Result<Option<u16>, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16)
    } else {
        text.parse::<u16>()
    };
    value
        .ok()
        .filter(|value| *value <= 0x1fff)
        .map(Some)
        .ok_or("Enter a PID from 0 to 8191 (or 0x0000 to 0x1FFF).")
}

fn item_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(225, 228, 235)
    } else {
        egui::Color32::from_rgb(45, 49, 58)
    }
}

fn value_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(105, 205, 255)
    } else {
        egui::Color32::from_rgb(0, 86, 150)
    }
}

fn key_value(ui: &mut egui::Ui, item: &str, value: impl ToString) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(format!("{item}:"))
                .monospace()
                .strong()
                .color(item_color(ui)),
        )
        .on_hover_text(
            "Syntax names follow ISO/IEC 13818-1, DVB SI or ATSC PSIP where applicable.",
        );
        ui.label(
            egui::RichText::new(value.to_string())
                .monospace()
                .color(value_color(ui)),
        );
    });
}

fn value_cell(ui: &mut egui::Ui, value: impl ToString) {
    ui.label(
        egui::RichText::new(value.to_string())
            .monospace()
            .color(value_color(ui)),
    );
}

fn table_name(pid: u16, id: u8) -> &'static str {
    match (pid, id) {
        (0, 0x00) => "PAT",
        (1, 0x01) => "CAT",
        (_, 0x02) => "PMT",
        (0x10, 0x40) => "NIT actual",
        (0x10, 0x41) => "NIT other",
        (0x11, 0x42) => "SDT actual",
        (0x11, 0x46) => "SDT other",
        (0x11, 0x4a) => "BAT",
        (0x12, 0x4e) => "EIT present/following",
        (0x12, 0x50..=0x6f) => "EIT schedule",
        (0x14, 0x70) => "TDT",
        (0x14, 0x73) => "TOT",
        (0x1ffb, 0xc7) => "ATSC MGT",
        (0x1ffb, 0xc8 | 0xc9) => "ATSC VCT",
        (0x1ffb, 0xcd) => "ATSC STT",
        _ => "Other table",
    }
}

fn descriptor_name(tag: u8) -> &'static str {
    match tag {
        0x02 => "Video stream",
        0x03 => "Audio stream",
        0x05 => "Registration",
        0x09 => "Conditional access",
        0x0a => "ISO 639 language",
        0x40 => "Network name",
        0x41 => "Service list",
        0x43 => "Satellite delivery",
        0x44 => "Cable delivery",
        0x48 => "Service",
        0x4a => "Linkage",
        0x4d => "Short event",
        0x4e => "Extended event",
        0x52 => "Stream identifier",
        0x56 => "Teletext",
        0x59 => "Subtitling",
        0x6a => "AC-3",
        _ => "Descriptor",
    }
}

fn descriptor_list(ui: &mut egui::Ui, bytes: &[u8]) {
    let mut cursor = 0;
    while cursor + 2 <= bytes.len() {
        let tag = bytes[cursor];
        let length = usize::from(bytes[cursor + 1]);
        cursor += 2;
        if cursor + length > bytes.len() {
            ui.colored_label(ui.visuals().error_fg_color, "Truncated descriptor");
            return;
        }
        let body = &bytes[cursor..cursor + length];
        egui::CollapsingHeader::new(
            egui::RichText::new(format!(
                "{} descriptor (tag 0x{tag:02X})",
                descriptor_name(tag)
            ))
            .color(item_color(ui)),
        )
        .default_open(true)
        .show(ui, |ui| {
            key_value(ui, "descriptor_tag", format!("0x{tag:02X}"));
            key_value(ui, "descriptor_length", length);
            if tag == 0x40 {
                key_value(ui, "network_name", String::from_utf8_lossy(body));
            } else if tag == 0x48 && body.len() >= 3 {
                key_value(ui, "service_type", format!("0x{:02X}", body[0]));
                let provider_len = usize::from(body[1]);
                if body.len() > 2 + provider_len {
                    let provider = &body[2..2 + provider_len];
                    let name_len = usize::from(body[2 + provider_len]);
                    if body.len() >= 3 + provider_len + name_len {
                        key_value(
                            ui,
                            "service_provider_name",
                            String::from_utf8_lossy(provider),
                        );
                        key_value(
                            ui,
                            "service_name",
                            String::from_utf8_lossy(
                                &body[3 + provider_len..3 + provider_len + name_len],
                            ),
                        );
                    }
                }
            } else if tag == 0x0a && body.len() >= 3 {
                key_value(
                    ui,
                    "ISO_639_language_code",
                    String::from_utf8_lossy(&body[..3]),
                );
                if body.len() >= 4 {
                    key_value(ui, "audio_type", format!("0x{:02X}", body[3]));
                }
            } else if tag == 0x09 && body.len() >= 4 {
                let ca_pid = (u16::from(body[2] & 0x1f) << 8) | u16::from(body[3]);
                key_value(
                    ui,
                    "CA_system_ID",
                    format!("0x{:04X}", u16::from_be_bytes([body[0], body[1]])),
                );
                key_value(ui, "CA_PID", format!("0x{ca_pid:04X}"));
            }
        });
        cursor += length;
    }
    if cursor != bytes.len() {
        ui.colored_label(ui.visuals().error_fg_color, "Trailing descriptor byte");
    }
}

fn section_header(ui: &mut egui::Ui, pid: u16, table_id: u8, section: &[u8]) -> (usize, bool) {
    let section_syntax_indicator = section[1] & 0x80 != 0;
    let private_indicator = section[1] & 0x40 != 0;
    let section_length = (usize::from(section[1] & 0x0f) << 8) | usize::from(section[2]);
    let crc_present = section_syntax_indicator || table_id == 0x73;
    let end = section.len().saturating_sub(usize::from(crc_present) * 4);
    egui::CollapsingHeader::new(
        egui::RichText::new("Section header fields")
            .strong()
            .color(item_color(ui)),
    )
    .default_open(true)
    .show(ui, |ui| {
        key_value(
            ui,
            "table_id",
            format!("0x{table_id:02X} ({})", table_name(pid, table_id)),
        );
        key_value(
            ui,
            "section_syntax_indicator",
            u8::from(section_syntax_indicator),
        );
        key_value(ui, "private_indicator", u8::from(private_indicator));
        key_value(
            ui,
            "reserved",
            format!("0b{:02b}", (section[1] >> 4) & 0x03),
        );
        key_value(ui, "section_length", section_length);
        if section_syntax_indicator && section.len() >= 8 {
            let extension_name = match table_id {
                0x00 | 0x42 | 0x46 => "transport_stream_id",
                0x02 => "program_number",
                0x40 | 0x41 => "network_id",
                0x4e..=0x6f => "service_id",
                _ => "table_id_extension",
            };
            key_value(
                ui,
                extension_name,
                format!(
                    "0x{:04X} ({})",
                    u16::from_be_bytes([section[3], section[4]]),
                    u16::from_be_bytes([section[3], section[4]])
                ),
            );
            key_value(ui, "version_number", (section[5] >> 1) & 0x1f);
            key_value(ui, "current_next_indicator", section[5] & 0x01);
            key_value(ui, "section_number", section[6]);
            key_value(ui, "last_section_number", section[7]);
        }
        if crc_present && section.len() >= 4 {
            key_value(
                ui,
                "CRC_32",
                format!(
                    "0x{:08X}",
                    u32::from_be_bytes([
                        section[section.len() - 4],
                        section[section.len() - 3],
                        section[section.len() - 2],
                        section[section.len() - 1],
                    ])
                ),
            );
        }
    });
    (end, section_syntax_indicator)
}

fn section_tree(ui: &mut egui::Ui, pid: u16, table_id: u8, section: &[u8]) {
    if section.len() < 3 {
        return;
    }
    let (end, _section_syntax_indicator) = section_header(ui, pid, table_id, section);
    match table_id {
        0x00 if section.len() >= 12 => {
            egui::CollapsingHeader::new(
                egui::RichText::new("programs")
                    .strong()
                    .color(item_color(ui)),
            )
            .default_open(true)
            .show(ui, |ui| {
                for entry in section[8..end].chunks_exact(4) {
                    let program = u16::from_be_bytes([entry[0], entry[1]]);
                    let target_pid = (u16::from(entry[2] & 0x1f) << 8) | u16::from(entry[3]);
                    if program == 0 {
                        key_value(ui, "network_PID", format!("0x{target_pid:04X}"));
                    } else {
                        ui.group(|ui| {
                            key_value(ui, "program_number", program);
                            key_value(ui, "program_map_PID", format!("0x{target_pid:04X}"));
                        });
                    }
                }
            });
        }
        0x01 if section.len() >= 12 => descriptor_list(ui, &section[8..end]),
        0x02 if section.len() >= 16 => {
            let pcr_pid = (u16::from(section[8] & 0x1f) << 8) | u16::from(section[9]);
            let info_len = (usize::from(section[10] & 0x0f) << 8) | usize::from(section[11]);
            key_value(ui, "PCR_PID", format!("0x{pcr_pid:04X}"));
            key_value(ui, "program_info_length", info_len);
            let mut cursor = 12 + info_len;
            if cursor > end {
                return;
            }
            if info_len > 0 {
                egui::CollapsingHeader::new(
                    egui::RichText::new("program_descriptors").color(item_color(ui)),
                )
                .show(ui, |ui| descriptor_list(ui, &section[12..cursor]));
            }
            while cursor + 5 <= end {
                let stream_type = section[cursor];
                let elementary_pid =
                    (u16::from(section[cursor + 1] & 0x1f) << 8) | u16::from(section[cursor + 2]);
                let es_info_length = (usize::from(section[cursor + 3] & 0x0f) << 8)
                    | usize::from(section[cursor + 4]);
                cursor += 5;
                if cursor + es_info_length > end {
                    break;
                }
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("elementary_PID 0x{elementary_pid:04X}"))
                        .color(item_color(ui)),
                )
                .default_open(true)
                .show(ui, |ui| {
                    key_value(ui, "stream_type", format!("0x{stream_type:02X}"));
                    key_value(ui, "elementary_PID", format!("0x{elementary_pid:04X}"));
                    key_value(ui, "ES_info_length", es_info_length);
                    descriptor_list(ui, &section[cursor..cursor + es_info_length]);
                });
                cursor += es_info_length;
            }
        }
        0x40 | 0x41 if section.len() >= 16 => {
            let descriptors_len = (usize::from(section[8] & 0x0f) << 8) | usize::from(section[9]);
            key_value(ui, "network_descriptors_length", descriptors_len);
            let mut cursor = 10 + descriptors_len;
            if cursor + 2 > end {
                return;
            }
            egui::CollapsingHeader::new(
                egui::RichText::new("network_descriptors").color(item_color(ui)),
            )
            .show(ui, |ui| {
                descriptor_list(ui, &section[10..cursor]);
            });
            let transport_stream_loop_length =
                (usize::from(section[cursor] & 0x0f) << 8) | usize::from(section[cursor + 1]);
            key_value(
                ui,
                "transport_stream_loop_length",
                transport_stream_loop_length,
            );
            cursor += 2;
            let loop_end = (cursor + transport_stream_loop_length).min(end);
            while cursor + 6 <= loop_end {
                let tsid = u16::from_be_bytes([section[cursor], section[cursor + 1]]);
                let onid = u16::from_be_bytes([section[cursor + 2], section[cursor + 3]]);
                let len = (usize::from(section[cursor + 4] & 0x0f) << 8)
                    | usize::from(section[cursor + 5]);
                cursor += 6;
                if cursor + len > end {
                    break;
                }
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("transport_stream_id {tsid}"))
                        .color(item_color(ui)),
                )
                .show(ui, |ui| {
                    key_value(ui, "transport_stream_id", tsid);
                    key_value(ui, "original_network_id", onid);
                    key_value(ui, "transport_descriptors_length", len);
                    descriptor_list(ui, &section[cursor..cursor + len]);
                });
                cursor += len;
            }
        }
        0x42 | 0x46 if section.len() >= 15 => {
            key_value(
                ui,
                "original_network_id",
                u16::from_be_bytes([section[8], section[9]]),
            );
            let mut cursor = 11;
            while cursor + 5 <= end {
                let service = u16::from_be_bytes([section[cursor], section[cursor + 1]]);
                let eit_flags = section[cursor + 2];
                let status_flags = section[cursor + 3];
                let running = (status_flags >> 5) & 0x07;
                let free_ca_mode = status_flags & 0x10 != 0;
                let len =
                    (usize::from(status_flags & 0x0f) << 8) | usize::from(section[cursor + 4]);
                cursor += 5;
                if cursor + len > end {
                    break;
                }
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("service_id {service}")).color(item_color(ui)),
                )
                .show(ui, |ui| {
                    key_value(ui, "service_id", service);
                    key_value(ui, "EIT_schedule_flag", (eit_flags >> 1) & 0x01);
                    key_value(ui, "EIT_present_following_flag", eit_flags & 0x01);
                    key_value(ui, "running_status", running);
                    key_value(ui, "free_CA_mode", u8::from(free_ca_mode));
                    key_value(ui, "descriptors_loop_length", len);
                    descriptor_list(ui, &section[cursor..cursor + len]);
                });
                cursor += len;
            }
        }
        0x4e..=0x6f if section.len() >= 18 => {
            key_value(
                ui,
                "transport_stream_id",
                u16::from_be_bytes([section[8], section[9]]),
            );
            key_value(
                ui,
                "original_network_id",
                u16::from_be_bytes([section[10], section[11]]),
            );
            key_value(ui, "segment_last_section_number", section[12]);
            key_value(ui, "last_table_id", format!("0x{:02X}", section[13]));
            let mut cursor = 14;
            while cursor + 12 <= end {
                let event = u16::from_be_bytes([section[cursor], section[cursor + 1]]);
                let len = (usize::from(section[cursor + 10] & 0x0f) << 8)
                    | usize::from(section[cursor + 11]);
                cursor += 12;
                if cursor + len > end {
                    break;
                }
                let start_time = &section[cursor - 10..cursor - 5];
                let duration = &section[cursor - 5..cursor - 2];
                let flags = section[cursor - 2];
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("event_id {event}")).color(item_color(ui)),
                )
                .show(ui, |ui| {
                    key_value(ui, "event_id", event);
                    key_value(ui, "start_time", format!("{:02X?}", start_time));
                    key_value(ui, "duration", format!("{:02X?}", duration));
                    key_value(ui, "running_status", (flags >> 5) & 0x07);
                    key_value(ui, "free_CA_mode", u8::from(flags & 0x10 != 0));
                    key_value(ui, "descriptors_loop_length", len);
                    descriptor_list(ui, &section[cursor..cursor + len]);
                });
                cursor += len;
            }
        }
        0xc7 if section.len() >= 15 => {
            let count = u16::from_be_bytes([section[9], section[10]]);
            key_value(ui, "protocol_version", section[8]);
            key_value(ui, "tables_defined", count);
            let mut cursor = 11;
            for _ in 0..count {
                if cursor + 11 > end {
                    break;
                }
                let table_type = u16::from_be_bytes([section[cursor], section[cursor + 1]]);
                let pid =
                    (u16::from(section[cursor + 2] & 0x1f) << 8) | u16::from(section[cursor + 3]);
                let len = (usize::from(section[cursor + 9] & 0x0f) << 8)
                    | usize::from(section[cursor + 10]);
                cursor += 11;
                if cursor + len > end {
                    break;
                }
                let table_type_version_number = section[cursor - 7] & 0x1f;
                let number_bytes = u32::from_be_bytes([
                    section[cursor - 6],
                    section[cursor - 5],
                    section[cursor - 4],
                    section[cursor - 3],
                ]);
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("table_type 0x{table_type:04X}"))
                        .color(item_color(ui)),
                )
                .show(ui, |ui| {
                    key_value(ui, "table_type", format!("0x{table_type:04X}"));
                    key_value(ui, "table_type_PID", format!("0x{pid:04X}"));
                    key_value(ui, "table_type_version_number", table_type_version_number);
                    key_value(ui, "number_bytes", number_bytes);
                    key_value(ui, "table_type_descriptors_length", len);
                    descriptor_list(ui, &section[cursor..cursor + len]);
                });
                cursor += len;
            }
        }
        0xc8 | 0xc9 if section.len() >= 14 => {
            key_value(ui, "protocol_version", section[8]);
            key_value(ui, "num_channels_in_section", section[9]);
            let mut cursor = 10;
            for _ in 0..section[9] {
                if cursor + 32 > end {
                    break;
                }
                let channel = &section[cursor..cursor + 32];
                let name = channel[..14]
                    .chunks_exact(2)
                    .filter_map(|part| {
                        char::from_u32(u32::from(u16::from_be_bytes([part[0], part[1]])))
                    })
                    .filter(|character| *character != '\0')
                    .collect::<String>();
                let major = (u16::from(channel[14] & 0x0f) << 6) | u16::from(channel[15] >> 2);
                let minor = (u16::from(channel[15] & 0x03) << 8) | u16::from(channel[16]);
                let program = u16::from_be_bytes([channel[24], channel[25]]);
                let len = (usize::from(channel[30] & 0x03) << 8) | usize::from(channel[31]);
                cursor += 32;
                if cursor + len > end {
                    break;
                }
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!("virtual_channel {major}.{minor} {name}"))
                        .color(item_color(ui)),
                )
                .show(ui, |ui| {
                    key_value(ui, "short_name", &name);
                    key_value(ui, "major_channel_number", major);
                    key_value(ui, "minor_channel_number", minor);
                    key_value(ui, "modulation_mode", format!("0x{:02X}", channel[17]));
                    key_value(
                        ui,
                        "carrier_frequency",
                        u32::from_be_bytes([channel[18], channel[19], channel[20], channel[21]]),
                    );
                    key_value(
                        ui,
                        "channel_TSID",
                        u16::from_be_bytes([channel[22], channel[23]]),
                    );
                    key_value(ui, "program_number", program);
                    key_value(ui, "access_controlled", u8::from(channel[26] & 0x20 != 0));
                    key_value(ui, "hidden", u8::from(channel[26] & 0x10 != 0));
                    key_value(ui, "service_type", channel[27] & 0x3f);
                    key_value(
                        ui,
                        "source_id",
                        u16::from_be_bytes([channel[28], channel[29]]),
                    );
                    key_value(ui, "descriptors_length", len);
                    descriptor_list(ui, &section[cursor..cursor + len]);
                });
                cursor += len;
            }
        }
        _ => {
            if section.len() >= 8 && section[1] & 0x80 != 0 {
                ui.label(format!(
                    "Table extension: {}",
                    u16::from_be_bytes([section[3], section[4]])
                ));
            }
            ui.weak(format!(
                "{} section bytes; table-specific fields are not decoded",
                section.len()
            ));
        }
    }
}

pub fn psi_si_tree(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) {
    ui.heading("PSI / SI Tree");
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.add(
            egui::TextEdit::singleline(&mut state.psi_filter).hint_text("Table, PID or table ID"),
        );
    });
    ui.weak("Each branch summarizes a unique table section. Repeated TS packets and raw bytes are in Packets.");
    let needle = state.psi_filter.trim().to_ascii_lowercase();
    let mut shown = 0;
    for (&(pid, table_id), table) in &report.tables {
        let name = table_name(pid, table_id);
        if !needle.is_empty()
            && !name.to_ascii_lowercase().contains(&needle)
            && !format!("0x{pid:04x}").contains(&needle)
            && !format!("0x{table_id:02x}").contains(&needle)
        {
            continue;
        }
        shown += 1;
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("{name} | PID 0x{pid:04X} | table_id 0x{table_id:02X}"))
                .strong()
                .color(item_color(ui)),
        )
        .id_salt((pid, table_id))
        .show(ui, |ui| {
            key_value(ui, "occurrences", table.sections);
            key_value(ui, "unique_sections", table.instances.len());
            key_value(ui, "CRC_errors", table.crc_errors);
            for (&(extension, version, number), section) in &table.instances {
                egui::CollapsingHeader::new(
                    egui::RichText::new(format!(
                        "table_id_extension 0x{extension:04X} | version_number {version} | section_number {number}"
                    ))
                    .color(item_color(ui)),
                )
                .id_salt((pid, table_id, extension, version, number))
                .show(ui, |ui| {
                    key_value(ui, "section_bytes", section.len());
                    section_tree(ui, pid, table_id, section);
                });
            }
        });
    }
    if shown == 0 {
        ui.label("No observed tables match the filter.");
    }
}

fn packet_payload_offset(bytes: &[u8; 188]) -> Option<usize> {
    if bytes[3] & 0x10 == 0 {
        return None;
    }
    let offset = if bytes[3] & 0x20 != 0 {
        5 + usize::from(bytes[4])
    } else {
        4
    };
    (offset <= bytes.len()).then_some(offset)
}

fn clock_90khz(bytes: &[u8]) -> Option<u64> {
    (bytes.len() >= 5).then(|| {
        (u64::from((bytes[0] >> 1) & 0x07) << 30)
            | (u64::from(bytes[1]) << 22)
            | (u64::from(bytes[2] >> 1) << 15)
            | (u64::from(bytes[3]) << 7)
            | u64::from(bytes[4] >> 1)
    })
}

fn pcr_27mhz(bytes: &[u8]) -> Option<u64> {
    (bytes.len() >= 6).then(|| {
        let base = (u64::from(bytes[0]) << 25)
            | (u64::from(bytes[1]) << 17)
            | (u64::from(bytes[2]) << 9)
            | (u64::from(bytes[3]) << 1)
            | u64::from(bytes[4] >> 7);
        let extension = (u64::from(bytes[4] & 1) << 8) | u64::from(bytes[5]);
        base * 300 + extension
    })
}

fn packet_details(ui: &mut egui::Ui, packet: &tsan_analyzer::PacketRecord) {
    let bytes = &packet.bytes;
    let scrambling = match (bytes[3] >> 6) & 0x03 {
        0 => "not_scrambled",
        1 => "reserved",
        2 => "even_key",
        _ => "odd_key",
    };
    let adaptation_control = (bytes[3] >> 4) & 0x03;
    egui::Grid::new("packet-header-fields")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (item, value) in [
                ("sync_byte", format!("0x{:02X}", bytes[0])),
                (
                    "transport_error_indicator",
                    u8::from(bytes[1] & 0x80 != 0).to_string(),
                ),
                (
                    "payload_unit_start_indicator",
                    u8::from(bytes[1] & 0x40 != 0).to_string(),
                ),
                (
                    "transport_priority",
                    u8::from(bytes[1] & 0x20 != 0).to_string(),
                ),
                ("PID", format!("0x{:04X} ({})", packet.pid, packet.pid)),
                (
                    "transport_scrambling_control",
                    format!("0b{:02b} ({scrambling})", (bytes[3] >> 6) & 0x03),
                ),
                (
                    "adaptation_field_control",
                    format!("0b{adaptation_control:02b}"),
                ),
                ("continuity_counter", packet.continuity_counter.to_string()),
            ] {
                ui.label(
                    egui::RichText::new(item)
                        .monospace()
                        .strong()
                        .color(item_color(ui)),
                );
                value_cell(ui, value);
                ui.end_row();
            }
        });
    if packet.adaptation {
        let length = usize::from(bytes[4]).min(183);
        egui::CollapsingHeader::new(
            egui::RichText::new("adaptation_field")
                .strong()
                .color(item_color(ui)),
        )
        .default_open(true)
        .show(ui, |ui| {
            key_value(ui, "adaptation_field_length", length);
            if length == 0 {
                return;
            }
            let flags = bytes[5];
            for (item, mask) in [
                ("discontinuity_indicator", 0x80),
                ("random_access_indicator", 0x40),
                ("elementary_stream_priority_indicator", 0x20),
                ("PCR_flag", 0x10),
                ("OPCR_flag", 0x08),
                ("splicing_point_flag", 0x04),
                ("transport_private_data_flag", 0x02),
                ("adaptation_field_extension_flag", 0x01),
            ] {
                key_value(ui, item, u8::from(flags & mask != 0));
            }
            let end = (5 + length).min(bytes.len());
            let mut cursor = 6;
            if flags & 0x10 != 0 && cursor + 6 <= end {
                if let Some(value) = pcr_27mhz(&bytes[cursor..cursor + 6]) {
                    key_value(
                        ui,
                        "program_clock_reference",
                        format!("{value} ticks | {:.9} s", value as f64 / 27_000_000.0),
                    );
                }
                cursor += 6;
            }
            if flags & 0x08 != 0 && cursor + 6 <= end {
                if let Some(value) = pcr_27mhz(&bytes[cursor..cursor + 6]) {
                    key_value(
                        ui,
                        "original_program_clock_reference",
                        format!("{value} ticks | {:.9} s", value as f64 / 27_000_000.0),
                    );
                }
                cursor += 6;
            }
            if flags & 0x04 != 0 && cursor < end {
                key_value(ui, "splice_countdown", bytes[cursor] as i8);
                cursor += 1;
            }
            if flags & 0x02 != 0 && cursor < end {
                let private_length = usize::from(bytes[cursor]);
                cursor += 1;
                let private_end = (cursor + private_length).min(end);
                key_value(
                    ui,
                    "transport_private_data",
                    format!("{:02X?}", &bytes[cursor..private_end]),
                );
                cursor = private_end;
            }
            if flags & 0x01 != 0 && cursor < end {
                let extension_length = usize::from(bytes[cursor]);
                let extension_end = (cursor + 1 + extension_length).min(end);
                key_value(
                    ui,
                    "adaptation_field_extension",
                    format!("{:02X?}", &bytes[cursor + 1..extension_end]),
                );
            }
        });
    }
    if let Some(offset) = packet_payload_offset(bytes) {
        egui::CollapsingHeader::new(
            egui::RichText::new("payload")
                .strong()
                .color(item_color(ui)),
        )
        .default_open(true)
        .show(ui, |ui| {
            key_value(ui, "payload_offset", offset);
            key_value(ui, "payload_length", bytes.len().saturating_sub(offset));
            if bytes.get(offset..offset + 3) == Some(&[0x00, 0x00, 0x01])
                && offset + 6 <= bytes.len()
            {
                let stream_id = bytes[offset + 3];
                let pes_length = u16::from_be_bytes([bytes[offset + 4], bytes[offset + 5]]);
                key_value(ui, "packet_start_code_prefix", "0x000001");
                key_value(ui, "stream_id", format!("0x{stream_id:02X}"));
                key_value(ui, "PES_packet_length", pes_length);
                if offset + 9 <= bytes.len() && bytes[offset + 6] & 0xC0 == 0x80 {
                    let pts_dts = (bytes[offset + 7] >> 6) & 0x03;
                    let header_length = usize::from(bytes[offset + 8]);
                    key_value(ui, "PES_header_data_length", header_length);
                    let optional = offset + 9;
                    if pts_dts & 0x02 != 0 {
                        if let Some(value) = clock_90khz(&bytes[optional..]) {
                            key_value(
                                ui,
                                "PTS",
                                format!("{value} ticks | {:.6} s", value as f64 / 90_000.0),
                            );
                        }
                    }
                    if pts_dts == 0x03 {
                        if let Some(value) = clock_90khz(&bytes[optional + 5..]) {
                            key_value(
                                ui,
                                "DTS",
                                format!("{value} ticks | {:.6} s", value as f64 / 90_000.0),
                            );
                        }
                    }
                }
            } else if packet.payload_unit_start && offset < bytes.len() {
                let pointer = usize::from(bytes[offset]);
                let section_offset = offset + 1 + pointer;
                key_value(ui, "pointer_field", pointer);
                if section_offset + 3 <= bytes.len() {
                    let table_id = bytes[section_offset];
                    let section_length = (usize::from(bytes[section_offset + 1] & 0x0f) << 8)
                        | usize::from(bytes[section_offset + 2]);
                    key_value(
                        ui,
                        "table_id",
                        format!("0x{table_id:02X} ({})", table_name(packet.pid, table_id)),
                    );
                    key_value(ui, "section_offset", section_offset);
                    key_value(ui, "section_length", section_length);
                }
            }
            let payload_end = (offset + 32).min(bytes.len());
            key_value(
                ui,
                "payload_prefix",
                format!("{:02X?}", &bytes[offset..payload_end]),
            );
        });
    } else {
        ui.weak("No payload in this packet.");
    }
}

fn packet_table(ui: &mut egui::Ui, window: &PacketWindow, selected: &mut Option<u64>, height: f32) {
    let min_width = 650.0;
    ArrowScrollArea::both()
        .id_salt("packet-list-scroll")
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width().max(min_width));
            let widths = [72.0, 105.0, 78.0, 54.0, 48.0, 125.0];
            ui.horizontal(|ui| {
                for (title, width) in ["#", "Offset", "PID", "PUSI", "CC", "PCR"]
                    .into_iter()
                    .zip(widths)
                {
                    ui.add_sized(
                        [width, 22.0],
                        egui::Label::new(egui::RichText::new(title).strong().color(item_color(ui))),
                    );
                }
                ui.label(egui::RichText::new("Flags").strong().color(item_color(ui)));
            });
            ui.separator();
            for (row, packet) in window.packets.iter().enumerate() {
                let fill = if row % 2 == 0 {
                    ui.visuals().faint_bg_color
                } else {
                    egui::Color32::TRANSPARENT
                };
                egui::Frame::new().fill(fill).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized(
                                [widths[0], 22.0],
                                egui::Button::selectable(
                                    *selected == Some(packet.index),
                                    egui::RichText::new(packet.index.to_string())
                                        .monospace()
                                        .color(value_color(ui)),
                                ),
                            )
                            .clicked()
                        {
                            *selected = Some(packet.index);
                        }
                        for (value, width) in [
                            (format!("0x{:X}", packet.offset), widths[1]),
                            (format!("0x{:04X}", packet.pid), widths[2]),
                            (
                                if packet.payload_unit_start { "1" } else { "0" }.to_owned(),
                                widths[3],
                            ),
                            (packet.continuity_counter.to_string(), widths[4]),
                            (
                                packet.pcr_27mhz.map_or_else(String::new, |pcr| {
                                    format!("{:.6}", pcr as f64 / 27_000_000.0)
                                }),
                                widths[5],
                            ),
                        ] {
                            ui.add_sized(
                                [width, 22.0],
                                egui::Label::new(
                                    egui::RichText::new(value)
                                        .monospace()
                                        .color(value_color(ui)),
                                ),
                            );
                        }
                        let flags = format!(
                            "{}{}{}{}",
                            if packet.transport_error { "TEI " } else { "" },
                            if packet.scrambled { "SCR " } else { "" },
                            if packet.random_access { "RAP " } else { "" },
                            if packet.discontinuity { "DISC" } else { "" }
                        );
                        ui.label(
                            egui::RichText::new(flags)
                                .monospace()
                                .color(value_color(ui)),
                        );
                    });
                });
            }
        });
}

fn selected_packet_details(ui: &mut egui::Ui, window: &PacketWindow, selected: Option<u64>) {
    let Some(packet) = window
        .packets
        .iter()
        .find(|packet| Some(packet.index) == selected)
    else {
        ui.weak("Select a packet to inspect all fields.");
        return;
    };
    ui.heading(format!(
        "Packet {} | PID 0x{:04X}",
        packet.index, packet.pid
    ));
    key_value(ui, "file_offset", format!("0x{:X}", packet.offset));
    packet_details(ui, packet);
    ui.separator();
    ui.label(
        egui::RichText::new("Complete 188-byte packet")
            .strong()
            .color(item_color(ui)),
    );
    let mut hex = String::new();
    for (index, byte) in packet.bytes.iter().enumerate() {
        if index % 16 == 0 {
            let _ = write!(hex, "\n{index:04X}: ");
        }
        let _ = write!(hex, "{byte:02X} ");
    }
    let mut text = hex.as_str();
    ArrowScrollArea::horizontal().show(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut text)
                .font(egui::TextStyle::Monospace)
                .text_color(value_color(ui))
                .desired_width(f32::INFINITY),
        );
    });
}

pub fn packets(ui: &mut egui::Ui, path: &Path, report: &AnalysisReport, state: &mut ViewState) {
    ui.heading("Packets");
    key_value(
        ui,
        "transport_stream",
        format!("{} packets | {} PIDs", report.packets, report.pids.len()),
    );
    ui.horizontal_wrapped(|ui| {
        ui.label("PID:");
        ui.add(
            egui::TextEdit::singleline(&mut state.packet_pid_filter)
                .hint_text("all / 0x0101 / 257")
                .desired_width(140.0),
        );
        ui.label("From packet:");
        ui.add(
            egui::DragValue::new(&mut state.packet_start)
                .range(0..=report.packets.saturating_sub(1)),
        );
        if ui.button("Previous").clicked() {
            state.packet_start = state.packet_start.saturating_sub(256);
        }
    });
    let filter = match pid_filter(&state.packet_pid_filter) {
        Ok(filter) => filter,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
    };
    let key_changed = state
        .packet_cache
        .as_ref()
        .is_none_or(|(cached_path, start, pid, _)| {
            cached_path != path || *start != state.packet_start || *pid != filter
        });
    if key_changed {
        match read_packet_window(path, state.packet_start, 256, filter) {
            Ok(window) => {
                state.packet_cache = Some((path.to_path_buf(), state.packet_start, filter, window))
            }
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                return;
            }
        }
    }
    let Some((_, _, _, window)) = &state.packet_cache else {
        return;
    };
    let next_index = window.next_index;
    if ui.button("Next").clicked() {
        state.packet_start = next_index.min(report.packets.saturating_sub(1));
    }
    let mut selected = state.selected_packet;
    if selected.is_none_or(|index| !window.packets.iter().any(|packet| packet.index == index)) {
        selected = window.packets.first().map(|packet| packet.index);
    }

    let available_height = (ui.clip_rect().bottom() - ui.cursor().top()).clamp(480.0, 900.0);
    if ui.available_width() >= 760.0 {
        let total_width = ui.available_width();
        let handle_width = 10.0;
        let content_width = (total_width - handle_width).max(1.0);
        let min_left = 260.0_f32.min(content_width * 0.45);
        let min_right = 320.0_f32.min(content_width * 0.45);
        let minimum_ratio = min_left / content_width;
        let maximum_ratio = 1.0 - min_right / content_width;
        state.packet_split_ratio = state
            .packet_split_ratio
            .clamp(minimum_ratio, maximum_ratio.max(minimum_ratio));
        let left_width = content_width * state.packet_split_ratio;
        let right_width = content_width - left_width;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.allocate_ui_with_layout(
                egui::vec2(left_width, available_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(left_width);
                    ui.heading("Packet list");
                    packet_table(ui, window, &mut selected, available_height - 45.0);
                },
            );
            let handle_response = ResizeHandle::horizontal(available_height)
                .thickness(handle_width)
                .hover_text("Drag to resize the packet list and packet details")
                .show(ui);
            if handle_response.dragged() {
                let delta = ui.ctx().input(|input| input.pointer.delta().x);
                state.packet_split_ratio = ((left_width + delta) / content_width)
                    .clamp(minimum_ratio, maximum_ratio.max(minimum_ratio));
            }
            ui.allocate_ui_with_layout(
                egui::vec2(right_width, available_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(right_width);
                    ArrowScrollArea::both()
                        .id_salt("packet-detail-scroll")
                        .max_height(available_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            selected_packet_details(ui, window, selected);
                        });
                },
            );
        });
    } else {
        ui.heading("Packet list");
        packet_table(
            ui,
            window,
            &mut selected,
            (available_height * 0.52).max(280.0),
        );
        ui.separator();
        ArrowScrollArea::both()
            .id_salt("packet-detail-scroll")
            .max_height((available_height * 0.48).max(280.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                selected_packet_details(ui, window, selected);
            });
    }
    state.selected_packet = selected;
}

fn tr101290_summary_panel(
    ui: &mut egui::Ui,
    summary: &tsan_analyzer::ComplianceReport,
    needle: &str,
    panel_width: f32,
) {
    let indicator_font = egui::TextStyle::Body.resolve(ui.style());
    let indicator_width = ui.fonts_mut(|fonts| {
        summary
            .indicators
            .iter()
            .map(|indicator| {
                fonts
                    .layout_no_wrap(
                        indicator.name.to_owned(),
                        indicator_font.clone(),
                        egui::Color32::WHITE,
                    )
                    .size()
                    .x
            })
            .fold(180.0_f32, f32::max)
            + 16.0
    });
    let table_width = indicator_width + 250.0;
    let frame_content_width = (panel_width - 34.0).max(table_width);
    let groups = summary
        .indicators
        .iter()
        .map(|item| item.group)
        .collect::<std::collections::BTreeSet<_>>();

    egui::Frame::group(ui.style())
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_min_width(frame_content_width);
            for group in groups {
                egui::CollapsingHeader::new(group)
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new(("compliance-group", group))
                            .striped(true)
                            .spacing(egui::vec2(16.0, 6.0))
                            .show(ui, |ui| {
                                ui.add_sized(
                                    [indicator_width, 20.0],
                                    egui::Label::new(
                                        egui::RichText::new("Indicator")
                                            .strong()
                                            .color(item_color(ui)),
                                    ),
                                );
                                for heading in ["Status", "Observed"] {
                                    ui.label(
                                        egui::RichText::new(heading).strong().color(item_color(ui)),
                                    );
                                }
                                ui.end_row();
                                for indicator in
                                    summary.indicators.iter().filter(|item| item.group == group)
                                {
                                    if !needle.is_empty()
                                        && !indicator.name.to_ascii_lowercase().contains(needle)
                                        && !indicator.note.to_ascii_lowercase().contains(needle)
                                    {
                                        continue;
                                    }
                                    ui.add_sized(
                                        [indicator_width, 20.0],
                                        egui::Label::new(
                                            egui::RichText::new(indicator.name)
                                                .color(item_color(ui))
                                                .strong(),
                                        ),
                                    )
                                    .on_hover_text(format!(
                                        "{}\n{}",
                                        indicator.name, indicator.note
                                    ));
                                    let status_color = match indicator.status {
                                        tsan_analyzer::ComplianceStatus::Fail => {
                                            ui.visuals().error_fg_color
                                        }
                                        _ => value_color(ui),
                                    };
                                    ui.colored_label(status_color, indicator.status.label());
                                    match indicator.observed {
                                        Some(count) => {
                                            value_cell(ui, count.to_string());
                                        }
                                        None => {
                                            let _ = ui.weak("\u{2014}");
                                        }
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                ui.add_space(6.0);
            }
        });
    ui.add(
        egui::Label::new(
            egui::RichText::new(
                "Pass and Fail are shown only for measured checks. Not observed, not applicable and not implemented remain distinct states.",
            )
            .weak(),
        )
        .wrap(),
    );
}

fn compliance_event_text(report: &AnalysisReport, event: &tsan_analyzer::TrEvent) -> String {
    let offset = report.packet_offset(event.packet_index);
    let location = if event.exact_packet {
        format!("{} / 0x{offset:X}", event.packet_index)
    } else {
        format!("~{} / ~0x{offset:X}", event.packet_index)
    };
    format!(
        "{location}  0x{:04X}  {}  {}",
        event.pid, event.indicator, event.detail
    )
}

fn tr101290_events_panel(
    ui: &mut egui::Ui,
    report: &AnalysisReport,
    summary: &tsan_analyzer::ComplianceReport,
    needle: &str,
    panel_height: f32,
) -> Option<u64> {
    ui.heading(format!("Events ({})", summary.events.len()));
    ui.add(
        egui::Label::new(
            egui::RichText::new("Exact packet offsets are links to the complete 188-byte packet. A leading ~ marks an estimated analysis position for an absence or duration event.")
                .weak(),
        )
        .wrap(),
    );
    ui.horizontal(|ui| {
        for heading in [
            "Packet / byte offset",
            "PID",
            "Indicator and observed detail",
        ] {
            ui.label(egui::RichText::new(heading).strong().color(item_color(ui)));
        }
    });

    let visible = summary
        .events
        .iter()
        .filter(|event| {
            needle.is_empty()
                || event.indicator.to_ascii_lowercase().contains(needle)
                || event.detail.to_ascii_lowercase().contains(needle)
                || format!("0x{:04x}", event.pid).contains(needle)
        })
        .collect::<Vec<_>>();
    let longest_chars = visible
        .iter()
        .map(|event| compliance_event_text(report, event).chars().count())
        .max()
        .unwrap_or(0);
    let character_width = ui.text_style_height(&egui::TextStyle::Monospace) * 0.62;
    let stable_content_width =
        (longest_chars as f32 * character_width + 32.0).max(ui.available_width());
    let mut jump_to_packet = None;
    ArrowScrollArea::both()
        .id_salt("compliance-event-scroll")
        .max_height((panel_height - 90.0).max(120.0))
        .auto_shrink([false, false])
        .show_rows(ui, 25.0, visible.len(), |ui, range| {
            ui.set_min_width(stable_content_width);
            for event in &visible[range] {
                ui.horizontal(|ui| {
                    let offset = report.packet_offset(event.packet_index);
                    if event.exact_packet {
                        if ui
                            .link(format!("{} / 0x{offset:X}", event.packet_index))
                            .clicked()
                        {
                            jump_to_packet = Some(event.packet_index);
                        }
                    } else {
                        ui.monospace(format!("~{} / ~0x{offset:X}", event.packet_index))
                            .on_hover_text(
                                "Estimated analysis position; there is no single offending packet.",
                            );
                    }
                    ui.label(
                        egui::RichText::new(format!("0x{:04X}", event.pid))
                            .monospace()
                            .color(value_color(ui)),
                    );
                    ui.label(
                        egui::RichText::new(event.indicator)
                            .strong()
                            .color(item_color(ui)),
                    );
                    ui.label(egui::RichText::new(&event.detail).color(value_color(ui)));
                });
            }
        });
    jump_to_packet
}

fn tr101290_profile_menu(ui: &mut egui::Ui, profile: &mut tsan_analyzer::ComplianceProfile) {
    ui.menu_button(profile.label(), |ui| {
        ui.set_min_width(420.0);
        for family in tsan_analyzer::ComplianceFamily::ALL {
            let family_profiles = tsan_analyzer::ComplianceProfile::ALL
                .into_iter()
                .filter(|candidate| candidate.hierarchy().family == family)
                .collect::<Vec<_>>();
            if family_profiles.is_empty() {
                continue;
            }
            ui.menu_button(family.label(), |ui| {
                let mut systems = Vec::new();
                for candidate in &family_profiles {
                    let system = candidate.hierarchy().system;
                    if systems.contains(&system) {
                        continue;
                    }
                    systems.push(system);
                    ui.menu_button(system.label(), |ui| {
                        for candidate in family_profiles
                            .iter()
                            .copied()
                            .filter(|candidate| candidate.hierarchy().system == system)
                        {
                            if ui
                                .selectable_value(profile, candidate, candidate.variant_label())
                                .clicked()
                            {
                                ui.close();
                            }
                        }
                    });
                }
            });
        }
    });
}

pub fn tr101290(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) -> Option<u64> {
    ui.heading("TR 101 290");
    let suggested = tsan_analyzer::ComplianceProfile::suggested(report);
    let profile = state.tr101290_profile.get_or_insert(suggested);
    ui.horizontal_wrapped(|ui| {
        ui.label("Compliance hierarchy:");
        tr101290_profile_menu(ui, profile);
        ui.label("Filter:");
        ui.add(
            egui::TextEdit::singleline(&mut state.tr101290_filter)
                .hint_text("Indicator, PID or event detail"),
        );
    });
    let summary = tsan_analyzer::tr101290_report(report, *profile);
    ui.add(
        egui::Label::new(
            egui::RichText::new(format!(
                "Applied hierarchy: {} › {} › {} › {}. Suggested from detected stream signalling: {:?}.",
                summary.hierarchy.family.label(),
                summary.hierarchy.system.label(),
                summary.hierarchy.signaling,
                summary.hierarchy.delivery,
                report.standard
            ))
            .strong(),
        )
        .wrap(),
    );
    let needle = state.tr101290_filter.trim().to_ascii_lowercase();
    let available_width = ui.available_width().max(1.0);
    let available_height = (ui.clip_rect().bottom() - ui.cursor().top()).clamp(520.0, 900.0);
    let handle_height = 10.0;
    let content_height = (available_height - handle_height).max(1.0);
    let min_summary = 210.0_f32.min(content_height * 0.45);
    let min_events = 190.0_f32.min(content_height * 0.45);
    let minimum_ratio = min_summary / content_height;
    let maximum_ratio = 1.0 - min_events / content_height;
    state.tr101290_summary_ratio = state
        .tr101290_summary_ratio
        .clamp(minimum_ratio, maximum_ratio.max(minimum_ratio));
    let summary_height = content_height * state.tr101290_summary_ratio;
    let events_height = content_height - summary_height;
    let mut jump_to_packet = None;

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), summary_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ArrowScrollArea::both()
                    .id_salt("compliance-summary-scroll")
                    .max_width(available_width)
                    .max_height(summary_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        tr101290_summary_panel(ui, &summary, &needle, available_width);
                    });
            },
        );
        let handle_response = ResizeHandle::vertical(ui.available_width())
            .thickness(handle_height)
            .hover_text("Drag to resize the compliance summary and Events")
            .show(ui);
        if handle_response.dragged() {
            let delta = ui.ctx().input(|input| input.pointer.delta().y);
            state.tr101290_summary_ratio = ((summary_height + delta) / content_height)
                .clamp(minimum_ratio, maximum_ratio.max(minimum_ratio));
        }
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), events_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                jump_to_packet =
                    tr101290_events_panel(ui, report, &summary, &needle, events_height);
            },
        );
    });
    jump_to_packet
}

fn graph_range_controls(ui: &mut egui::Ui, state: &mut ViewState, plot_id: &str) {
    let mut reset_filter = false;
    {
        let view = state.graph_views.entry(plot_id.to_owned()).or_default();
        ui.horizontal_wrapped(|ui| {
            ui.label("X range:");
            ui.add(egui::Slider::new(&mut view.x_from_percent, 0.0..=99.0).text("from %"));
            ui.add(egui::Slider::new(&mut view.x_to_percent, 1.0..=100.0).text("to %"));
            if ui.button("Reset view").clicked() {
                *view = GraphView::default();
            }
            reset_filter = ui
                .button("Reset Filter")
                .on_hover_text("Show every available PID, service and clock series again.")
                .clicked();
            ui.checkbox(&mut state.graph_show_legend, "Legend");
        });
        ui.add(
            egui::Label::new(
                egui::RichText::new("Left drag: rectangle zoom X/Y. Right drag: pan. Wheel or middle-button vertical drag: zoom X/Y around cursor. Zoom stops at the full data extent and at a minimum of eight sample intervals.")
                    .weak(),
            )
            .wrap(),
        );
        if view.x_from_percent >= view.x_to_percent {
            view.x_to_percent = (view.x_from_percent + 1.0).min(100.0);
        }
    }
    if reset_filter {
        state.graph_pid_filter.clear();
        state.graph_series_enabled.clear();
        state.graph_service = None;
        state.show_pcr = true;
        state.show_pts = true;
        state.show_dts = true;
    }
}

#[derive(Clone, Copy)]
enum PlotMarker {
    Circle,
    Triangle,
    Square,
    Diamond,
    Cross,
    Plus,
}

impl PlotMarker {
    fn for_index(index: usize) -> Self {
        match index % 6 {
            0 => Self::Circle,
            1 => Self::Triangle,
            2 => Self::Square,
            3 => Self::Diamond,
            4 => Self::Cross,
            _ => Self::Plus,
        }
    }
}

struct Series {
    name: String,
    color: egui::Color32,
    marker: PlotMarker,
    points: Vec<(f64, f64)>,
    details: Vec<String>,
    connected: bool,
    fill_baseline: Option<Vec<f64>>,
}

fn paint_plot_marker(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    marker: PlotMarker,
    color: egui::Color32,
) {
    let stroke = egui::Stroke::new(1.8, color);
    match marker {
        PlotMarker::Circle => {
            painter.circle_filled(center, radius, color);
        }
        PlotMarker::Triangle => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(center.x, center.y - radius),
                    egui::pos2(center.x + radius, center.y + radius),
                    egui::pos2(center.x - radius, center.y + radius),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        PlotMarker::Square => {
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::vec2(radius * 1.8, radius * 1.8)),
                0.0,
                color,
            );
        }
        PlotMarker::Diamond => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(center.x, center.y - radius),
                    egui::pos2(center.x + radius, center.y),
                    egui::pos2(center.x, center.y + radius),
                    egui::pos2(center.x - radius, center.y),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        PlotMarker::Cross => {
            painter.line_segment(
                [
                    egui::pos2(center.x - radius, center.y - radius),
                    egui::pos2(center.x + radius, center.y + radius),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - radius, center.y + radius),
                    egui::pos2(center.x + radius, center.y - radius),
                ],
                stroke,
            );
        }
        PlotMarker::Plus => {
            painter.hline(center.x - radius..=center.x + radius, center.y, stroke);
            painter.vline(center.x, center.y - radius..=center.y + radius, stroke);
        }
    }
}

fn plot_marker_label(ui: &mut egui::Ui, marker: PlotMarker, color: egui::Color32) {
    let size = ui.text_style_height(&egui::TextStyle::Body);
    let (rectangle, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    paint_plot_marker(
        ui.painter(),
        rectangle.center(),
        (size * 0.28).max(3.0),
        marker,
        color,
    );
}

const MIN_VISIBLE_GRAPH_SAMPLES: f64 = 8.0;

fn normalized_axis_range(low: f64, high: f64) -> (f64, f64) {
    if !low.is_finite() || !high.is_finite() {
        return (0.0, 1.0);
    }
    if high > low {
        return (low, high);
    }
    let padding = low.abs().mul_add(1.0e-6, 1.0e-6).max(1.0e-6);
    (low - padding, high + padding)
}

fn bounded_axis_window(
    low: f64,
    high: f64,
    full_low: f64,
    full_high: f64,
    minimum_span: f64,
) -> (f64, f64) {
    let (full_low, full_high) = normalized_axis_range(full_low, full_high);
    let full_span = full_high - full_low;
    let minimum_span = minimum_span.clamp(f64::EPSILON, full_span);
    if !low.is_finite() || !high.is_finite() {
        return (full_low, full_high);
    }
    let span = (high - low).abs().clamp(minimum_span, full_span);
    let center = (low + high) * 0.5;
    let next_low = (center - span * 0.5).clamp(full_low, full_high - span);
    (next_low, next_low + span)
}

fn normalize_graph_view(view: &mut GraphView, minimum_x_span: f64, full_y: (f64, f64)) {
    let (x_from, x_to) = bounded_axis_window(
        view.x_from_percent,
        view.x_to_percent,
        0.0,
        100.0,
        minimum_x_span,
    );
    view.x_from_percent = x_from;
    view.x_to_percent = x_to;
    if let Some((low, high)) = view.y_bounds {
        let full_y_span = full_y.1 - full_y.0;
        view.y_bounds = Some(bounded_axis_window(
            low,
            high,
            full_y.0,
            full_y.1,
            full_y_span * minimum_x_span / 100.0,
        ));
    }
}

fn zoom_view(
    view: &mut GraphView,
    requested_factor: f64,
    x_anchor: f64,
    y_anchor: f64,
    current_y: (f64, f64),
    full_y: (f64, f64),
    minimum_x_span: f64,
) {
    normalize_graph_view(view, minimum_x_span, full_y);
    let old_x_span = view.x_to_percent - view.x_from_percent;
    let (old_y_low, old_y_high) = view.y_bounds.unwrap_or(current_y);
    let old_y_span = old_y_high - old_y_low;
    let full_y_span = full_y.1 - full_y.0;
    let minimum_y_span = full_y_span * minimum_x_span / 100.0;

    let x_factor = if requested_factor < 1.0 {
        requested_factor.max(minimum_x_span / old_x_span).min(1.0)
    } else {
        requested_factor.min(100.0 / old_x_span).max(1.0)
    };
    let y_factor = if requested_factor < 1.0 {
        requested_factor.max(minimum_y_span / old_y_span).min(1.0)
    } else {
        requested_factor.min(full_y_span / old_y_span).max(1.0)
    };

    let x_anchor = x_anchor.clamp(0.0, 1.0);
    let x_span = old_x_span * x_factor;
    let x_pivot = view.x_from_percent + old_x_span * x_anchor;
    let (x_from, x_to) = bounded_axis_window(
        x_pivot - x_span * x_anchor,
        x_pivot + x_span * (1.0 - x_anchor),
        0.0,
        100.0,
        minimum_x_span,
    );
    view.x_from_percent = x_from;
    view.x_to_percent = x_to;

    let y_anchor = y_anchor.clamp(0.0, 1.0);
    let y_span = old_y_span * y_factor;
    let y_pivot = old_y_high - old_y_span * y_anchor;
    let y_bounds = bounded_axis_window(
        y_pivot - y_span * (1.0 - y_anchor),
        y_pivot + y_span * y_anchor,
        full_y.0,
        full_y.1,
        minimum_y_span,
    );
    let epsilon = full_y_span.abs().mul_add(1.0e-9, 1.0e-9);
    let full_x_visible = view.x_from_percent <= 1.0e-9 && view.x_to_percent >= 100.0 - 1.0e-9;
    let full_y_visible =
        (y_bounds.0 - full_y.0).abs() <= epsilon && (y_bounds.1 - full_y.1).abs() <= epsilon;
    view.y_bounds = if requested_factor >= 1.0 && full_x_visible && full_y_visible {
        None
    } else {
        Some(y_bounds)
    };
}

fn axis_tick(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 1_000_000.0 || (magnitude > 0.0 && magnitude < 0.001) {
        format!("{value:.3e}")
    } else {
        format!("{value:.3}")
    }
}

fn elapsed_time_tick(seconds: f64) -> String {
    let sign = if seconds < 0.0 { "-" } else { "" };
    let seconds = seconds.abs();
    let hours = (seconds / 3600.0).floor() as u64;
    let minutes = ((seconds % 3600.0) / 60.0).floor() as u64;
    let remaining = seconds % 60.0;
    if hours > 0 {
        format!("{sign}{hours:02}:{minutes:02}:{remaining:06.3}")
    } else {
        format!("{sign}{minutes:02}:{remaining:06.3}")
    }
}

fn x_axis_tick(x_label: &str, value: f64) -> String {
    if x_label.contains("time") || x_label.contains("Time") || x_label.contains("seconds") {
        elapsed_time_tick(value)
    } else {
        axis_tick(value)
    }
}

fn series_y(line: &Series, index: usize) -> f64 {
    line.points[index].1
        + line
            .fill_baseline
            .as_ref()
            .and_then(|baseline| baseline.get(index))
            .copied()
            .unwrap_or(0.0)
}

fn plot(
    ui: &mut egui::Ui,
    plot_id: &str,
    title: &str,
    x_label: &str,
    y_label: &str,
    series: &[Series],
    state: &mut ViewState,
) {
    ui.heading(title);
    let scroll_gutter = ui.spacing().scroll.bar_width + 18.0;
    let width = (ui.available_width() - scroll_gutter).max(220.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, 500.0), egui::Sense::click_and_drag());
    let all = series
        .iter()
        .flat_map(|line| {
            line.points
                .iter()
                .enumerate()
                .map(|(index, point)| (point.0, series_y(line, index)))
        })
        .collect::<Vec<_>>();
    if all.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No samples for this filter",
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        return;
    }

    let full_min_x = all
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let full_max_x = all
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let full_span = (full_max_x - full_min_x).max(1.0);
    let baseline_min = series
        .iter()
        .filter_map(|line| line.fill_baseline.as_ref())
        .flatten()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let full_min_y = all
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min)
        .min(baseline_min);
    let full_y = normalized_axis_range(
        full_min_y,
        all.iter()
            .map(|point| point.1)
            .fold(f64::NEG_INFINITY, f64::max),
    );
    let longest_series = series
        .iter()
        .map(|line| line.points.len())
        .max()
        .unwrap_or(1)
        .saturating_sub(1)
        .max(1);
    let minimum_x_span =
        (100.0 * MIN_VISIBLE_GRAPH_SAMPLES / longest_series as f64).clamp(0.05, 5.0);
    normalize_graph_view(
        state.graph_views.entry(plot_id.to_owned()).or_default(),
        minimum_x_span,
        full_y,
    );
    let view = state.graph_views[plot_id];
    let min_x = full_min_x + full_span * view.x_from_percent / 100.0;
    let max_x = full_min_x + full_span * view.x_to_percent / 100.0;
    let visible = all
        .iter()
        .copied()
        .filter(|point| (min_x..=max_x).contains(&point.0))
        .collect::<Vec<_>>();
    if visible.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No samples in the selected range",
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        return;
    }

    let visible_baseline_min = series
        .iter()
        .filter_map(|line| line.fill_baseline.as_ref().map(|baseline| (line, baseline)))
        .flat_map(|(line, baseline)| line.points.iter().zip(baseline))
        .filter(|(point, _)| (min_x..=max_x).contains(&point.0))
        .map(|(_, baseline)| *baseline)
        .fold(f64::INFINITY, f64::min);
    let auto_y = (
        visible
            .iter()
            .map(|point| point.1)
            .fold(f64::INFINITY, f64::min)
            .min(visible_baseline_min),
        visible
            .iter()
            .map(|point| point.1)
            .fold(f64::NEG_INFINITY, f64::max),
    );
    let (auto_min_y, auto_max_y) = normalized_axis_range(auto_y.0, auto_y.1);
    let (min_y, max_y) = view.y_bounds.map_or((auto_min_y, auto_max_y), |bounds| {
        normalized_axis_range(bounds.0, bounds.1)
    });
    let axis_font = egui::TextStyle::Body.resolve(ui.style());
    let axis_color = item_color(ui);
    let y_label_width = [axis_tick(min_y), axis_tick(max_y)]
        .iter()
        .map(|label| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(label.clone(), axis_font.clone(), axis_color)
                    .size()
                    .x
            })
        })
        .fold(0.0_f32, f32::max);
    let left_margin = (y_label_width + 18.0).clamp(68.0, 150.0);
    let top_margin = axis_font.size + 16.0;
    let bottom_margin = axis_font.size * 2.8 + 14.0;
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + left_margin, rect.top() + top_margin),
        egui::pos2(rect.right() - 14.0, rect.bottom() - bottom_margin),
    );
    let grid = if ui.visuals().dark_mode {
        egui::Color32::from_gray(90)
    } else {
        egui::Color32::from_gray(190)
    };
    let endpoint_width = [x_axis_tick(x_label, min_x), x_axis_tick(x_label, max_x)]
        .iter()
        .map(|label| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(label.clone(), axis_font.clone(), axis_color)
                    .size()
                    .x
            })
        })
        .fold(0.0_f32, f32::max);
    let x_steps = ((plot.width() / (endpoint_width + 28.0)).floor() as usize).clamp(2, 8);
    let y_steps = ((plot.height() / (axis_font.size * 3.6)).floor() as usize).clamp(2, 6);
    for step in 0..=x_steps {
        let x = plot.left() + plot.width() * step as f32 / x_steps as f32;
        ui.painter()
            .vline(x, plot.y_range(), egui::Stroke::new(1.0, grid));
        ui.painter().text(
            egui::pos2(x, plot.bottom() + 7.0),
            egui::Align2::CENTER_TOP,
            x_axis_tick(
                x_label,
                min_x + (max_x - min_x) * step as f64 / x_steps as f64,
            ),
            axis_font.clone(),
            axis_color,
        );
    }
    for step in 0..=y_steps {
        let y = plot.top() + plot.height() * step as f32 / y_steps as f32;
        ui.painter()
            .hline(plot.x_range(), y, egui::Stroke::new(1.0, grid));
        ui.painter().text(
            egui::pos2(plot.left() - 8.0, y),
            egui::Align2::RIGHT_CENTER,
            axis_tick(max_y - (max_y - min_y) * step as f64 / y_steps as f64),
            axis_font.clone(),
            axis_color,
        );
    }

    let position = |x: f64, y: f64| {
        egui::pos2(
            plot.left() + ((x - min_x) / (max_x - min_x)) as f32 * plot.width(),
            plot.bottom() - ((y - min_y) / (max_y - min_y)) as f32 * plot.height(),
        )
    };
    let clipped = ui.painter().with_clip_rect(plot);

    for line in series {
        let Some(baseline) = &line.fill_baseline else {
            continue;
        };
        let visible_indices = line
            .points
            .iter()
            .enumerate()
            .filter(|(_, point)| (min_x..=max_x).contains(&point.0))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if visible_indices.len() < 2 {
            continue;
        }
        let maximum_fill_points = (plot.width() * 2.0).max(250.0) as usize;
        let stride = visible_indices.len().div_ceil(maximum_fill_points).max(1);
        let fill_color = egui::Color32::from_rgba_unmultiplied(
            line.color.r(),
            line.color.g(),
            line.color.b(),
            if ui.visuals().dark_mode { 72 } else { 52 },
        );
        let mut mesh = egui::Mesh::default();
        let mut previous = None;
        for index in visible_indices
            .iter()
            .step_by(stride)
            .copied()
            .chain(visible_indices.last().copied())
        {
            let point = line.points[index];
            let bottom = baseline.get(index).copied().unwrap_or(0.0);
            let top_vertex = mesh.vertices.len() as u32;
            mesh.colored_vertex(position(point.0, bottom + point.1), fill_color);
            let bottom_vertex = mesh.vertices.len() as u32;
            mesh.colored_vertex(position(point.0, bottom), fill_color);
            if let Some((previous_top, previous_bottom)) = previous {
                mesh.add_triangle(previous_top, previous_bottom, top_vertex);
                mesh.add_triangle(previous_bottom, bottom_vertex, top_vertex);
            }
            previous = Some((top_vertex, bottom_vertex));
        }
        clipped.add(egui::Shape::mesh(mesh));
    }

    for line in series {
        let points = line
            .points
            .iter()
            .enumerate()
            .filter(|(_, point)| (min_x..=max_x).contains(&point.0))
            .map(|(index, point)| (index, point.0, series_y(line, index)))
            .collect::<Vec<_>>();
        if line.connected {
            let stride = points.len().div_ceil(4000).max(1);
            let mut drawn = Vec::new();
            for chunk in points.chunks(stride) {
                let low = chunk
                    .iter()
                    .enumerate()
                    .min_by(|a, b| a.1.2.total_cmp(&b.1.2));
                let high = chunk
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.2.total_cmp(&b.1.2));
                if let (Some((low_index, low)), Some((high_index, high))) = (low, high) {
                    if low_index <= high_index {
                        drawn.push(position(low.1, low.2));
                        if high_index != low_index {
                            drawn.push(position(high.1, high.2));
                        }
                    } else {
                        drawn.push(position(high.1, high.2));
                        drawn.push(position(low.1, low.2));
                    }
                }
            }
            if drawn.len() >= 2 {
                clipped.add(egui::Shape::line(drawn, egui::Stroke::new(2.1, line.color)));
            }
        }

        let marker_capacity = (plot.width() / 8.0).max(16.0) as usize;
        let show_every_marker = points.len() <= marker_capacity;
        if show_every_marker || !line.connected {
            let marker_stride = if show_every_marker {
                1
            } else {
                points.len().div_ceil(marker_capacity).max(1)
            };
            for (position_index, (_, x, y)) in points.iter().enumerate() {
                if position_index % marker_stride == 0 {
                    paint_plot_marker(
                        &clipped,
                        position(*x, *y),
                        if line.connected { 3.0 } else { 4.0 },
                        line.marker,
                        line.color,
                    );
                }
            }
        }
    }

    ui.painter().text(
        egui::pos2(plot.center().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        x_label,
        axis_font.clone(),
        axis_color,
    );
    ui.painter().text(
        egui::pos2(plot.left(), rect.top() + 2.0),
        egui::Align2::LEFT_TOP,
        y_label,
        axis_font.clone(),
        axis_color,
    );

    if let Some(pointer) = response
        .hover_pos()
        .filter(|pointer| plot.contains(*pointer))
    {
        let nearest = series
            .iter()
            .flat_map(|line| {
                line.points
                    .iter()
                    .enumerate()
                    .map(move |(index, point)| (line, index, point))
            })
            .filter(|(_, _, point)| (min_x..=max_x).contains(&point.0))
            .map(|(line, index, point)| {
                let screen = position(point.0, series_y(line, index));
                (line, index, point, screen, screen.distance_sq(pointer))
            })
            .min_by(|left, right| left.4.total_cmp(&right.4));
        if let Some((line, index, point, screen, distance)) = nearest {
            if distance <= 14.0 * 14.0 && plot.contains(screen) {
                paint_plot_marker(ui.painter(), screen, 5.0, line.marker, line.color);
                let detail = line.details.get(index).map_or("", String::as_str);
                let x_value = if x_label.contains("time")
                    || x_label.contains("Time")
                    || x_label.contains("seconds")
                {
                    format!("{} ({:.6} s)", elapsed_time_tick(point.0), point.0)
                } else {
                    format!("{:.6}", point.0)
                };
                response.clone().on_hover_text_at_pointer(format!(
                    "{}
{}: {}
{}: {:.9}
{}",
                    line.name, x_label, x_value, y_label, point.1, detail
                ));
            }
        }
    }

    let key = plot_id.to_owned();
    if response.drag_started_by(egui::PointerButton::Primary) {
        if let Some(pointer) = response
            .interact_pointer_pos()
            .filter(|pointer| plot.contains(*pointer))
        {
            state.graph_drag_start = Some((key.clone(), pointer));
        }
    }
    if let Some((drag_key, start)) = &state.graph_drag_start {
        if drag_key == &key {
            if let Some(pointer) = response.interact_pointer_pos() {
                let selection = egui::Rect::from_two_pos(*start, pointer).intersect(plot);
                ui.painter().rect_filled(
                    selection,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(40, 110, 220, 45),
                );
            }
        }
    }
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        if let (Some((drag_key, start)), Some(end)) = (
            state.graph_drag_start.take(),
            response.interact_pointer_pos(),
        ) {
            if drag_key == key && (end - start).length() > 8.0 {
                let left = start.x.min(end.x).clamp(plot.left(), plot.right());
                let right = start.x.max(end.x).clamp(plot.left(), plot.right());
                let top = start.y.min(end.y).clamp(plot.top(), plot.bottom());
                let bottom = start.y.max(end.y).clamp(plot.top(), plot.bottom());
                let next = state.graph_views.entry(key.clone()).or_default();
                if right - left > 8.0 && bottom - top > 8.0 {
                    let range = next.x_to_percent - next.x_from_percent;
                    let from = next.x_from_percent
                        + range * f64::from((left - plot.left()) / plot.width());
                    let to = next.x_from_percent
                        + range * f64::from((right - plot.left()) / plot.width());
                    next.x_from_percent = from;
                    next.x_to_percent = to;
                    let lower =
                        max_y - (max_y - min_y) * f64::from((bottom - plot.top()) / plot.height());
                    let upper =
                        max_y - (max_y - min_y) * f64::from((top - plot.top()) / plot.height());
                    next.y_bounds = Some((lower, upper));
                }
                normalize_graph_view(next, minimum_x_span, full_y);
            }
        }
    }
    if response.drag_started_by(egui::PointerButton::Secondary) {
        if let Some(pointer) = response
            .interact_pointer_pos()
            .filter(|pointer| plot.contains(*pointer))
        {
            state.graph_pan_start = Some((key.clone(), pointer, view));
        }
    }
    if response.dragged_by(egui::PointerButton::Secondary) {
        if let (Some((pan_key, start, initial)), Some(pointer)) =
            (&state.graph_pan_start, response.interact_pointer_pos())
        {
            if pan_key == &key {
                let x_span = initial.x_to_percent - initial.x_from_percent;
                let x_shift = -f64::from((pointer.x - start.x) / plot.width()) * x_span * 0.35;
                let next_from = (initial.x_from_percent + x_shift).clamp(0.0, 100.0 - x_span);
                let (low, high) = initial.y_bounds.unwrap_or((min_y, max_y));
                let y_shift =
                    f64::from((pointer.y - start.y) / plot.height()) * (high - low) * 0.35;
                let minimum_y_span = (full_y.1 - full_y.0) * minimum_x_span / 100.0;
                let y_bounds = bounded_axis_window(
                    low + y_shift,
                    high + y_shift,
                    full_y.0,
                    full_y.1,
                    minimum_y_span,
                );
                state.graph_views.insert(
                    key.clone(),
                    GraphView {
                        x_from_percent: next_from,
                        x_to_percent: next_from + x_span,
                        y_bounds: Some(y_bounds),
                    },
                );
            }
        }
    }
    if response.drag_stopped_by(egui::PointerButton::Secondary) {
        state.graph_pan_start = None;
    }
    if let Some(pointer) = response
        .hover_pos()
        .filter(|pointer| plot.contains(*pointer))
    {
        let x_anchor = f64::from((pointer.x - plot.left()) / plot.width());
        let y_anchor = f64::from((pointer.y - plot.top()) / plot.height());
        if response.dragged_by(egui::PointerButton::Middle) {
            let delta = ui.input(|input| input.pointer.delta().y);
            let factor = (f64::from(delta) * 0.01).exp().clamp(0.85, 1.18);
            zoom_view(
                state.graph_views.entry(key.clone()).or_default(),
                factor,
                x_anchor,
                y_anchor,
                (min_y, max_y),
                full_y,
                minimum_x_span,
            );
        }
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 {
            ui.input_mut(|input| input.smooth_scroll_delta = egui::Vec2::ZERO);
            let factor = (-f64::from(scroll) * 0.002).exp().clamp(0.65, 1.55);
            zoom_view(
                state.graph_views.entry(key.clone()).or_default(),
                factor,
                x_anchor,
                y_anchor,
                (min_y, max_y),
                full_y,
                minimum_x_span,
            );
        }
    }
    if state.graph_show_legend {
        ui.horizontal_wrapped(|ui| {
            for line in series {
                plot_marker_label(ui, line.marker, line.color);
                ui.colored_label(line.color, &line.name);
            }
        });
    }
}

fn axis_controls(ui: &mut egui::Ui, state: &mut ViewState) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("X axis:");
        changed |= ui
            .selectable_value(
                &mut state.horizontal_unit,
                HorizontalUnit::Packet,
                "Packet number",
            )
            .changed();
        changed |= ui
            .selectable_value(&mut state.horizontal_unit, HorizontalUnit::Seconds, "Time")
            .changed();
    });
    changed
}

fn format_bitrate_differences(min: f64, max: f64, average: f64, percent: bool) -> (String, String) {
    if percent {
        if average.abs() <= f64::EPSILON {
            return ("N/A".to_owned(), "N/A".to_owned());
        }
        (
            format!("{:+.1}%", (max - average) / average * 100.0),
            format!("{:+.1}%", (min - average) / average * 100.0),
        )
    } else {
        (
            format!("{:.3} Mb/s", max - average),
            format!("{:.3} Mb/s", average - min),
        )
    }
}

pub fn bitrate(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) {
    const PLOT_ID: &str = "bitrate";
    const BITRATE_KIND: u8 = 3;
    const TOTAL_KEY: u16 = 0xffff;

    ui.heading("Bitrate");
    ui.horizontal_wrapped(|ui| {
        ui.label("PID filter:");
        ui.add(
            egui::TextEdit::singleline(&mut state.graph_pid_filter)
                .hint_text("all / 0x0101")
                .desired_width(140.0),
        );
    });
    if axis_controls(ui, state) {
        state.graph_views.remove(PLOT_ID);
    }
    graph_range_controls(ui, state, PLOT_ID);
    let filter = match pid_filter(&state.graph_pid_filter) {
        Ok(filter) => filter,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
    };
    let mut by_pcr_pid: BTreeMap<u16, Vec<(u64, u64)>> = BTreeMap::new();
    for point in report
        .clock_points
        .iter()
        .filter(|point| point.kind == ClockKind::Pcr)
    {
        by_pcr_pid
            .entry(point.pid)
            .or_default()
            .push((point.packet_index, point.ticks));
    }
    let Some((pcr_pid, pcr)) = by_pcr_pid.iter().max_by_key(|(_, samples)| samples.len()) else {
        ui.label("Per-PID bitrate needs PCR timestamps; none were found.");
        return;
    };
    const WRAP: u64 = (1_u64 << 33) * 300;
    let sample_rate = |left: (u64, u64), right: (u64, u64)| {
        let delta = (right.1 + WRAP - left.1) % WRAP;
        (delta > 0 && delta <= 54_000_000 && right.0 > left.0)
            .then(|| (right.0 - left.0) as f64 * 188.0 * 8.0 * 27.0 / delta as f64)
    };
    let mut rates = pcr
        .windows(2)
        .filter_map(|pair| sample_rate(pair[0], pair[1]))
        .collect::<Vec<_>>();
    if rates.is_empty() {
        ui.label("PCR timestamps cannot establish a transport bitrate.");
        return;
    }
    rates.sort_by(f64::total_cmp);
    let fallback_rate = rates[rates.len() / 2];
    let interval_seconds =
        BITRATE_WINDOW_PACKETS as f64 * 188.0 * 8.0 / (fallback_rate * 1_000_000.0);
    key_value(
        ui,
        "Sampling interval",
        format!(
            "{} TS packets (about {:.3} s), PCR PID 0x{pcr_pid:04X}",
            BITRATE_WINDOW_PACKETS, interval_seconds
        ),
    );
    ui.weak("Bitrate is measured from packet counts in each sampling interval. Enabled PIDs are stacked by contribution; extra ticks and sample markers appear as you zoom in.");

    let horizontal_unit = state.horizontal_unit;
    let x_value = |packet: u64| match horizontal_unit {
        HorizontalUnit::Packet => packet as f64,
        HorizontalUnit::Seconds => packet as f64 * 188.0 * 8.0 / (fallback_rate * 1_000_000.0),
    };
    let x_label = if horizontal_unit == HorizontalUnit::Packet {
        "TS packet number at interval start"
    } else {
        "Elapsed stream time (PCR-derived)"
    };
    let mut window_rates = Vec::with_capacity(report.bitrate_windows.len());
    let mut pcr_cursor = 0;
    for window in &report.bitrate_windows {
        let end = window.first_packet + u64::from(window.packet_count);
        while pcr_cursor < pcr.len() && pcr[pcr_cursor].0 < window.first_packet {
            pcr_cursor += 1;
        }
        let mut last = None;
        let mut local = Vec::new();
        let mut cursor = pcr_cursor;
        while cursor < pcr.len() && pcr[cursor].0 < end {
            if let Some(previous) = last {
                if let Some(rate) = sample_rate(previous, pcr[cursor]) {
                    local.push(rate);
                }
            }
            last = Some(pcr[cursor]);
            cursor += 1;
        }
        window_rates.push(if local.is_empty() {
            fallback_rate
        } else {
            local.iter().sum::<f64>() / local.len() as f64
        });
    }

    let total_points = report
        .bitrate_windows
        .iter()
        .zip(&window_rates)
        .map(|(window, &rate)| (x_value(window.first_packet), rate))
        .collect::<Vec<_>>();
    let total_details = report
        .bitrate_windows
        .iter()
        .map(|window| {
            format!(
                "Packet {}..{}; {} TS packets",
                window.first_packet,
                window.first_packet + u64::from(window.packet_count) - 1,
                window.packet_count
            )
        })
        .collect::<Vec<_>>();
    let stats = |points: &[(f64, f64)]| {
        let min = points
            .iter()
            .map(|point| point.1)
            .fold(f64::INFINITY, f64::min);
        let max = points
            .iter()
            .map(|point| point.1)
            .fold(f64::NEG_INFINITY, f64::max);
        let avg = points.iter().map(|point| point.1).sum::<f64>() / points.len().max(1) as f64;
        (min, max, avg)
    };

    let mut rows = Vec::new();
    for (position, (&pid, pid_report)) in report.pids.iter().enumerate() {
        if filter.is_some_and(|wanted| wanted != pid) {
            continue;
        }
        let points = report
            .bitrate_windows
            .iter()
            .zip(&window_rates)
            .map(|(window, &rate)| {
                let count = f64::from(*window.pid_packets.get(&pid).unwrap_or(&0));
                (
                    x_value(window.first_packet),
                    rate * count / f64::from(window.packet_count),
                )
            })
            .collect::<Vec<_>>();
        let details = report
            .bitrate_windows
            .iter()
            .map(|window| {
                format!(
                    "Packet {}..{}; PID packets {}/{}",
                    window.first_packet,
                    window.first_packet + u64::from(window.packet_count) - 1,
                    window.pid_packets.get(&pid).copied().unwrap_or(0),
                    window.packet_count
                )
            })
            .collect::<Vec<_>>();
        let role = report
            .programs
            .values()
            .find_map(|program| program.streams.get(&pid))
            .map_or_else(
                || {
                    if pid == 0x1fff {
                        "Null"
                    } else {
                        "PSI/SI or other"
                    }
                },
                |stream| stream.name_for_standard(report.standard),
            );
        let values = stats(&points);
        rows.push((
            pid,
            format!("PID 0x{pid:04X} - {role}"),
            pid_report.packets,
            values,
            Series {
                name: format!("PID 0x{pid:04X} - {role}"),
                color: contrast_color(ui, position + 1),
                marker: PlotMarker::Square,
                points,
                details,
                connected: true,
                fill_baseline: None,
            },
        ));
    }

    ui.horizontal_wrapped(|ui| {
        if filter.is_none() {
            let enabled = state
                .graph_series_enabled
                .entry((TOTAL_KEY, BITRATE_KIND))
                .or_insert(true);
            ui.colored_label(contrast_color(ui, 0), "■");
            ui.checkbox(enabled, "Transport total");
        }
        for (pid, name, _, _, series) in &rows {
            let enabled = state
                .graph_series_enabled
                .entry((*pid, BITRATE_KIND))
                .or_insert(true);
            ui.colored_label(series.color, "■");
            ui.checkbox(enabled, name)
                .on_hover_text("Show or hide this PID");
        }
    });

    egui::CollapsingHeader::new("Bitrate Statistics")
        .default_open(true)
        .show(ui, |ui| {
            let difference_label = if state.bitrate_difference_percent {
                "Display: ± percent"
            } else {
                "Display: Mb/s difference"
            };
            if ui
                .button(difference_label)
                .on_hover_text("Switch between relative percentage and actual bitrate difference.")
                .clicked()
            {
                state.bitrate_difference_percent = !state.bitrate_difference_percent;
            }
            let percent = state.bitrate_difference_percent;
            let headings = if percent {
                [
                    "Series",
                    "Packets",
                    "Min Mb/s",
                    "Max Mb/s",
                    "Avg Mb/s",
                    "Max vs Avg",
                    "Min vs Avg",
                ]
            } else {
                [
                    "Series",
                    "Packets",
                    "Min Mb/s",
                    "Max Mb/s",
                    "Avg Mb/s",
                    "Max - Avg",
                    "Avg - Min",
                ]
            };
            egui::Grid::new("bitrate-statistics")
                .striped(true)
                .show(ui, |ui| {
                    for heading in headings {
                        ui.label(egui::RichText::new(heading).strong().color(item_color(ui)));
                    }
                    ui.end_row();
                    let add_row = |ui: &mut egui::Ui,
                                   color: egui::Color32,
                                   name: &str,
                                   packets: u64,
                                   min: f64,
                                   max: f64,
                                   avg: f64| {
                        ui.colored_label(color, format!("■ {name}"));
                        value_cell(ui, packets.to_string());
                        value_cell(ui, format!("{min:.3}"));
                        value_cell(ui, format!("{max:.3}"));
                        value_cell(ui, format!("{avg:.3}"));
                        let (above, below) = format_bitrate_differences(min, max, avg, percent);
                        value_cell(ui, above);
                        value_cell(ui, below);
                        ui.end_row();
                    };
                    if filter.is_none() {
                        let (min, max, avg) = stats(&total_points);
                        add_row(
                            ui,
                            contrast_color(ui, 0),
                            "Transport total",
                            report.packets,
                            min,
                            max,
                            avg,
                        );
                    }
                    for (_, name, packets, (min, max, avg), series) in &rows {
                        add_row(ui, series.color, name, *packets, *min, *max, *avg);
                    }
                });
        });

    let mut series = Vec::new();
    if filter.is_none()
        && state
            .graph_series_enabled
            .get(&(TOTAL_KEY, BITRATE_KIND))
            .copied()
            .unwrap_or(true)
    {
        series.push(Series {
            name: "Transport total".to_owned(),
            color: contrast_color(ui, 0),
            marker: PlotMarker::Square,
            points: total_points,
            details: total_details,
            connected: true,
            fill_baseline: None,
        });
    }
    let mut cumulative = vec![0.0; report.bitrate_windows.len()];
    for (pid, _, _, _, mut line) in rows {
        if state
            .graph_series_enabled
            .get(&(pid, BITRATE_KIND))
            .copied()
            .unwrap_or(true)
        {
            line.fill_baseline = Some(cumulative.clone());
            for (total, (_, value)) in cumulative.iter_mut().zip(&line.points) {
                *total += *value;
            }
            series.push(line);
        }
    }
    plot(
        ui,
        PLOT_ID,
        "Bitrate by PID",
        x_label,
        "Bitrate (Mb/s)",
        &series,
        state,
    );
}

fn contrast_color(ui: &egui::Ui, index: usize) -> egui::Color32 {
    const DARK: [egui::Color32; 10] = [
        egui::Color32::from_rgb(80, 190, 255),
        egui::Color32::from_rgb(255, 135, 65),
        egui::Color32::from_rgb(65, 220, 170),
        egui::Color32::from_rgb(255, 90, 165),
        egui::Color32::from_rgb(255, 205, 70),
        egui::Color32::from_rgb(185, 135, 255),
        egui::Color32::from_rgb(40, 225, 245),
        egui::Color32::from_rgb(155, 225, 70),
        egui::Color32::from_rgb(255, 120, 115),
        egui::Color32::from_rgb(235, 235, 235),
    ];
    const LIGHT: [egui::Color32; 10] = [
        egui::Color32::from_rgb(0, 82, 180),
        egui::Color32::from_rgb(210, 70, 0),
        egui::Color32::from_rgb(0, 125, 100),
        egui::Color32::from_rgb(190, 0, 95),
        egui::Color32::from_rgb(215, 125, 0),
        egui::Color32::from_rgb(105, 55, 165),
        egui::Color32::from_rgb(0, 140, 195),
        egui::Color32::from_rgb(80, 135, 0),
        egui::Color32::from_rgb(135, 65, 25),
        egui::Color32::from_rgb(30, 30, 30),
    ];
    if ui.visuals().dark_mode {
        DARK[index % DARK.len()]
    } else {
        LIGHT[index % LIGHT.len()]
    }
}

fn timestamp_color(ui: &egui::Ui, index: usize) -> egui::Color32 {
    if index % 10 == 3 {
        if ui.visuals().dark_mode {
            egui::Color32::from_rgb(180, 115, 255)
        } else {
            egui::Color32::from_rgb(100, 55, 170)
        }
    } else {
        contrast_color(ui, index)
    }
}

pub fn timestamps(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) {
    ui.heading("PCR / PTS / DTS");
    ui.horizontal_wrapped(|ui| {
        ui.label("Service:");
        egui::ComboBox::from_id_salt("clock-service")
            .selected_text(
                state
                    .graph_service
                    .map_or_else(|| "All".to_owned(), |id| format!("Service {id}")),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.graph_service, None, "All");
                for (&id, program) in &report.programs {
                    ui.selectable_value(
                        &mut state.graph_service,
                        Some(id),
                        format!(
                            "Service {id}, PCR PID {}",
                            program
                                .pcr_pid
                                .map_or_else(|| "none".to_owned(), |pid| format!("0x{pid:04X}"))
                        ),
                    );
                }
            });
        ui.label("PID:");
        ui.add(
            egui::TextEdit::singleline(&mut state.graph_pid_filter)
                .hint_text("all / 0x0101")
                .desired_width(120.0),
        );
        ui.checkbox(&mut state.show_pcr, "PCR");
        ui.checkbox(&mut state.show_pts, "PTS");
        ui.checkbox(&mut state.show_dts, "DTS");
    });
    let mut axes_changed = axis_controls(ui, state);
    ui.horizontal_wrapped(|ui| {
        ui.label("Y axis:");
        axes_changed |= ui
            .selectable_value(&mut state.graph_relative_clock, false, "Clock seconds")
            .changed();
        axes_changed |= ui
            .selectable_value(
                &mut state.graph_relative_clock,
                true,
                "Relative to first sample",
            )
            .changed();
    });
    if axes_changed {
        state.graph_views.remove("timestamps");
    }
    graph_range_controls(ui, state, "timestamps");
    let filter = match pid_filter(&state.graph_pid_filter) {
        Ok(filter) => filter,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
    };
    let mut first: BTreeMap<(u16, u8), u64> = BTreeMap::new();
    let mut grouped: BTreeMap<(u16, u8), (Vec<(f64, f64)>, Vec<String>)> = BTreeMap::new();
    let pcr_first = report
        .clock_points
        .iter()
        .find(|point| point.kind == ClockKind::Pcr)
        .map(|point| (point.packet_index, point.ticks));
    let pcr_last = report
        .clock_points
        .iter()
        .rev()
        .find(|point| point.kind == ClockKind::Pcr);
    let packet_rate = pcr_first.zip(pcr_last).and_then(|((packet, ticks), last)| {
        let wrap = (1_u64 << 33) * 300;
        let delta = (last.ticks + wrap - ticks) % wrap;
        (delta > 0 && last.packet_index > packet)
            .then_some((last.packet_index - packet) as f64 / (delta as f64 / 27_000_000.0))
    });
    let selected_service = state.graph_service.and_then(|id| report.programs.get(&id));
    for point in &report.clock_points {
        let (kind, enabled, scale, wrap) = match point.kind {
            ClockKind::Pcr => (0, state.show_pcr, 27_000_000.0, (1_u64 << 33) * 300),
            ClockKind::Pts => (1, state.show_pts, 90_000.0, 1_u64 << 33),
            ClockKind::Dts => (2, state.show_dts, 90_000.0, 1_u64 << 33),
        };
        if !enabled
            || filter.is_some_and(|pid| pid != point.pid)
            || !state
                .graph_series_enabled
                .get(&(point.pid, kind))
                .copied()
                .unwrap_or(true)
            || selected_service.is_some_and(|service| {
                service.pcr_pid != Some(point.pid) && !service.streams.contains_key(&point.pid)
            })
        {
            continue;
        }
        let initial = *first.entry((point.pid, kind)).or_insert(point.ticks);
        let delta = (i128::from(point.ticks) - i128::from(initial) + i128::from(wrap / 2))
            .rem_euclid(i128::from(wrap))
            - i128::from(wrap / 2);
        let y = if state.graph_relative_clock {
            delta as f64 / scale
        } else {
            initial as f64 / scale + delta as f64 / scale
        };
        let x = match (state.horizontal_unit, pcr_first, packet_rate) {
            (HorizontalUnit::Seconds, Some((packet, _)), Some(rate)) => {
                (i128::from(point.packet_index) - i128::from(packet)) as f64 / rate
            }
            _ => point.packet_index as f64,
        };
        let entry = grouped.entry((point.pid, kind)).or_default();
        entry.0.push((x, y));
        entry.1.push(format!(
            "Packet {} / offset 0x{:X}; raw {} ticks; {:.9} clock seconds",
            point.packet_index,
            report.packet_offset(point.packet_index),
            point.ticks,
            point.ticks as f64 / scale
        ));
    }
    let mut series = Vec::new();
    ui.horizontal_wrapped(|ui| {
        let keys = report
            .clock_points
            .iter()
            .map(|p| {
                (
                    p.pid,
                    match p.kind {
                        ClockKind::Pcr => 0,
                        ClockKind::Pts => 1,
                        ClockKind::Dts => 2,
                    },
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        for (index, (pid, kind)) in keys.into_iter().enumerate() {
            let name = match kind {
                0 => "PCR",
                1 => "PTS",
                _ => "DTS",
            };
            let enabled = state
                .graph_series_enabled
                .entry((pid, kind))
                .or_insert(true);
            let color = timestamp_color(ui, index);
            let marker = PlotMarker::for_index(index);
            plot_marker_label(ui, marker, color);
            ui.checkbox(enabled, format!("0x{pid:04X} {name}"))
                .on_hover_text("Toggle this PID and clock type");
            if let Some((points, details)) = grouped.remove(&(pid, kind)) {
                series.push(Series {
                    name: format!("PID 0x{pid:04X} {name} ({} samples)", points.len()),
                    color,
                    marker,
                    points,
                    details,
                    connected: false,
                    fill_baseline: None,
                });
            }
        }
    });
    plot(
        ui,
        "timestamps",
        "Clock samples",
        if state.horizontal_unit == HorizontalUnit::Packet {
            "TS packet number"
        } else {
            "Elapsed stream time (PCR-derived)"
        },
        if state.graph_relative_clock {
            "Relative seconds"
        } else {
            "Clock seconds"
        },
        &series,
        state,
    );
}

fn access_point_list(ui: &mut egui::Ui, by_pid: &BTreeMap<u16, Vec<u64>>) {
    ui.heading("Access-point packets");
    ArrowScrollArea::vertical()
        .id_salt("access-point-packet-list")
        .max_height(500.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (pid, packets) in by_pid {
                egui::CollapsingHeader::new(format!(
                    "PID 0x{pid:04X} ({} access points)",
                    packets.len()
                ))
                .default_open(true)
                .show(ui, |ui| {
                    for packet in packets {
                        key_value(ui, "Packet", format!("{packet} | PID 0x{pid:04X}"));
                    }
                });
            }
        });
}

pub fn gop(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) {
    const PLOT_ID: &str = "gop";
    ui.heading("Random access spacing");
    ui.label("Each point is the distance from the preceding random_access_indicator on the same PID. It is measured in TS packets; this does not identify video frame types or prove a complete GOP.");
    ui.horizontal(|ui| {
        ui.label("PID:");
        ui.add(
            egui::TextEdit::singleline(&mut state.graph_pid_filter)
                .hint_text("all / 0x0101")
                .desired_width(140.0),
        );
    });
    graph_range_controls(ui, state, PLOT_ID);
    let filter = match pid_filter(&state.graph_pid_filter) {
        Ok(filter) => filter,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
    };
    let mut by_pid: BTreeMap<u16, Vec<u64>> = BTreeMap::new();
    for &(packet, pid) in &report.random_access_points {
        if filter.is_none_or(|wanted| wanted == pid) {
            by_pid.entry(pid).or_default().push(packet);
        }
    }
    let total = by_pid.values().map(Vec::len).sum::<usize>();
    key_value(ui, "random_access_indicator count", total.to_string());

    let mut series = Vec::new();
    egui::Grid::new("gop-statistics")
        .striped(true)
        .show(ui, |ui| {
            for heading in [
                "PID",
                "Access points",
                "Min spacing",
                "Max spacing",
                "Average",
            ] {
                ui.label(egui::RichText::new(heading).strong().color(item_color(ui)));
            }
            ui.end_row();
            for (position, (pid, packets)) in by_pid.iter().enumerate() {
                let points = packets
                    .windows(2)
                    .map(|pair| (pair[1] as f64, (pair[1] - pair[0]) as f64))
                    .collect::<Vec<_>>();
                if points.is_empty() {
                    continue;
                }
                let mut gaps = points.iter().map(|point| point.1).collect::<Vec<_>>();
                gaps.sort_by(f64::total_cmp);
                let avg = gaps.iter().sum::<f64>() / gaps.len() as f64;
                value_cell(ui, format!("0x{pid:04X}"));
                value_cell(ui, packets.len().to_string());
                value_cell(ui, format!("{:.0}", gaps[0]));
                value_cell(ui, format!("{:.0}", gaps[gaps.len() - 1]));
                value_cell(ui, format!("{avg:.1} TS packets"));
                ui.end_row();
                series.push(Series {
                    name: format!("PID 0x{pid:04X} spacing"),
                    color: contrast_color(ui, position),
                    marker: PlotMarker::Diamond,
                    details: packets
                        .windows(2)
                        .map(|pair| format!("Access points: packets {} and {}", pair[0], pair[1]))
                        .collect(),
                    points,
                    connected: true,
                    fill_baseline: None,
                });
            }
        });

    if ui.available_width() >= 900.0 {
        ui.columns(2, |columns| {
            plot(
                &mut columns[0],
                PLOT_ID,
                "Access-point spacing in stream order",
                "Second access-point packet",
                "TS packets",
                &series,
                state,
            );
            access_point_list(&mut columns[1], &by_pid);
        });
    } else {
        access_point_list(ui, &by_pid);
        plot(
            ui,
            PLOT_ID,
            "Access-point spacing in stream order",
            "Second access-point packet",
            "TS packets",
            &series,
            state,
        );
    }
}

#[cfg(test)]
mod view_tests {
    use super::{
        GraphView, HorizontalUnit, ViewState, elapsed_time_tick, format_bitrate_differences,
        zoom_view,
    };

    #[test]
    fn graph_axis_defaults_to_time() {
        assert!(matches!(
            ViewState::new().horizontal_unit,
            HorizontalUnit::Seconds
        ));
    }

    #[test]
    fn elapsed_axis_uses_clock_style_labels() {
        assert_eq!(elapsed_time_tick(0.0), "00:00.000");
        assert_eq!(elapsed_time_tick(65.25), "01:05.250");
        assert_eq!(elapsed_time_tick(3_661.5), "01:01:01.500");
    }

    #[test]
    fn bitrate_statistics_formats_percent_and_actual_differences() {
        assert_eq!(
            format_bitrate_differences(8.0, 12.0, 10.0, true),
            ("+20.0%".to_owned(), "-20.0%".to_owned())
        );
        assert_eq!(
            format_bitrate_differences(8.0, 12.0, 10.0, false),
            ("2.000 Mb/s".to_owned(), "2.000 Mb/s".to_owned())
        );
        assert_eq!(
            format_bitrate_differences(0.0, 0.0, 0.0, true),
            ("N/A".to_owned(), "N/A".to_owned())
        );
    }

    #[test]
    fn proportional_zoom_keeps_cursor_anchor_in_both_axes() {
        let mut view = GraphView {
            x_from_percent: 0.0,
            x_to_percent: 100.0,
            y_bounds: Some((0.0, 100.0)),
        };

        zoom_view(&mut view, 0.5, 0.5, 0.5, (0.0, 100.0), (0.0, 100.0), 1.0);

        assert_eq!(view.x_from_percent, 25.0);
        assert_eq!(view.x_to_percent, 75.0);
        assert_eq!(view.y_bounds, Some((25.0, 75.0)));
    }

    #[test]
    fn zoom_stops_both_axes_at_the_data_resolution_limit() {
        let mut view = GraphView {
            x_from_percent: 0.0,
            x_to_percent: 100.0,
            y_bounds: Some((0.0, 200.0)),
        };

        zoom_view(
            &mut view,
            0.000_001,
            0.5,
            0.5,
            (0.0, 200.0),
            (0.0, 200.0),
            1.0,
        );
        assert!((view.x_to_percent - view.x_from_percent - 1.0).abs() < f64::EPSILON);
        let (low, high) = view.y_bounds.expect("zoom creates explicit Y bounds");
        assert!((high - low - 2.0).abs() < f64::EPSILON);

        let limited = view;
        zoom_view(&mut view, 0.1, 0.25, 0.75, (0.0, 200.0), (0.0, 200.0), 1.0);
        assert_eq!(view.x_from_percent, limited.x_from_percent);
        assert_eq!(view.x_to_percent, limited.x_to_percent);
        assert_eq!(view.y_bounds, limited.y_bounds);
    }

    #[test]
    fn zoom_out_is_bounded_by_the_full_data_extent() {
        let mut view = GraphView {
            x_from_percent: 25.0,
            x_to_percent: 75.0,
            y_bounds: Some((50.0, 150.0)),
        };

        zoom_view(&mut view, 10.0, 0.5, 0.5, (50.0, 150.0), (0.0, 200.0), 1.0);

        assert_eq!(view.x_from_percent, 0.0);
        assert_eq!(view.x_to_percent, 100.0);
        assert_eq!(view.y_bounds, None);
    }

    #[test]
    fn a_full_y_axis_does_not_block_x_zoom_out() {
        let mut view = GraphView {
            x_from_percent: 40.0,
            x_to_percent: 60.0,
            y_bounds: Some((0.0, 200.0)),
        };

        zoom_view(&mut view, 2.0, 0.5, 0.5, (0.0, 200.0), (0.0, 200.0), 1.0);
        assert_eq!(view.x_from_percent, 30.0);
        assert_eq!(view.x_to_percent, 70.0);
        assert_eq!(view.y_bounds, Some((0.0, 200.0)));

        zoom_view(&mut view, 10.0, 0.5, 0.5, (0.0, 200.0), (0.0, 200.0), 1.0);
        assert_eq!(view.x_from_percent, 0.0);
        assert_eq!(view.x_to_percent, 100.0);
        assert_eq!(view.y_bounds, None);
    }

    #[test]
    fn graph_viewports_are_independent() {
        let mut state = ViewState::new();
        state.graph_views.insert(
            "bitrate".to_owned(),
            GraphView {
                x_from_percent: 10.0,
                x_to_percent: 20.0,
                y_bounds: Some((1.0, 2.0)),
            },
        );

        assert_eq!(
            state
                .graph_views
                .entry("timestamps".to_owned())
                .or_default()
                .x_to_percent,
            100.0
        );
        assert_eq!(state.graph_views["bitrate"].x_from_percent, 10.0);
    }
}
