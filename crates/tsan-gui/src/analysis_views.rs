use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use eframe::egui;
use tsan_analyzer::{AnalysisReport, ClockKind, PacketWindow, read_packet_window};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum HorizontalUnit {
    #[default]
    Packet,
    Seconds,
}

#[derive(Default)]
pub struct ViewState {
    pub psi_filter: String,
    pub packet_pid_filter: String,
    pub packet_start: u64,
    pub selected_packet: Option<u64>,
    pub tr_filter: String,
    pub graph_pid_filter: String,
    pub horizontal_unit: HorizontalUnit,
    pub show_pcr: bool,
    pub show_pts: bool,
    pub show_dts: bool,
    packet_cache: Option<(PathBuf, u64, Option<u16>, PacketWindow)>,
}

impl ViewState {
    pub fn new() -> Self {
        Self {
            show_pcr: true,
            show_pts: true,
            show_dts: true,
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

pub fn psi_si_tree(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) {
    ui.heading("PSI / SI Tree");
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.add(
            egui::TextEdit::singleline(&mut state.psi_filter).hint_text("Table, PID or table ID"),
        );
    });
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
        egui::CollapsingHeader::new(format!(
            "{name} · PID 0x{pid:04X} · table 0x{table_id:02X} · {} sections",
            table.sections
        ))
        .id_salt((pid, table_id))
        .show(ui, |ui| {
            ui.monospace(format!(
                "Version: {}  Section: {}/{}  CRC errors: {}",
                table
                    .version
                    .map_or_else(|| "N/A".to_owned(), |v| v.to_string()),
                table
                    .section_number
                    .map_or_else(|| "N/A".to_owned(), |v| v.to_string()),
                table
                    .last_section_number
                    .map_or_else(|| "N/A".to_owned(), |v| v.to_string()),
                table.crc_errors
            ));
            if table_id == 0x00 {
                for (&program, data) in &report.programs {
                    ui.monospace(format!(
                        "Program {program} → PMT PID 0x{:04X}",
                        data.pmt_pid
                    ));
                }
            } else if table_id == 0x02 {
                for (&program, data) in &report.programs {
                    if data.pmt_pid != pid {
                        continue;
                    }
                    ui.monospace(format!(
                        "Program {program}; PCR PID {}",
                        data.pcr_pid
                            .map_or_else(|| "unknown".to_owned(), |pid| format!("0x{pid:04X}"))
                    ));
                    for (&stream_pid, stream) in &data.streams {
                        ui.monospace(format!(
                            "  ES PID 0x{stream_pid:04X}: {} (type 0x{:02X})",
                            stream.name_for_standard(report.standard),
                            stream.stream_type
                        ));
                    }
                }
            }
            egui::CollapsingHeader::new("First section bytes").show(ui, |ui| {
                let mut hex = String::new();
                for (index, byte) in table.first_section.iter().enumerate() {
                    if index % 16 == 0 {
                        let _ = write!(hex, "\n{index:04X}: ");
                    }
                    let _ = write!(hex, "{byte:02X} ");
                }
                let mut text = hex.as_str();
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
        });
    }
    if shown == 0 {
        ui.label("No observed tables match the filter.");
    }
    ui.weak("Only observed table instances are shown. Unknown descriptor payloads remain available as section bytes.");
}

pub fn packets(ui: &mut egui::Ui, path: &Path, report: &AnalysisReport, state: &mut ViewState) {
    ui.heading("Packets");
    ui.label(format!(
        "{} TS packets · {} PIDs",
        report.packets,
        report.pids.len()
    ));
    ui.horizontal(|ui| {
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
    egui::ScrollArea::both()
        .id_salt("packet-list-scroll")
        .max_height(400.0)
        .show(ui, |ui| {
            egui::Grid::new("packet-grid").striped(true).show(ui, |ui| {
                for title in ["#", "Offset", "PID", "PUSI", "CC", "PCR", "Flags"] {
                    ui.strong(title);
                }
                ui.end_row();
                for packet in &window.packets {
                    if ui
                        .selectable_label(selected == Some(packet.index), packet.index.to_string())
                        .clicked()
                    {
                        selected = Some(packet.index);
                    }
                    ui.monospace(format!("0x{:X}", packet.offset));
                    ui.monospace(format!("0x{:04X}", packet.pid));
                    ui.label(if packet.payload_unit_start { "Y" } else { "" });
                    ui.label(packet.continuity_counter.to_string());
                    ui.label(packet.pcr_27mhz.map_or_else(String::new, |pcr| {
                        format!("{:.6}", pcr as f64 / 27_000_000.0)
                    }));
                    ui.label(format!(
                        "{}{}{}{}",
                        if packet.transport_error { "TEI " } else { "" },
                        if packet.scrambled { "SCR " } else { "" },
                        if packet.random_access { "RAP " } else { "" },
                        if packet.discontinuity { "DISC" } else { "" }
                    ));
                    ui.end_row();
                }
            });
        });
    state.selected_packet = selected;
    if let Some(packet) = window
        .packets
        .iter()
        .find(|packet| Some(packet.index) == selected)
    {
        ui.separator();
        ui.heading(format!(
            "Packet {} · PID 0x{:04X}",
            packet.index, packet.pid
        ));
        ui.monospace(format!(
            "Offset 0x{:X} · CC {} · Payload {} · Adaptation {}",
            packet.offset, packet.continuity_counter, packet.payload, packet.adaptation
        ));
        let mut hex = String::new();
        for (index, byte) in packet.bytes.iter().enumerate() {
            if index % 16 == 0 {
                let _ = write!(hex, "\n{index:04X}: ");
            }
            let _ = write!(hex, "{byte:02X} ");
        }
        let mut text = hex.as_str();
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        });
    }
}

fn priority(ui: &mut egui::Ui, name: &str, indicators: &[(&str, Option<u64>)], needle: &str) {
    egui::CollapsingHeader::new(name)
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new(name).striped(true).show(ui, |ui| {
                ui.strong("Indicator");
                ui.strong("Observed");
                ui.end_row();
                for &(label, count) in indicators {
                    if !needle.is_empty() && !label.to_ascii_lowercase().contains(needle) {
                        continue;
                    }
                    ui.label(label);
                    match count {
                        Some(0) => {
                            ui.colored_label(ui.visuals().strong_text_color(), "0");
                        }
                        Some(value) => {
                            ui.colored_label(ui.visuals().error_fg_color, value.to_string());
                        }
                        None => {
                            ui.weak("Not measured");
                        }
                    };
                    ui.end_row();
                }
            });
        });
}

