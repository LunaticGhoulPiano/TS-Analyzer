mod config;
mod os_integration;
#[cfg(target_os = "windows")]
mod plot_range;
mod release;
#[cfg(target_os = "windows")]
mod report_graphics;
mod update;
#[cfg(target_os = "windows")]
mod xlsx;

#[cfg(target_os = "windows")]
mod analysis_views;
#[cfg(target_os = "windows")]
mod app;
#[cfg(target_os = "windows")]
mod liquid_glass;
#[cfg(target_os = "windows")]
mod logging;
#[cfg(target_os = "windows")]
mod platform;
#[cfg(target_os = "windows")]
mod player_worker;
#[cfg(target_os = "windows")]
mod report_export;
#[cfg(target_os = "windows")]
mod ui_components;

#[cfg(target_os = "windows")]
use std::env;
#[cfg(target_os = "windows")]
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use eframe::egui;

#[cfg(target_os = "windows")]
fn package_smoke(source: PathBuf, directory: PathBuf) -> Result<(), String> {
    use tsan_player::platform::windows::{GstreamerPlayerBackend, PlayerBackendKind};
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    for kind in [PlayerBackendKind::D3d11, PlayerBackendKind::D3d12] {
        let _backend = GstreamerPlayerBackend::new(kind).map_err(|e| e.to_string())?;
        if GstreamerPlayerBackend::discover_adapters(kind)
            .map_err(|e| e.to_string())?
            .is_empty()
        {
            return Err(format!("No GPU adapter for {kind}"));
        }
    }
    let input = [report_export::ExportInput {
        report: tsan_analyzer::analyze_file(&source).map_err(|e| e.to_string())?,
        path: source,
    }];
    for format in [
        report_export::ReportFormat::Cbor,
        report_export::ReportFormat::Xlsx,
        report_export::ReportFormat::Latex,
        report_export::ReportFormat::Pdf,
    ] {
        report_export::export(
            format,
            &directory.join(format!("report.{}", format.extension())),
            &input,
        )?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn main() -> eframe::Result {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|s| s == "--build-info") {
        if args.len() != 2 {
            std::process::exit(2);
        }
        let info = format!(
            "version = \"{}\"\ntarget = \"{}-{}\"\nstatus = \"{}\"\nminimum_os = \"{}\"\n",
            release::VERSION,
            std::env::consts::OS,
            std::env::consts::ARCH,
            release::STATUS,
            release::MINIMUM_OS
        );
        if std::fs::write(&args[1], info).is_err() {
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--package-smoke") {
        if args.len() != 3 {
            std::process::exit(2);
        }
        let directory = PathBuf::from(&args[2]);
        let result = package_smoke(PathBuf::from(&args[1]), directory.clone());
        let message = result
            .as_ref()
            .map(|_| {
                "PASS: TSDuck, GStreamer D3D11/D3D12, CBOR, XLSX and XeLaTeX report export"
                    .to_owned()
            })
            .unwrap_or_else(|e| format!("FAIL: {e}"));
        let _ = std::fs::create_dir_all(&directory);
        let _ = std::fs::write(directory.join("smoke-result.txt"), message);
        if result.is_err() {
            std::process::exit(1);
        }
        return Ok(());
    }
    let initial_input = args.first().map(PathBuf::from);
    let mut wgpu_options = eframe::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = eframe::wgpu::Backends::GL;
    }

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        viewport: egui::ViewportBuilder::default()
            .with_title("TS Analyzer")
            // Keep an alpha-capable surface for runtime theme switching; Light still clears opaquely.
            .with_transparent(true)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native(
        "TS Analyzer",
        options,
        Box::new(move |context| Ok(Box::new(app::TsanApp::new(context, initial_input)))),
    )
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("No native video backend is implemented for this operating system yet.");
}
