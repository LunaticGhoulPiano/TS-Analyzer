use std::collections::{BTreeMap, BTreeSet};

use crate::transport::{AnalysisReport, ClockKind, TrEvent};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tr101290Rules {
    pub pat_repetition_seconds: f64,
    pub pmt_repetition_seconds: f64,
    pub pat_note: &'static str,
    pub pmt_note: &'static str,
}
impl Tr101290Rules {
    pub(crate) const MPEG_TS: Self = Self {
        pat_repetition_seconds: 0.5,
        pmt_repetition_seconds: 0.5,
        pat_note: "Presence, table ID, scrambling and 500 ms repetition",
        pmt_note: "Presence, table ID, scrambling and 500 ms repetition",
    };
    pub(crate) const ATSC_A78: Self = Self {
        pat_repetition_seconds: 0.1,
        pmt_repetition_seconds: 0.4,
        pat_note: "Presence, table ID, scrambling and 100 ms A/78 repetition overlay",
        pmt_note: "Presence, table ID, scrambling and 400 ms A/78 repetition overlay",
    };
}
#[derive(Clone, Debug)]
pub(super) struct Tr101290Indicator {
    pub priority: u8,
    pub name: &'static str,
    pub observed: Option<u64>,
    pub note: &'static str,
}

#[derive(Clone, Debug)]
pub(super) struct Tr101290RawReport {
    pub indicators: Vec<Tr101290Indicator>,
    pub events: Vec<TrEvent>,
}

pub(crate) fn packet_rate(report: &AnalysisReport) -> Option<f64> {
    let pcr_pid = report
        .programs
        .values()
        .find_map(|program| program.pcr_pid)?;
    let points = report
        .clock_points
        .iter()
        .filter(|point| point.kind == ClockKind::Pcr && point.pid == pcr_pid)
        .collect::<Vec<_>>();
    const WRAP: u64 = (1_u64 << 33) * 300;
    let mut rates = points
        .windows(2)
        .filter_map(|pair| {
            let ticks = (pair[1].ticks + WRAP - pair[0].ticks) % WRAP;
            let packets = pair[1].packet_index.saturating_sub(pair[0].packet_index);
            (ticks > 0 && ticks <= 54_000_000 && packets > 0)
                .then_some(packets as f64 * 27_000_000.0 / ticks as f64)
        })
        .collect::<Vec<_>>();
    rates.sort_by(f64::total_cmp);
    rates.get(rates.len() / 2).copied()
}

fn section_gap_errors(
    report: &AnalysisReport,
    pid: u16,
    table_id: u8,
    max_seconds: f64,
    packets_per_second: Option<f64>,
) -> Option<u64> {
    let rate = packets_per_second?;
    let events = report
        .section_events
        .iter()
        .filter(|event| event.pid == pid && event.table_id == table_id)
        .collect::<Vec<_>>();
    if events.is_empty() {
        return None;
    }

    let mut previous = BTreeMap::new();
    let mut errors = 0;
    for event in &events {
        let key = (event.extension, event.section_number);
        if let Some(last) = previous.insert(key, event.packet_index) {
            let seconds = event.packet_index.saturating_sub(last) as f64 / rate;
            if !(0.025..=max_seconds).contains(&seconds) {
                errors += 1;
            }
        }
    }
    for last in previous.values() {
        if report.packets.saturating_sub(*last) as f64 / rate > max_seconds {
            errors += 1;
        }
    }
    Some(errors)
}

fn wrong_table_ids(report: &AnalysisReport, pid: u16, allowed: &[u8]) -> u64 {
    report
        .section_events
        .iter()
        .filter(|event| event.pid == pid && !allowed.contains(&event.table_id))
        .count() as u64
}

fn combine_observations(definite: u64, timed: Option<u64>) -> Option<u64> {
    match (definite, timed) {
        (0, None) => None,
        (value, None) => Some(value),
        (value, Some(timed)) => Some(value + timed),
    }
}

fn sum_observations(values: impl IntoIterator<Item = Option<u64>>) -> Option<u64> {
    let mut total = 0;
    let mut unknown = false;
    for value in values {
        match value {
            Some(value) => total += value,
            None => unknown = true,
        }
    }
    if total > 0 || !unknown {
        Some(total)
    } else {
        None
    }
}

