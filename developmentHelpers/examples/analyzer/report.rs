use std::env;
use std::error::Error;
use std::path::PathBuf;

use tsan_analyzer::{ComplianceProfile, analyze_file, tr101290_report};

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: cargo run -p tsan-analyzer --example report -- INPUT.ts")?;
    let report = analyze_file(&path)?;
    println!(
        "format={:?} standard={:?} signalled_modulation={:?} packets={} malformed={} trailing={} null={} pids={} programs={} sections={} crc_errors={}",
        report.format,
        report.standard,
        report.signalled_modulation,
        report.packets,
        report.malformed_packets,
        report.trailing_bytes,
        report.null_packets,
        report.pids.len(),
        report.programs.len(),
        report.valid_sections,
        report.section_crc_errors
    );
    if let Some(native) = report.native {
        println!(
            "tsduck_packets={} tsduck_sections={} tsduck_cc_errors={} standards=0x{:04x}",
            native.packets, native.valid_sections, native.continuity_errors, native.standards
        );
    }
    let media = report.media_information();
    println!(
        "video={:?} services={} size={:?}x{:?} frame_rate={:?} audio={:?} tracks={}",
        media.video_codecs,
        media.video_services,
        media.video_width,
        media.video_height,
        media.video_frame_rate,
        media.audio_codecs,
        media.audio_tracks
    );
    for (pid, data) in &report.pids {
        if data.pcr_samples > 0 || data.continuity_errors > 0 {
            println!(
                "pid=0x{pid:04x} cc={} pcr_samples={} pcr_rep={} pcr_disc={} pcr_accuracy={}",
                data.continuity_errors,
                data.pcr_samples,
                data.pcr_repetition_errors,
                data.pcr_discontinuity_errors,
                data.pcr_accuracy_errors
            );
        }
    }
    let compliance = tr101290_report(&report, ComplianceProfile::suggested(&report));
    println!("compliance_profile={}", compliance.profile.label());
    for indicator in &compliance.indicators {
        println!(
            "compliance[{}] {}={} observed={:?}",
            indicator.group,
            indicator.name,
            indicator.status.label(),
            indicator.observed
        );
    }
    for event in compliance.events.iter().take(5) {
        println!(
            "event{} offset=0x{:X} pid=0x{:04X} {}: {}",
            if event.exact_packet { "" } else { "~" },
            report.packet_offset(event.packet_index),
            event.pid,
            event.indicator,
            event.detail
        );
    }
    for (number, program) in report.programs {
        println!(
            "program={number} pmt_pid=0x{:04x} pcr_pid={:?} streams={}",
            program.pmt_pid,
            program.pcr_pid,
            program.streams.len()
        );
        for (pid, stream) in program.streams {
            println!(
                "  pid=0x{pid:04x} type=0x{:02x} {}",
                stream.stream_type,
                stream.name_for_standard(report.standard)
            );
        }
    }
    Ok(())
}