pub fn tr_101_290(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) {
    ui.heading("TR 101 290");
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.add(egui::TextEdit::singleline(&mut state.tr_filter).hint_text("Indicator"));
    });
    let needle = state.tr_filter.trim().to_ascii_lowercase();
    let cc = report.pids.values().map(|pid| pid.continuity_errors).sum();
    let tei = report.pids.values().map(|pid| pid.transport_errors).sum();
    let pat_missing = u64::from(!report.tables.contains_key(&(0, 0)));
    let pmt_missing = u64::from(
        report
            .programs
            .values()
            .any(|program| !report.tables.contains_key(&(program.pmt_pid, 2))),
    );
    priority(
        ui,
        "Priority 1",
        &[
            ("TS_sync_loss", None),
            ("Sync_byte_error", Some(report.malformed_packets)),
            ("PAT_error (presence only)", Some(pat_missing)),
            ("Continuity_count_error", Some(cc)),
            ("PMT_error (presence only)", Some(pmt_missing)),
            ("PID_error", None),
        ],
        &needle,
    );
    priority(
        ui,
        "Priority 2",
        &[
            ("Transport_error", Some(tei)),
            ("CRC_error", Some(report.section_crc_errors)),
            ("PCR_repetition_error", None),
            ("PCR_discontinuity_indicator_error", None),
            ("PCR_accuracy_error", None),
            ("PTS_error", None),
            ("CAT_error", None),
        ],
        &needle,
    );
    priority(
        ui,
        "Priority 3",
        &[
            ("NIT_actual_error", None),
            ("NIT_other_error", None),
            ("SI_repetition_error", None),
            ("Unreferenced_PID", None),
            ("SDT_actual_error", None),
            ("SDT_other_error", None),
            ("EIT_actual_error", None),
            ("EIT_other_error", None),
            ("EIT_PF_error", None),
            ("RST_error", None),
            ("TDT_error", None),
        ],
        &needle,
    );
    ui.separator();
    ui.heading("Observed PID events");
    for (&pid, data) in &report.pids {
        if data.continuity_errors + data.transport_errors == 0 {
            continue;
        }
        ui.monospace(format!(
            "PID 0x{pid:04X}: CC {} · TEI {}",
            data.continuity_errors, data.transport_errors
        ));
    }
    ui.weak("A zero is reported only for measured checks. Repetition, jitter and many SI checks require event-level validators and are not yet claimed as passed.");
}