fn pts_occurrence_errors(
    report: &AnalysisReport,
    rate: Option<f64>,
) -> (Option<u64>, Vec<TrEvent>) {
    let Some(rate) = rate else {
        return (None, Vec::new());
    };
    let mut first_pmt = BTreeMap::new();
    for program in report.programs.values() {
        let first = report
            .section_events
            .iter()
            .find(|event| event.pid == program.pmt_pid && event.table_id == 2)
            .map_or(0, |event| event.packet_index);
        for pid in program.streams.keys() {
            first_pmt.insert(*pid, first);
        }
    }
    let mut previous = BTreeMap::new();
    let mut events = Vec::new();
    let mut samples = 0;
    for point in report
        .clock_points
        .iter()
        .filter(|point| point.kind == ClockKind::Pts)
    {
        let Some(&activation) = first_pmt.get(&point.pid) else {
            continue;
        };
        if point.packet_index < activation {
            continue;
        }
        samples += 1;
        if let Some(last) = previous.insert(point.pid, point.packet_index) {
            let seconds = (point.packet_index - last) as f64 / rate;
            if seconds >= 0.7 {
                events.push(TrEvent {
                    packet_index: point.packet_index,
                    pid: point.pid,
                    indicator: "PTS_error",
                    detail: format!("PTS arrival interval {seconds:.3} s exceeds 700 ms"),
                    exact_packet: true,
                });
            }
        }
    }
    ((samples > 0).then_some(events.len() as u64), events)
}

fn pid_outage_errors(report: &AnalysisReport, rate: Option<f64>) -> (Option<u64>, Vec<TrEvent>) {
    let Some(rate) = rate else {
        return (None, Vec::new());
    };
    let mut events = Vec::new();
    let mut checked = BTreeSet::new();
    for program in report.programs.values() {
        let activation = report
            .section_events
            .iter()
            .find(|event| event.pid == program.pmt_pid && event.table_id == 2)
            .map_or(0, |event| event.packet_index);
        for pid in program.streams.keys().copied().chain(program.pcr_pid) {
            if !checked.insert(pid) {
                continue;
            }
            let mut absent_start = None;
            let mut reported = false;
            for window in report
                .bitrate_windows
                .iter()
                .filter(|window| window.first_packet + u64::from(window.packet_count) > activation)
            {
                let present = window.pid_packets.get(&pid).is_some_and(|count| *count > 0);
                if present {
                    absent_start = None;
                    reported = false;
                    continue;
                }
                let start = *absent_start.get_or_insert(window.first_packet.max(activation));
                let elapsed =
                    (window.first_packet + u64::from(window.packet_count) - start) as f64 / rate;
                if elapsed > 5.0 && !reported {
                    events.push(TrEvent {
                        packet_index: window.first_packet,
                        pid,
                        indicator: "PID_error",
                        detail: "Referenced PID absent for over 5 s after PMT activation"
                            .to_owned(),
                        exact_packet: false,
                    });
                    reported = true;
                }
            }
        }
    }
    (Some(events.len() as u64), events)
}

fn eit_pf_errors(report: &AnalysisReport) -> Option<u64> {
    let mut errors = 0;
    let mut observed = false;
    for id in [0x4e, 0x4f] {
        let Some(table) = report.tables.get(&(0x12, id)) else {
            continue;
        };
        observed = true;
        let mut pairs: BTreeMap<(u16, u8), BTreeSet<u8>> = BTreeMap::new();
        for &(extension, version, number) in table.instances.keys() {
            pairs
                .entry((extension, version))
                .or_default()
                .insert(number);
        }
        for numbers in pairs.values() {
            if !numbers.contains(&0) || !numbers.contains(&1) {
                errors += 1;
            }
        }
    }
    observed.then_some(errors)
}

