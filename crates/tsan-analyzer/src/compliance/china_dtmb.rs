use crate::transport::AnalysisReport;

use super::table_rules::{ProfileCheckReport, TableRule, check_pid_tables, check_table};

const GROUP: &str = "DTMB — China DTV SI";
const REFERENCE: &str = "GB/T 28161-2011; GB/T 17975.1-2010";
const EIT_IDS: &[u8] = &[
    0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d,
    0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d,
    0x6e, 0x6f,
];

fn rule(
    indicator: &'static str,
    pid: u16,
    table_ids: &'static [u8],
    required: bool,
    max_interval_seconds: f64,
    note: &'static str,
) -> TableRule {
    TableRule {
        group: GROUP,
        indicator,
        pid,
        table_ids,
        required,
        max_interval_seconds,
        note,
        reference: REFERENCE,
    }
}

pub(super) fn analyze(report: &AnalysisReport) -> ProfileCheckReport {
    let mut result = ProfileCheckReport::default();
    for (indicator, pid, ids, note) in [
        (
            "NIT_PID_error",
            0x0010,
            &[0x40, 0x41][..],
            "NIT table IDs must use PID 0x0010",
        ),
        (
            "SDT_BAT_PID_error",
            0x0011,
            &[0x42, 0x46, 0x4a][..],
            "SDT/BAT table IDs must use PID 0x0011",
        ),
        (
            "EIT_PID_error",
            0x0012,
            EIT_IDS,
            "EIT table IDs must use PID 0x0012",
        ),
        (
            "TDT_TOT_PID_error",
            0x0014,
            &[0x70, 0x73][..],
            "TDT/TOT table IDs must use PID 0x0014",
        ),
    ] {
        result.extend(check_pid_tables(
            report, GROUP, indicator, pid, ids, note, REFERENCE,
        ));
    }

    for table in [
        rule(
            "NIT_actual_error",
            0x0010,
            &[0x40],
            true,
            10.0,
            "NIT actual completeness and maximum ten-second repetition",
        ),
        rule(
            "SDT_actual_error",
            0x0011,
            &[0x42],
            true,
            2.0,
            "SDT actual completeness and maximum two-second repetition",
        ),
        rule(
            "EIT_actual_error",
            0x0012,
            &[0x4e],
            true,
            2.0,
            "EIT present/following actual completeness and maximum two-second repetition",
        ),
        rule(
            "TDT_TOT_error",
            0x0014,
            &[0x70, 0x73],
            true,
            30.0,
            "Time table completeness and maximum 30-second repetition",
        ),
        rule(
            "BAT_error",
            0x0011,
            &[0x4a],
            false,
            10.0,
            "Observed bouquet table completeness and repetition",
        ),
    ] {
        result.extend(check_table(report, table));
    }
    result
}