struct Series {
    name: &'static str,
    color: egui::Color32,
    points: Vec<(f64, f64)>,
}

fn plot(ui: &mut egui::Ui, title: &str, x_label: &str, y_label: &str, series: &[Series]) {
    ui.heading(title);
    let dark = ui.visuals().dark_mode;
    let grid = if dark {
        egui::Color32::from_gray(90)
    } else {
        egui::Color32::from_gray(190)
    };
    let width = ui.available_width().max(320.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 280.0), egui::Sense::hover());
    let plot = rect.shrink2(egui::vec2(48.0, 22.0));
    let all = series
        .iter()
        .flat_map(|series| series.points.iter())
        .copied()
        .collect::<Vec<_>>();
    if all.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No timestamp samples for this filter",
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        return;
    }
    let min_x = all
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let max_x = all
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max)
        .max(min_x + 1.0);
    let min_y = all
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max_y = all
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .max(min_y + 0.001);
    for step in 0..=4 {
        let x = plot.left() + plot.width() * step as f32 / 4.0;
        let y = plot.top() + plot.height() * step as f32 / 4.0;
        ui.painter()
            .vline(x, plot.y_range(), egui::Stroke::new(1.0, grid));
        ui.painter()
            .hline(plot.x_range(), y, egui::Stroke::new(1.0, grid));
        ui.painter().text(
            egui::pos2(x, plot.bottom() + 2.0),
            egui::Align2::CENTER_TOP,
            format!("{:.1}", min_x + (max_x - min_x) * step as f64 / 4.0),
            egui::TextStyle::Small.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        ui.painter().text(
            egui::pos2(plot.left() - 4.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.2}", max_y - (max_y - min_y) * step as f64 / 4.0),
            egui::TextStyle::Small.resolve(ui.style()),
            ui.visuals().text_color(),
        );
    }
    for line in series {
        let stride = (line.points.len() / 4000).max(1);
        let points = line
            .points
            .iter()
            .step_by(stride)
            .map(|&(x, y)| {
                egui::pos2(
                    plot.left() + ((x - min_x) / (max_x - min_x)) as f32 * plot.width(),
                    plot.bottom() - ((y - min_y) / (max_y - min_y)) as f32 * plot.height(),
                )
            })
            .collect::<Vec<_>>();
        if points.len() >= 2 {
            ui.painter().add(egui::Shape::line(
                points,
                egui::Stroke::new(2.0, line.color),
            ));
        }
    }
    ui.painter().text(
        egui::pos2(plot.center().x, rect.bottom()),
        egui::Align2::CENTER_BOTTOM,
        x_label,
        egui::TextStyle::Small.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    ui.painter().text(
        egui::pos2(rect.left(), plot.top()),
        egui::Align2::LEFT_TOP,
        y_label,
        egui::TextStyle::Small.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    if let Some(pointer) = response.hover_pos().filter(|point| plot.contains(*point)) {
        let x = min_x + ((pointer.x - plot.left()) / plot.width()) as f64 * (max_x - min_x);
        if let Some((name, value)) = series
            .iter()
            .flat_map(|line| line.points.iter().map(move |point| (line.name, point)))
            .min_by(|left, right| (left.1.0 - x).abs().total_cmp(&(right.1.0 - x).abs()))
        {
            response.on_hover_text(format!("{name}: x={:.3}, y={:.3}", value.0, value.1));
        }
    }
    ui.horizontal_wrapped(|ui| {
        for line in series {
            ui.colored_label(line.color, format!("● {}", line.name));
        }
    });
}

fn axis_controls(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.horizontal(|ui| {
        ui.label("X axis:");
        ui.selectable_value(
            &mut state.horizontal_unit,
            HorizontalUnit::Packet,
            "Packet no.",
        );
        ui.selectable_value(
            &mut state.horizontal_unit,
            HorizontalUnit::Seconds,
            "PCR seconds",
        );
    });
}

pub fn bitrate(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) {
    ui.heading("Bitrate");
    ui.horizontal(|ui| {
        ui.label("PCR PID:");
        ui.add(
            egui::TextEdit::singleline(&mut state.graph_pid_filter)
                .hint_text("all / 0x0101")
                .desired_width(140.0),
        );
    });
    axis_controls(ui, state);
    let filter = match pid_filter(&state.graph_pid_filter) {
        Ok(filter) => filter,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
    };
    let mut previous: BTreeMap<u16, (u64, u64)> = BTreeMap::new();
    let mut first: BTreeMap<u16, u64> = BTreeMap::new();
    let mut rates: BTreeMap<u16, Vec<(f64, f64)>> = BTreeMap::new();
    for point in report
        .clock_points
        .iter()
        .filter(|point| point.kind == ClockKind::Pcr)
    {
        if filter.is_some_and(|pid| pid != point.pid) {
            continue;
        }
        first.entry(point.pid).or_insert(point.ticks);
        if let Some((packet, ticks)) = previous.insert(point.pid, (point.packet_index, point.ticks))
        {
            const WRAP: u64 = (1_u64 << 33) * 300;
            let delta = (point.ticks + WRAP - ticks) % WRAP;
            if delta == 0 || delta > 54_000_000 || point.packet_index <= packet {
                continue;
            }
            let rate = (point.packet_index - packet) as f64 * 188.0 * 8.0 * 27.0 / delta as f64;
            let x = match state.horizontal_unit {
                HorizontalUnit::Packet => point.packet_index as f64,
                HorizontalUnit::Seconds => {
                    ((point.ticks + WRAP - first[&point.pid]) % WRAP) as f64 / 27_000_000.0
                }
            };
            rates.entry(point.pid).or_default().push((x, rate));
        }
    }
    let mut series = Vec::new();
    for (position, (pid, points)) in rates.iter().enumerate() {
        let mut values = points.iter().map(|point| point.1).collect::<Vec<_>>();
        values.sort_by(f64::total_cmp);
        let min = values[0];
        let max = values[values.len() - 1];
        let avg = values.iter().sum::<f64>() / values.len() as f64;
        let p95 = values[(values.len() * 95 / 100).min(values.len() - 1)];
        ui.monospace(format!("PCR PID 0x{pid:04X} · min {min:.3} · max {max:.3} · avg {avg:.3} · p95 {p95:.3} Mb/s · burst {:.2}×", max / avg));
        series.push(Series {
            name: "Rate per PCR",
            color: contrast_color(ui, position),
            points: points.clone(),
        });
    }
    plot(
        ui,
        "Rate per PCR",
        if state.horizontal_unit == HorizontalUnit::Packet {
            "Packet no."
        } else {
            "PCR seconds"
        },
        "Mb/s",
        &series,
    );
    ui.weak("Rate is estimated from adjacent PCR samples. Faulty PCR or missing samples can distort these values; this is not an independent wall-clock measurement.");
}

fn contrast_color(ui: &egui::Ui, index: usize) -> egui::Color32 {
    const DARK: [egui::Color32; 6] = [
        egui::Color32::from_rgb(100, 210, 255),
        egui::Color32::from_rgb(255, 205, 90),
        egui::Color32::from_rgb(165, 235, 125),
        egui::Color32::from_rgb(255, 145, 190),
        egui::Color32::from_rgb(200, 165, 255),
        egui::Color32::from_rgb(255, 170, 110),
    ];
    const LIGHT: [egui::Color32; 6] = [
        egui::Color32::from_rgb(0, 91, 163),
        egui::Color32::from_rgb(157, 91, 0),
        egui::Color32::from_rgb(28, 108, 20),
        egui::Color32::from_rgb(162, 30, 91),
        egui::Color32::from_rgb(100, 51, 152),
        egui::Color32::from_rgb(156, 62, 0),
    ];
    if ui.visuals().dark_mode {
        DARK[index % DARK.len()]
    } else {
        LIGHT[index % LIGHT.len()]
    }
}

pub fn timestamps(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) {
    ui.heading("PCR / PTS / DTS");
    ui.horizontal(|ui| {
        ui.label("PID:");
        ui.add(
            egui::TextEdit::singleline(&mut state.graph_pid_filter)
                .hint_text("all / 0x0101")
                .desired_width(140.0),
        );
        ui.checkbox(&mut state.show_pcr, "PCR");
        ui.checkbox(&mut state.show_pts, "PTS");
        ui.checkbox(&mut state.show_dts, "DTS");
    });
    axis_controls(ui, state);
    let filter = match pid_filter(&state.graph_pid_filter) {
        Ok(filter) => filter,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
    };
    let mut first: BTreeMap<(u16, u8), u64> = BTreeMap::new();
    let mut grouped: BTreeMap<(u16, u8), Vec<(f64, f64)>> = BTreeMap::new();
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
    for point in &report.clock_points {
        let (kind, enabled, scale, wrap) = match point.kind {
            ClockKind::Pcr => (0, state.show_pcr, 27_000_000.0, (1_u64 << 33) * 300),
            ClockKind::Pts => (1, state.show_pts, 90_000.0, 1_u64 << 33),
            ClockKind::Dts => (2, state.show_dts, 90_000.0, 1_u64 << 33),
        };
        if !enabled || filter.is_some_and(|pid| pid != point.pid) {
            continue;
        }
        let initial = *first.entry((point.pid, kind)).or_insert(point.ticks);
        let y = ((point.ticks + wrap - initial) % wrap) as f64 / scale;
        let x = match (state.horizontal_unit, pcr_first) {
            (HorizontalUnit::Packet, _) => point.packet_index as f64,
            (HorizontalUnit::Seconds, Some((packet, ticks))) if point.packet_index >= packet => {
                let _ = ticks;
                packet_rate.map_or(point.packet_index as f64, |rate| {
                    (point.packet_index - packet) as f64 / rate
                })
            }
            _ => point.packet_index as f64,
        };
        grouped.entry((point.pid, kind)).or_default().push((x, y));
    }
    let series = grouped
        .into_iter()
        .enumerate()
        .map(|(position, ((pid, kind), points))| {
            let name = match kind {
                0 => "PCR",
                1 => "PTS",
                _ => "DTS",
            };
            ui.colored_label(
                contrast_color(ui, position),
                format!("PID 0x{pid:04X} {name} · {} samples", points.len()),
            );
            Series {
                name,
                color: contrast_color(ui, position),
                points,
            }
        })
        .collect::<Vec<_>>();
    plot(
        ui,
        "Clock samples",
        if state.horizontal_unit == HorizontalUnit::Packet {
            "Packet no."
        } else {
            "Estimated seconds"
        },
        "Relative seconds",
        &series,
    );
}

pub fn gop(ui: &mut egui::Ui, report: &AnalysisReport, state: &mut ViewState) {
    ui.heading("GOP / Random Access");
    ui.horizontal(|ui| {
        ui.label("PID:");
        ui.add(
            egui::TextEdit::singleline(&mut state.graph_pid_filter)
                .hint_text("all / 0x0101")
                .desired_width(140.0),
        );
    });
    let filter = match pid_filter(&state.graph_pid_filter) {
        Ok(filter) => filter,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
    };
    let points = report
        .random_access_points
        .iter()
        .filter(|(_, pid)| filter.is_none_or(|wanted| *pid == wanted))
        .copied()
        .collect::<Vec<_>>();
    ui.label(format!("{} random-access flags observed", points.len()));
    if points.is_empty() {
        return;
    }
    let mut gaps = points
        .windows(2)
        .filter(|pair| pair[0].1 == pair[1].1)
        .map(|pair| pair[1].0 - pair[0].0)
        .collect::<Vec<_>>();
    if !gaps.is_empty() {
        gaps.sort_unstable();
        let avg = gaps.iter().sum::<u64>() as f64 / gaps.len() as f64;
        ui.monospace(format!(
            "Spacing: min {} · max {} · avg {avg:.1} TS packets; average {:.1} bytes",
            gaps[0],
            gaps[gaps.len() - 1],
            avg * 188.0
        ));
        let graph = Series {
            name: "Random-access spacing",
            color: contrast_color(ui, 0),
            points: gaps
                .iter()
                .enumerate()
                .map(|(index, &gap)| (index as f64, gap as f64))
                .collect(),
        };
        plot(
            ui,
            "Access-point spacing",
            "Access-point index",
            "TS packets",
            &[graph],
        );
    }
    egui::CollapsingHeader::new("Access-point list").show(ui, |ui| {
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for (packet, pid) in points.iter().take(10_000) {
                    ui.monospace(format!(
                        "Packet {packet} · byte {} · PID 0x{pid:04X}",
                        packet * 188
                    ));
                }
            });
    });
    ui.weak("These are TS random_access_indicator flags, not verified IDR/IRAP or complete GOP structure. GOP frame lists and slice bytes require elementary-stream indexing.");
}
