use std::env;
use std::error::Error;
use std::path::PathBuf;

use tsan_analyzer::analyze_file;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: cargo run -p tsan-analyzer --example report -- INPUT.ts")?;
    let report = analyze_file(&path)?;
    println!(
        "format={:?} standard={:?} packets={} malformed={} trailing={} null={} pids={} programs={} sections={} crc_errors={}",
        report.format,
        report.standard,
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
