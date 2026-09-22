use std::collections::{BTreeMap, BTreeSet};

use crate::tr101290::packet_rate;
use crate::transport::{AnalysisReport, SectionEvent, TrEvent};

use super::{ComplianceIndicator, measured_status};

#[derive(Default)]
pub(super) struct ProfileCheckReport {
    pub indicators: Vec<ComplianceIndicator>,
    pub events: Vec<TrEvent>,
}

impl ProfileCheckReport {
    pub(super) fn extend(&mut self, mut other: Self) {
        self.indicators.append(&mut other.indicators);
        self.events.append(&mut other.events);
    }
}

pub(super) struct TableRule {
    pub group: &'static str,
    pub indicator: &'static str,
    pub pid: u16,
    pub table_ids: &'static [u8],
    pub required: bool,
    pub max_interval_seconds: f64,
    pub note: &'static str,
    pub reference: &'static str,
}

fn combined_observation(definite: u64, timed: Option<u64>) -> Option<u64> {
    match (definite, timed) {
        (0, None) => None,
        (value, None) => Some(value),
        (value, Some(timed)) => Some(value + timed),
    }
}

fn matching_events<'a>(
    report: &'a AnalysisReport,
    pid: u16,
    table_ids: &'static [u8],
) -> Vec<&'a SectionEvent> {
    report
        .section_events
        .iter()
        .filter(|event| event.pid == pid && table_ids.contains(&event.table_id))
        .collect()
}

fn completeness_errors(events: &[&SectionEvent]) -> (u64, Vec<TrEvent>) {
    let mut latest = BTreeMap::<(u8, u16), (u64, u8, u8)>::new();
    for event in events {
        let key = (event.table_id, event.extension);
        let candidate = (event.packet_index, event.version, event.last_section_number);
        if latest
            .get(&key)
            .is_none_or(|current| candidate.0 >= current.0)
        {
            latest.insert(key, candidate);
        }
    }

    let mut count = 0;
    let mut failures = Vec::new();
    for ((table_id, extension), (_, version, last_section_number)) in latest {
        let observed = events
            .iter()
            .filter(|event| {
                event.table_id == table_id
                    && event.extension == extension
                    && event.version == version
            })
            .map(|event| event.section_number)
            .collect::<BTreeSet<_>>();
        for section_number in 0..=last_section_number {
            if observed.contains(&section_number) {
                continue;
            }
            count += 1;
            let packet_index = events
                .iter()
                .filter(|event| event.table_id == table_id && event.extension == extension)
                .map(|event| event.packet_index)
                .max()
                .unwrap_or(0);
            failures.push(TrEvent {
                packet_index,
                pid: events.first().map_or(0, |event| event.pid),
                indicator: "",
                detail: format!(
                    "Table 0x{table_id:02X}, extension 0x{extension:04X}, version {version} is missing section {section_number}/{last_section_number}"
                ),
                exact_packet: false,
            });
        }
    }
    (count, failures)
}

fn repetition_errors(
    report: &AnalysisReport,
    events: &[&SectionEvent],
    max_seconds: f64,
) -> (Option<u64>, Vec<TrEvent>) {
    let Some(rate) = packet_rate(report) else {
        return (None, Vec::new());
    };
    let mut groups = BTreeMap::<(u8, u16, u8), Vec<&SectionEvent>>::new();
    for event in events {
        groups
            .entry((event.table_id, event.extension, event.section_number))
            .or_default()
            .push(event);
    }

    let mut count = 0;
    let mut failures = Vec::new();
    for ((table_id, extension, section_number), points) in groups {
        let first = points[0];
        let initial_seconds = first.packet_index as f64 / rate;
        if initial_seconds > max_seconds {
            count += 1;
            failures.push(TrEvent {
                packet_index: first.packet_index,
                pid: first.pid,
                indicator: "",
                detail: format!(
                    "First table 0x{table_id:02X} section {section_number}, extension 0x{extension:04X}, arrived after {initial_seconds:.3} s (limit {max_seconds:.3} s)"
                ),
                exact_packet: false,
            });
        }
        for pair in points.windows(2) {
            let seconds = pair[1].packet_index.saturating_sub(pair[0].packet_index) as f64 / rate;
            if seconds <= max_seconds {
                continue;
            }
            count += 1;
            failures.push(TrEvent {
                packet_index: pair[1].packet_index,
                pid: pair[1].pid,
                indicator: "",
                detail: format!(
                    "Table 0x{table_id:02X} section {section_number}, extension 0x{extension:04X}, repeated after {seconds:.3} s (limit {max_seconds:.3} s)"
                ),
                exact_packet: true,
            });
        }
        let last = points[points.len() - 1];
        let trailing_seconds = report.packets.saturating_sub(last.packet_index) as f64 / rate;
        if trailing_seconds > max_seconds {
            count += 1;
            failures.push(TrEvent {
                packet_index: report.packets.saturating_sub(1),
                pid: last.pid,
                indicator: "",
                detail: format!(
                    "Table 0x{table_id:02X} section {section_number}, extension 0x{extension:04X}, was absent for the final {trailing_seconds:.3} s (limit {max_seconds:.3} s)"
                ),
                exact_packet: false,
            });
        }
    }
    (Some(count), failures)
}

