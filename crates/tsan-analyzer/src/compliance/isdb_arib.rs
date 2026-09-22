use crate::transport::AnalysisReport;

use super::table_rules::{ProfileCheckReport, TableRule, check_pid_tables, check_table};

const EIT_IDS: &[u8] = &[
    0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d,
    0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d,
    0x6e, 0x6f,
];

#[derive(Clone, Copy)]
pub(super) enum ProfileKind {
    Japan,
    International,
    J83C,
}

impl ProfileKind {
    const fn group(self) -> &'static str {
        match self {
            Self::Japan => "ISDB-T Japan — ARIB SI",
            Self::International => "ISDB-T International — ABNT SI",
            Self::J83C => "J.83 Annex C — ARIB-compatible SI",
        }
    }

    const fn reference(self) -> &'static str {
        match self {
            Self::Japan => "ARIB STD-B10; ARIB TR-B14",
            Self::International => "ABNT NBR 15603; ABNT NBR 15608",
            Self::J83C => "ARIB STD-B10; ARIB TR-B15; ITU-T J.83 Annex C",
        }
    }
}

fn rule(
    profile: ProfileKind,
    indicator: &'static str,
    pid: u16,
    table_ids: &'static [u8],
    required: bool,
    max_interval_seconds: f64,
    note: &'static str,
) -> TableRule {
    TableRule {
        group: profile.group(),
        indicator,
        pid,
        table_ids,
        required,
        max_interval_seconds,
        note,
        reference: profile.reference(),
    }
}

pub(super) fn analyze(report: &AnalysisReport, profile: ProfileKind) -> ProfileCheckReport {
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
        (
            "SDTT_PID_error",
            0x0023,
            &[0xc3][..],
            "SDTT must use PID 0x0023",
        ),
        (
            "BIT_PID_error",
            0x0024,
            &[0xc4][..],
            "BIT must use PID 0x0024",
        ),
        (
            "CDT_PID_error",
            0x0029,
            &[0xc8][..],
            "CDT must use PID 0x0029",
        ),
    ] {
        result.extend(check_pid_tables(
            report,
            profile.group(),
            indicator,
            pid,
            ids,
            note,
            profile.reference(),
        ));
    }

    for table in [
        rule(
            profile,
            "NIT_actual_error",
            0x0010,
            &[0x40],
            true,
            10.0,
            "NIT actual completeness and maximum ten-second repetition",
        ),
        rule(
            profile,
            "SDT_actual_error",
            0x0011,
            &[0x42],
            true,
            2.0,
            "SDT actual completeness and maximum two-second repetition",
        ),
        rule(
            profile,
            "EIT_actual_error",
            0x0012,
            &[0x4e],
            true,
            2.0,
            "EIT present/following actual completeness and maximum two-second repetition",
        ),
        rule(
            profile,
            "TDT_TOT_error",
            0x0014,
            &[0x70, 0x73],
            true,
            30.0,
            "Time table completeness and maximum 30-second repetition",
        ),
        rule(
            profile,
            "BIT_error",
            0x0024,
            &[0xc4],
            true,
            10.0,
            "BIT completeness and maximum ten-second repetition",
        ),
        rule(
            profile,
            "SDTT_error",
            0x0023,
            &[0xc3],
            false,
            10.0,
            "Observed SDTT completeness and repetition",
        ),
        rule(
            profile,
            "CDT_error",
            0x0029,
            &[0xc8],
            false,
            10.0,
            "Observed CDT completeness and repetition",
        ),
    ] {
        result.extend(check_table(report, table));
    }
    result
}
