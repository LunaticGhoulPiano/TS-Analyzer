use std::collections::BTreeSet;

use crate::transport::{AnalysisReport, TrEvent};

use super::table_rules::{ProfileCheckReport, TableRule, check_pid_tables, check_table};
use super::{ComplianceIndicator, measured_status};

const GROUP: &str = "ATSC 1.0 / J.83B — PSIP";
const PSIP_TABLE_IDS: &[u8] = &[0xc7, 0xc8, 0xc9, 0xca, 0xcb, 0xcc, 0xcd];

#[derive(Clone, Copy)]
pub(super) enum ServiceType {
    Terrestrial,
    Cable,
}

fn table_rule(
    indicator: &'static str,
    table_ids: &'static [u8],
    required: bool,
    max_interval_seconds: f64,
    note: &'static str,
    reference: &'static str,
) -> TableRule {
    TableRule {
        group: GROUP,
        indicator,
        pid: 0x1ffb,
        table_ids,
        required,
        max_interval_seconds,
        note,
        reference,
    }
}

fn transport_stream_id_check(
    report: &AnalysisReport,
    vct_table_id: u8,
    reference: &'static str,
) -> ProfileCheckReport {
    let pat_ids = report
        .tables
        .get(&(0, 0))
        .into_iter()
        .flat_map(|table| table.instances.keys().map(|(extension, _, _)| *extension))
        .collect::<BTreeSet<_>>();
    let vct_ids = report
        .tables
        .get(&(0x1ffb, vct_table_id))
        .into_iter()
        .flat_map(|table| table.instances.keys().map(|(extension, _, _)| *extension))
        .collect::<BTreeSet<_>>();
    let observed = if pat_ids.is_empty() || vct_ids.is_empty() {
        None
    } else {
        Some(
            vct_ids
                .iter()
                .filter(|extension| !pat_ids.contains(extension))
                .count() as u64,
        )
    };
    let events = vct_ids
        .iter()
        .filter(|extension| !pat_ids.contains(extension))
        .filter_map(|extension| {
            report
                .section_events
                .iter()
                .find(|event| {
                    event.pid == 0x1ffb
                        && event.table_id == vct_table_id
                        && event.extension == *extension
                })
                .map(|event| TrEvent {
                    packet_index: event.packet_index,
                    pid: event.pid,
                    indicator: "VCT_transport_stream_id_error",
                    detail: format!(
                        "VCT transport_stream_id 0x{:04X} is absent from the PAT",
                        extension
                    ),
                    exact_packet: true,
                })
        })
        .collect();
    ProfileCheckReport {
        indicators: vec![ComplianceIndicator {
            group: GROUP,
            name: "VCT_transport_stream_id_error",
            observed,
            status: measured_status(observed),
            note: "VCT transport_stream_id must identify the MPEG-2 TS carried by the PAT",
            reference,
        }],
        events,
    }
}

pub(super) fn analyze(
    report: &AnalysisReport,
    service_type: ServiceType,
    reference: &'static str,
) -> ProfileCheckReport {
    let mut result = ProfileCheckReport::default();
    result.extend(check_pid_tables(
        report,
        GROUP,
        "PSIP_PID_error",
        0x1ffb,
        PSIP_TABLE_IDS,
        "ATSC PSIP tables must use base PID 0x1FFB",
        reference,
    ));
    result.extend(check_table(
        report,
        table_rule(
            "MGT_error",
            &[0xc7],
            true,
            0.15,
            "MGT completeness and maximum 150 ms repetition",
            reference,
        ),
    ));
    let (vct_id, vct_ids, vct_name) = match service_type {
        ServiceType::Terrestrial => (0xc8, &[0xc8][..], "TVCT_error"),
        ServiceType::Cable => (0xc9, &[0xc9][..], "CVCT_error"),
    };
    result.extend(check_table(
        report,
        table_rule(
            vct_name,
            vct_ids,
            true,
            0.4,
            "VCT completeness and maximum 400 ms repetition",
            reference,
        ),
    ));
    result.extend(check_table(
        report,
        table_rule(
            "STT_error",
            &[0xcd],
            true,
            1.0,
            "STT completeness and maximum one-second repetition",
            reference,
        ),
    ));
    result.extend(check_table(
        report,
        table_rule(
            "RRT_error",
            &[0xca],
            true,
            60.0,
            "RRT completeness and maximum 60-second repetition",
            reference,
        ),
    ));
    result.extend(check_table(
        report,
        table_rule(
            "EIT_error",
            &[0xcb],
            true,
            60.0,
            "EIT completeness and maximum 60-second repetition",
            reference,
        ),
    ));
    result.extend(check_table(
        report,
        table_rule(
            "ETT_error",
            &[0xcc],
            false,
            10.0,
            "Observed ETT completeness and maximum ten-second repetition",
            reference,
        ),
    ));
    result.extend(transport_stream_id_check(report, vct_id, reference));
    result
}
