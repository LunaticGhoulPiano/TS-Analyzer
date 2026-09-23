use crate::{AnalysisReport, ClockKind};
use std::collections::BTreeMap;
pub struct BitrateSeries {
    pub pcr_pid: u16,
    pub median_mbps: f64,
    pub window_mbps: Vec<f64>,
}
pub fn bitrate_series(report: &AnalysisReport) -> Option<BitrateSeries> {
    let mut clocks: BTreeMap<u16, Vec<(u64, u64)>> = BTreeMap::new();
    for point in report
        .clock_points
        .iter()
        .filter(|p| p.kind == ClockKind::Pcr)
    {
        clocks
            .entry(point.pid)
            .or_default()
            .push((point.packet_index, point.ticks));
    }
    let (&pid, pcr) = clocks.iter().max_by_key(|(_, v)| v.len())?;
    let rate = |a: (u64, u64), b: (u64, u64)| {
        let delta = (b.1 + ((1_u64 << 33) * 300) - a.1) % ((1_u64 << 33) * 300);
        (delta > 0 && delta <= 54_000_000 && b.0 > a.0)
            .then(|| (b.0 - a.0) as f64 * 188.0 * 8.0 * 27.0 / delta as f64)
    };
    let mut rates = pcr
        .windows(2)
        .filter_map(|p| rate(p[0], p[1]))
        .collect::<Vec<_>>();
    if rates.is_empty() {
        return None;
    }
    rates.sort_by(f64::total_cmp);
    let median_mbps = rates[rates.len() / 2];
    let mut cursor = 0;
    let mut window_mbps = Vec::new();
    for window in &report.bitrate_windows {
        while cursor < pcr.len() && pcr[cursor].0 < window.first_packet {
            cursor += 1;
        }
        let mut at = cursor;
        let end = window.first_packet + u64::from(window.packet_count);
        let mut sum = 0.0;
        let mut count = 0;
        while at + 1 < pcr.len() && pcr[at + 1].0 < end {
            if let Some(v) = rate(pcr[at], pcr[at + 1]) {
                sum += v;
                count += 1;
            }
            at += 1;
        }
        window_mbps.push(if count == 0 {
            median_mbps
        } else {
            sum / f64::from(count)
        });
    }
    Some(BitrateSeries {
        pcr_pid: pid,
        median_mbps,
        window_mbps,
    })
}