pub(super) fn analyze(report: &AnalysisReport, rules: Tr101290Rules) -> Tr101290RawReport {
    let rate = packet_rate(report);
    let table = |pid, id| report.tables.contains_key(&(pid, id));
    let sum_pid = |value: fn(&crate::transport::PidReport) -> u64| {
        report.pids.values().map(value).sum::<u64>()
    };
    let pmt_pids = report
        .programs
        .values()
        .map(|program| program.pmt_pid)
        .collect::<BTreeSet<_>>();
    let mut referenced =
        BTreeSet::from([0, 1, 2, 0x10, 0x11, 0x12, 0x13, 0x14, 0x1e, 0x1f, 0x1fff]);
    referenced.extend(&pmt_pids);
    for program in report.programs.values() {
        referenced.extend(program.pcr_pid);
        referenced.extend(program.streams.keys().copied());
    }
    let unreferenced_pids = report
        .pids
        .iter()
        .filter(|(pid, data)| {
            !referenced.contains(pid)
                && data.first_packet.is_some_and(|first| {
                    rate.is_none_or(|rate| (report.packets - first) as f64 / rate > 0.5)
                })
        })
        .collect::<Vec<_>>();
    let unreferenced = unreferenced_pids.len() as u64;
    let (pid_errors, pid_events) = pid_outage_errors(report, rate);
    let (pts_errors, pts_events) = pts_occurrence_errors(report, rate);
    let pat_definite = u64::from(!table(0, 0))
        + wrong_table_ids(report, 0, &[0])
        + report.pids.get(&0).map_or(0, |pid| pid.scrambled_packets);
    let pat_errors = combine_observations(
        pat_definite,
        section_gap_errors(report, 0, 0, rules.pat_repetition_seconds, rate),
    );
    let pmt_errors = if report.programs.is_empty() {
        None
    } else {
        sum_observations(report.programs.values().map(|program| {
            let definite = u64::from(!table(program.pmt_pid, 2))
                + wrong_table_ids(report, program.pmt_pid, &[2])
                + report
                    .pids
                    .get(&program.pmt_pid)
                    .map_or(0, |pid| pid.scrambled_packets);
            combine_observations(
                definite,
                section_gap_errors(
                    report,
                    program.pmt_pid,
                    2,
                    rules.pmt_repetition_seconds,
                    rate,
                ),
            )
        }))
    };
    let scrambled = sum_pid(|pid| pid.scrambled_packets);
    let cat_errors = wrong_table_ids(report, 1, &[1]) + u64::from(scrambled > 0 && !table(1, 1));
    let pcr_samples = sum_pid(|pid| pid.pcr_samples);
    let nit_actual = combine_observations(
        wrong_table_ids(report, 0x10, &[0x40, 0x41, 0x72]),
        section_gap_errors(report, 0x10, 0x40, 10.0, rate),
    );
    let nit_other = section_gap_errors(report, 0x10, 0x41, 10.0, rate);
    let sdt_actual = combine_observations(
        wrong_table_ids(report, 0x11, &[0x42, 0x46, 0x4a, 0x72]),
        section_gap_errors(report, 0x11, 0x42, 2.0, rate),
    );
    let sdt_other = section_gap_errors(report, 0x11, 0x46, 10.0, rate);
    let eit_pf = eit_pf_errors(report);
    let eit_bad = wrong_table_ids(
        report,
        0x12,
        &(0x4e..=0x6f).chain([0x72]).collect::<Vec<_>>(),
    );
    let eit_actual =
        combine_observations(eit_bad, section_gap_errors(report, 0x12, 0x4e, 2.0, rate));
    let eit_other = section_gap_errors(report, 0x12, 0x4f, 10.0, rate);
    let rst = report
        .section_events
        .iter()
        .any(|event| event.pid == 0x13)
        .then(|| wrong_table_ids(report, 0x13, &[0x71, 0x72]));
    let tdt = combine_observations(
        wrong_table_ids(report, 0x14, &[0x70, 0x72, 0x73]),
        section_gap_errors(report, 0x14, 0x70, 30.0, rate),
    );
    let si_repetition = sum_observations(
        [
            (0x10, 0x40, 10.0),
            (0x10, 0x41, 10.0),
            (0x11, 0x42, 2.0),
            (0x11, 0x46, 10.0),
            (0x12, 0x4e, 2.0),
            (0x12, 0x4f, 10.0),
            (0x14, 0x70, 30.0),
            (0x14, 0x73, 30.0),
        ]
        .iter()
        .map(|&(pid, id, limit)| section_gap_errors(report, pid, id, limit, rate)),
    );
    let indicators = vec![
        Tr101290Indicator {
            priority: 1,
            name: "TS_sync_loss",
            observed: Some(report.sync_loss_events),
            note: "Consecutive aligned sync-byte failures",
        },
        Tr101290Indicator {
            priority: 1,
            name: "Sync_byte_error",
            observed: Some(report.sync_byte_errors),
            note: "Invalid 0x47 sync byte",
        },
        Tr101290Indicator {
            priority: 1,
            name: "PAT_error",
            observed: pat_errors,
            note: rules.pat_note,
        },
        Tr101290Indicator {
            priority: 1,
            name: "Continuity_count_error",
            observed: Some(sum_pid(|pid| pid.continuity_errors)),
            note: "Excludes null PID and exact duplicates",
        },
        Tr101290Indicator {
            priority: 1,
            name: "PMT_error",
            observed: pmt_errors,
            note: rules.pmt_note,
        },
        Tr101290Indicator {
            priority: 1,
            name: "PID_error",
            observed: pid_errors,
            note: "Referenced ES/PCR PID absent for over 5 s after PMT activation (1024-packet bins)",
        },
        Tr101290Indicator {
            priority: 2,
            name: "Transport_error",
            observed: Some(sum_pid(|pid| pid.transport_errors)),
            note: "Transport error indicator set",
        },
        Tr101290Indicator {
            priority: 2,
            name: "CRC_error",
            observed: Some(report.section_crc_errors),
            note: "MPEG section CRC failure",
        },
        Tr101290Indicator {
            priority: 2,
            name: "PCR_repetition_error",
            observed: (pcr_samples > 0).then(|| sum_pid(|pid| pid.pcr_repetition_errors)),
            note: "Consecutive program PCR values at least 40 ms apart",
        },
        Tr101290Indicator {
            priority: 2,
            name: "PCR_discontinuity_indicator_error",
            observed: (pcr_samples > 0).then(|| sum_pid(|pid| pid.pcr_discontinuity_errors)),
            note: "PCR gap over 100 ms or backward without discontinuity flag",
        },
        Tr101290Indicator {
            priority: 2,
            name: "PCR_accuracy_error",
            observed: (pcr_samples > 0).then(|| sum_pid(|pid| pid.pcr_accuracy_errors)),
            note: "Adjacent PCR packet-rate extrapolation; deviation at least 500 ns",
        },
        Tr101290Indicator {
            priority: 2,
            name: "PTS_error",
            observed: pts_errors,
            note: "Consecutive PTS arrivals over 700 ms apart, using PCR-derived packet rate",
        },
        Tr101290Indicator {
            priority: 2,
            name: "CAT_error",
            observed: Some(cat_errors),
            note: "CAT table ID or missing CAT with scrambled packets",
        },
        Tr101290Indicator {
            priority: 3,
            name: "NIT_actual_error",
            observed: nit_actual,
            note: "PID 0x0010 table ID and NIT actual repetition",
        },
        Tr101290Indicator {
            priority: 3,
            name: "NIT_other_error",
            observed: nit_other,
            note: "NIT other repetition",
        },
        Tr101290Indicator {
            priority: 3,
            name: "SI_repetition_error",
            observed: si_repetition,
            note: "Observed section intervals outside 25 ms to table-specific maximum",
        },
        Tr101290Indicator {
            priority: 3,
            name: "Unreferenced_PID",
            observed: Some(unreferenced),
            note: "DVB PSI reference profile; ATSC PSIP PID 0x1FFB is counted here",
        },
        Tr101290Indicator {
            priority: 3,
            name: "SDT_actual_error",
            observed: sdt_actual,
            note: "SDT PID table ID and actual repetition",
        },
        Tr101290Indicator {
            priority: 3,
            name: "SDT_other_error",
            observed: sdt_other,
            note: "SDT other repetition",
        },
        Tr101290Indicator {
            priority: 3,
            name: "EIT_actual_error",
            observed: eit_actual,
            note: "EIT PID table ID and present/following actual repetition",
        },
        Tr101290Indicator {
            priority: 3,
            name: "EIT_other_error",
            observed: eit_other,
            note: "EIT present/following other repetition",
        },
        Tr101290Indicator {
            priority: 3,
            name: "EIT_PF_error",
            observed: eit_pf,
            note: "Observed EIT P/F section 0 and 1 pairing; unobserved services are not checked",
        },
        Tr101290Indicator {
            priority: 3,
            name: "RST_error",
            observed: rst,
            note: "RST PID table ID",
        },
        Tr101290Indicator {
            priority: 3,
            name: "TDT_error",
            observed: tdt,
            note: "TDT PID table ID and repetition",
        },
    ];
    let mut events = report.tr_events.clone();
    events.extend(pid_events);
    events.extend(pts_events);
    for (&pid, data) in unreferenced_pids {
        if let Some(first) = data.first_packet {
            events.push(TrEvent {
                packet_index: first + rate.map_or(0, |rate| (rate * 0.5).ceil() as u64),
                pid,
                indicator: "Unreferenced_PID",
                detail: "DVB reference profile: PID remained unreferenced for over 500 ms (threshold offset estimated from PCR)".to_owned(),
                exact_packet: false,
            });
        }
    }
    events.sort_by_key(|event| event.packet_index);
    Tr101290RawReport { indicators, events }
}
