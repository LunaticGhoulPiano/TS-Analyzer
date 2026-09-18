use std::error::Error;
use std::io;

use gst::prelude::*;
use gstreamer as gst;
use tsan_player::platform::windows::{GstreamerPlayerBackend, PlayerBackendKind};

const MINIMUM_GSTREAMER_VERSION: (u32, u32) = (1, 28);
const REQUIRED_ELEMENTS: &[&str] = &[
    "appsrc",
    "queue",
    "tsparse",
    "tsdemux",
    "h265parse",
    "d3d12videosink",
    "d3d11videosink",
    "wasapi2sink",
];

fn require_element(factory_name: &str) -> Result<(), io::Error> {
    gst::ElementFactory::find(factory_name)
        .map(|_| ())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("missing GStreamer element: {factory_name}"),
            )
        })
}

fn main() -> Result<(), Box<dyn Error>> {
    gst::init()?;

    let runtime_version = gst::version();
    if (runtime_version.0, runtime_version.1) < MINIMUM_GSTREAMER_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "GStreamer {}.{} or newer is required, found {}",
                MINIMUM_GSTREAMER_VERSION.0,
                MINIMUM_GSTREAMER_VERSION.1,
                gst::version_string()
            ),
        )
        .into());
    }

    println!("GStreamer runtime: {}", gst::version_string());
    println!(
        "AppSrc binding: {}",
        gstreamer_app::AppSrc::static_type().name()
    );
    println!(
        "VideoOverlay binding: {}",
        gstreamer_video::VideoOverlay::static_type().name()
    );

    for factory_name in REQUIRED_ELEMENTS {
        require_element(factory_name)?;
        println!("{factory_name}: available");
    }

    for backend in [PlayerBackendKind::D3d12, PlayerBackendKind::D3d11] {
        println!("{backend} H.265 adapters:");
        for adapter in GstreamerPlayerBackend::discover_adapters(backend)? {
            println!(
                "  {}: {} vendor={:?} device={:?} luid={:?}",
                adapter.selection(),
                adapter.name(),
                adapter.vendor_id(),
                adapter.device_id(),
                adapter.adapter_luid()
            );
        }
    }

    Ok(())
}