pub(super) fn check_table(report: &AnalysisReport, rule: TableRule) -> ProfileCheckReport {
    let events = matching_events(report, rule.pid, rule.table_ids);
    if events.is_empty() {
        let observed = if rule.required {
            packet_rate(report)
                .filter(|rate| report.packets as f64 / rate >= rule.max_interval_seconds)
                .map(|_| 1)
        } else {
            None
        };
        let mut failures = Vec::new();
        if observed.is_some() {
            failures.push(TrEvent {
                packet_index: report.packets.saturating_sub(1),
                pid: rule.pid,
                indicator: rule.indicator,
                detail: format!(
                    "Required table IDs {} were not observed on PID 0x{:04X}",
                    rule.table_ids
                        .iter()
                        .map(|id| format!("0x{id:02X}"))
                        .collect::<Vec<_>>()
                        .join("/"),
                    rule.pid
                ),
                exact_packet: false,
            });
        }
        return ProfileCheckReport {
            indicators: vec![ComplianceIndicator {
                group: rule.group,
                name: rule.indicator,
                observed,
                status: measured_status(observed),
                note: rule.note,
                reference: rule.reference,
            }],
            events: failures,
        };
    }

    let (definite, mut failures) = completeness_errors(&events);
    let (timed, mut timing_failures) =
        repetition_errors(report, &events, rule.max_interval_seconds);
    failures.append(&mut timing_failures);
    for event in &mut failures {
        event.indicator = rule.indicator;
    }
    let observed = combined_observation(definite, timed);
    ProfileCheckReport {
        indicators: vec![ComplianceIndicator {
            group: rule.group,
            name: rule.indicator,
            observed,
            status: measured_status(observed),
            note: rule.note,
            reference: rule.reference,
        }],
        events: failures,
    }
}

pub(super) fn check_pid_tables(
    report: &AnalysisReport,
    group: &'static str,
    indicator: &'static str,
    expected_pid: u16,
    table_ids: &'static [u8],
    note: &'static str,
    reference: &'static str,
) -> ProfileCheckReport {
    let mut saw_relevant = false;
    let mut failures = Vec::new();
    for event in &report.section_events {
        let relevant_id = table_ids.contains(&event.table_id);
        if event.pid == expected_pid || relevant_id {
            saw_relevant = true;
        }
        let wrong_id_on_pid = event.pid == expected_pid && !relevant_id && event.table_id != 0x72;
        let expected_id_on_wrong_pid = relevant_id && event.pid != expected_pid;
        if !wrong_id_on_pid && !expected_id_on_wrong_pid {
            continue;
        }
        failures.push(TrEvent {
            packet_index: event.packet_index,
            pid: event.pid,
            indicator,
            detail: if wrong_id_on_pid {
                format!(
                    "Unexpected table ID 0x{:02X} on reserved PID 0x{expected_pid:04X}",
                    event.table_id
                )
            } else {
                format!(
                    "Table ID 0x{:02X} appeared on PID 0x{:04X}; expected PID 0x{expected_pid:04X}",
                    event.table_id, event.pid
                )
            },
            exact_packet: true,
        });
    }

    let observed = saw_relevant.then_some(failures.len() as u64);
    ProfileCheckReport {
        indicators: vec![ComplianceIndicator {
            group,
            name: indicator,
            observed,
            status: measured_status(observed),
            note,
            reference,
        }],
        events: failures,
    }
}
