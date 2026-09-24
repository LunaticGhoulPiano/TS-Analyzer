//! Application services shared by native frontends and future CLI integration.
//! Bounded live-stream fan-out remains planned; `pipeline_spike` is its experiment.

use std::io;
use std::path::Path;

pub use tsan_analyzer::AnalysisReport;

#[derive(Clone, Copy, Default)]
pub struct AnalysisService;

impl AnalysisService {
    pub fn analyze(&self, path: &Path) -> io::Result<AnalysisReport> {
        self.analyze_with_progress(path, |_, _| true)
    }

    pub fn analyze_with_progress(
        &self,
        path: &Path,
        progress: impl FnMut(u64, u64) -> bool,
    ) -> io::Result<AnalysisReport> {
        tsan_diagnostics::record(
            "INFO",
            "Analysis",
            &format!("Analysis started: {}", path.display()),
        );
        let result = tsan_analyzer::analyze_file_with_progress(path, progress);
        match &result {
            Ok(report) => tsan_diagnostics::record(
                "INFO",
                "Analysis",
                &format!(
                    "Analysis completed: {} packets; {}",
                    report.packets,
                    path.display()
                ),
            ),
            Err(error) => tsan_diagnostics::record(
                "ERROR",
                "Analysis",
                &format!("Analysis ended: {}; {}", error, path.display()),
            ),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_returns_analysis_and_propagates_progress_and_cancellation() -> io::Result<()> {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../developmentHelpers/test-data/inputs/synthetic/transport_detect_packet_size_188.ts");
        let mut progress = Vec::new();
        let report = AnalysisService.analyze_with_progress(&fixture, |read, total| {
            progress.push((read, total));
            true
        })?;
        assert!(report.packets > 0);
        assert!(progress.last().is_some_and(|(read, total)| read == total));
        let cancelled = AnalysisService.analyze_with_progress(&fixture, |_, _| false);
        assert!(matches!(cancelled, Err(error) if error.kind() == io::ErrorKind::Interrupted));
        let missing = AnalysisService.analyze(&fixture.with_extension("missing"));
        assert!(matches!(missing, Err(error) if error.kind() == io::ErrorKind::NotFound));
        Ok(())
    }
}
