#[cfg(target_os = "windows")]
mod analysis_views;
#[cfg(target_os = "windows")]
mod app;
#[cfg(target_os = "windows")]
mod logging;
#[cfg(target_os = "windows")]
mod platform;
#[cfg(target_os = "windows")]
mod player_worker;
#[cfg(target_os = "windows")]
mod report_export;

#[cfg(target_os = "windows")]
use std::env;
#[cfg(target_os = "windows")]
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use eframe::egui;

#[cfg(target_os = "windows")]
fn main() -> eframe::Result {
    let initial_input = env::args_os().nth(1).map(PathBuf::from);
    let mut wgpu_options = eframe::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = eframe::wgpu::Backends::GL;
    }

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        viewport: egui::ViewportBuilder::default()
            .with_title("TS Analyzer")
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
