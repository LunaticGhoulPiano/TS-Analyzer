use std::error::Error;
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: gop_probe <TS directory>")?;
    let mut paths = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ts")))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let now = std::time::Instant::now();
        let report = tsan_analyzer::analyze_file(&path)?;
        if report.video_gops.is_empty() {
            return Err(format!("No video GOPs: {}", path.display()).into());
        }
        for (pid, video) in &report.video_gops {
            let complete = video.gops.iter().filter(|g| g.complete).collect::<Vec<_>>();
            let lengths = complete
                .iter()
                .map(|g| g.pictures.len())
                .collect::<std::collections::BTreeSet<_>>();
            let unknown = complete
                .iter()
                .flat_map(|g| &g.pictures)
                .filter(|p| p.label() == "?")
                .count();
            let example = complete.first().map(|g| g.structure()).unwrap_or_default();
            println!(
                "{} | PID {} | codec {:02X} | frames {} | complete {} | lengths {:?} | unknown {} | {} | {:.2}s",
                path.file_name().unwrap_or_default().to_string_lossy(),
                pid,
                video.stream_type,
                video.frame_count,
                complete.len(),
                lengths,
                unknown,
                example,
                now.elapsed().as_secs_f64()
            );
            if complete.is_empty() || unknown != 0 {
                return Err("Incomplete picture classification".into());
            }
        }
    }
    Ok(())
}
