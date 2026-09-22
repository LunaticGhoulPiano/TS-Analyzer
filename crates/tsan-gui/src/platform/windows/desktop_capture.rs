// Native capture is isolated here; callers only receive owned RGBA pixels.
// Windows handles remain owned by this module or by the retained winit window.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat};
use windows::Win32::Foundation::{HMODULE, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::{Common::*, IDXGIDevice};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromWindow,
};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetClientRect, GetWindowDisplayAffinity, GetWindowLongW, IsIconic,
    SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WINDOW_DISPLAY_AFFINITY, WS_EX_LAYERED,
};
use windows::core::Interface;

static FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(1);

const FRAME_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Geometry {
    monitor: usize,
    // Physical desktop coordinates, including negative monitor origins.
    monitor_rect: [i32; 4],
    client: [i32; 4],
}

impl Geometry {
    fn crop(self) -> Option<[u32; 4]> {
        let [left, top, right, bottom] = self.client;
        let [ml, mt, mr, mb] = self.monitor_rect;
        if left < ml || top < mt || right > mr || bottom > mb || right <= left || bottom <= top {
            return None;
        }
        Some([
            (left - ml) as u32,
            (top - mt) as u32,
            (right - left) as u32,
            (bottom - top) as u32,
        ])
    }
}

#[derive(Clone)]
pub struct DesktopFrame {
    pub size: [u32; 2],
    pub rgba: Arc<Vec<u8>>,
    pub sequence: u64,
    geometry: Geometry,
}

impl DesktopFrame {
    fn reframe(&self, geometry: Geometry) -> Option<Arc<Self>> {
        if self.geometry.monitor != geometry.monitor
            || self.geometry.monitor_rect != geometry.monitor_rect
        {
            return None;
        }
        Some(Arc::new(Self {
            geometry,
            ..self.clone()
        }))
    }

    pub fn source_uv(&self) -> [f32; 4] {
        let [ml, mt, mr, mb] = self.geometry.monitor_rect;
        let [left, top, right, bottom] = self.geometry.client;
        let width = (mr - ml).max(1) as f32;
        let height = (mb - mt).max(1) as f32;
        [
            (left - ml) as f32 / width,
            (top - mt) as f32 / height,
            (right - left) as f32 / width,
            (bottom - top) as f32 / height,
        ]
    }
}

#[derive(Default)]
struct Shared {
    request: Option<Geometry>,
    frame: Option<Arc<DesktopFrame>>,
    error: Option<String>,
    stop: bool,
}

pub struct DesktopCapture {
    hwnd: usize,
    // Retain the window until after exclusion has been restored.
    _window: Option<Arc<winit::window::Window>>,
    previous_affinity: u32,
    excluded: bool,
    shared: Arc<Mutex<Shared>>,
    worker: Option<JoinHandle<()>>,
    frozen: Option<Arc<DesktopFrame>>,
}

impl DesktopCapture {
    pub fn start(window: Arc<winit::window::Window>) -> Result<Self, String> {
        let handle = window
            .window_handle()
            .map_err(|e| format!("Desktop capture: {e}"))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("Liquid Glass requires a Windows window.".to_owned());
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        let mut capture = Self::start_handle(hwnd)?;
        capture._window = Some(window);
        Ok(capture)
    }

    fn start_handle(hwnd: HWND) -> Result<Self, String> {
        check_capture_compatibility()?;
        let previous_affinity = previous_affinity(hwnd)?;
        // SAFETY: hwnd belongs to the calling process and the owner outlives capture.
        unsafe {
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)
                .map_err(|e| format!("Cannot exclude this window from capture: {e}"))?;
        }
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut capture = Self {
            hwnd: hwnd.0 as usize,
            _window: None,
            previous_affinity,
            excluded: true,
            shared: shared.clone(),
            worker: None,
            frozen: None,
        };
        let mut actual = 0;
        unsafe { GetWindowDisplayAffinity(hwnd, &mut actual) }
            .map_err(|e| format!("Verify capture exclusion: {e}"))?;
        if actual != WDA_EXCLUDEFROMCAPTURE.0 {
            return Err("Capture exclusion requires Windows 10 version 2004 or newer.".to_owned());
        }
        let window_id = hwnd.0 as usize;
        capture.worker = Some(
            thread::Builder::new()
                .name("liquid-glass-capture".to_owned())
                .spawn(move || {
                    if let Err(error) = capture_worker(&shared, window_id)
                        && let Ok(mut state) = shared.lock()
                    {
                        state.error = Some(error);
                        state.frame = None;
                    }
                })
                .map_err(|e| format!("Desktop capture: {e}"))?,
        );
        Ok(capture)
    }

    // Reposition the sampling rectangle immediately; desktop pixels update independently.
    pub fn update(&self) -> Result<Option<Arc<DesktopFrame>>, String> {
        if let Some(frame) = &self.frozen {
            return Ok(Some(frame.clone()));
        }
        let geometry = window_geometry(HWND(self.hwnd as *mut _))?;
        let mut state = self
            .shared
            .lock()
            .map_err(|e| format!("Desktop capture: {e}"))?;
        if let Some(error) = state.error.take() {
            return Err(error);
        }
        state.request = geometry.filter(|geometry| geometry.crop().is_some());
        Ok(state
            .frame
            .as_ref()
            .and_then(|frame| frame.reframe(state.request?)))
    }

    #[cfg(test)]
    pub(crate) fn fail_for_test(&self, message: &str) -> Result<(), String> {
        self.shared.lock().map_err(|error| error.to_string())?.error = Some(message.to_owned());
        Ok(())
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen.is_some()
    }

    pub fn freeze(&mut self) -> Result<(), String> {
        if self.is_frozen() {
            return Ok(());
        }
        let frame = self
            .update()?
            .ok_or("Wait for an aligned desktop frame before freezing.")?;
        self.stop()?;
        self.frozen = Some(frame);
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if !self.is_frozen() {
            return Ok(());
        }
        let mut capture = Self::start_handle(HWND(self.hwnd as *mut _))?;
        capture._window = self._window.clone();
        *self = capture;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.frozen = None;
        // Stop capture before making our pixels visible to capture clients again.
        if let Ok(mut state) = self.shared.lock() {
            state.stop = true;
            state.frame = None;
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if self.excluded {
            unsafe {
                SetWindowDisplayAffinity(
                    HWND(self.hwnd as *mut _),
                    WINDOW_DISPLAY_AFFINITY(self.previous_affinity),
                )
            }
            .map_err(|e| format!("Cannot restore screen capture visibility: {e}"))?;
            self.excluded = false;
        }
        Ok(())
    }
}

impl Drop for DesktopCapture {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("Liquid Glass: {error}");
        }
    }
}

// RtlGetVersion reports the real build without depending on the EXE's
// supportedOS manifest. OSVERSIONINFOW and RTL_OSVERSIONINFOW have the same ABI.
#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(
        version: *mut windows::Win32::System::SystemInformation::OSVERSIONINFOW,
    ) -> i32;
}

fn require_capture_version(major: u32, build: u32) -> Result<(), String> {
    if major < 10 || (major == 10 && build < 19041) {
        return Err(format!(
            "Live Liquid Glass requires Windows 10 2004 (build 19041) or newer, or Windows 11. Detected version {major}, build {build}."
        ));
    }
    Ok(())
}

fn check_capture_compatibility() -> Result<(), String> {
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: the initialized, correctly sized buffer lives through the call.
    let status = unsafe { RtlGetVersion(&mut version) };
    if status < 0 {
        return Err(format!(
            "Cannot check Windows capture compatibility: {status:#x}"
        ));
    }
    require_capture_version(version.dwMajorVersion, version.dwBuildNumber)
}

fn previous_affinity(hwnd: HWND) -> Result<u32, String> {
    let mut affinity = 0;
    // GetWindowDisplayAffinity can fail for a normal, never-protected window.
    // Our app owns this HWND and has not enabled protection through another path.
    match unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) } {
        Ok(()) => Ok(affinity),
        Err(_) if unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_LAYERED.0 == 0 => {
            Ok(0)
        }
        Err(error) => Err(format!("Read capture visibility: {error}")),
    }
}

fn window_geometry(hwnd: HWND) -> Result<Option<Geometry>, String> {
    // SAFETY: buffers are initialized, correctly sized, and live for each call.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            return Ok(None);
        }
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client).map_err(|e| format!("Desktop capture: {e}"))?;
        if client.right <= 0 || client.bottom <= 0 {
            return Ok(None);
        }
        let mut origin = POINT::default();
        ClientToScreen(hwnd, &mut origin)
            .ok()
            .map_err(|e| format!("Desktop capture: {e}"))?;
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info)
            .ok()
            .map_err(|e| format!("Desktop capture: {e}"))?;
        Ok(Some(Geometry {
            monitor: monitor.0 as usize,
            monitor_rect: [
                info.rcMonitor.left,
                info.rcMonitor.top,
                info.rcMonitor.right,
                info.rcMonitor.bottom,
            ],
            client: [
                origin.x,
                origin.y,
                origin.x + client.right,
                origin.y + client.bottom,
            ],
        }))
    }
}

struct Apartment;
impl Apartment {
    fn new() -> windows::core::Result<Self> {
        // This worker owns its MTA apartment and all its D3D/WinRT objects.
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

struct MonitorCapture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    latest: Option<Direct3D11CaptureFrame>,
    monitor: usize,
    monitor_rect: [i32; 4],
    size: windows::Graphics::SizeInt32,
    last_arrival: Instant,
}

impl MonitorCapture {
    fn new(device: &IDirect3DDevice, geometry: Geometry) -> windows::core::Result<Self> {
        let factory: IGraphicsCaptureItemInterop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem =
            unsafe { factory.CreateForMonitor(HMONITOR(geometry.monitor as *mut _))? };
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        // Keep the OS capture indicator. Do not request borderless capture.
        session.SetIsCursorCaptureEnabled(false)?;
        session.StartCapture()?;
        Ok(Self {
            pool,
            session,
            latest: None,
            monitor: geometry.monitor,
            monitor_rect: geometry.monitor_rect,
            size,
            last_arrival: Instant::now(),
        })
    }

    fn poll(&mut self, device: &IDirect3DDevice) -> windows::core::Result<bool> {
        let mut changed = false;
        // windows-rs represents a successful null WinRT result as Error::empty()
        // (S_OK); older bindings may use E_POINTER for the same empty queue.
        for _ in 0..2 {
            let frame = match self.pool.TryGetNextFrame() {
                Ok(frame) => frame,
                Err(error)
                    if error.code().0 == 0
                        || error.code() == windows::core::HRESULT(0x80004003_u32 as i32) =>
                {
                    break;
                }
                Err(error) => return Err(error),
            };
            let size = frame.ContentSize()?;
            if size != self.size {
                frame.Close()?;
                if let Some(old) = self.latest.take() {
                    old.Close()?;
                }
                self.pool
                    .Recreate(device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)?;
                self.size = size;
                return Ok(false);
            }
            if let Some(old) = self.latest.replace(frame) {
                old.Close()?;
            }
            self.last_arrival = Instant::now();
            changed = true;
        }
        Ok(changed)
    }
}
impl Drop for MonitorCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        if let Some(frame) = self.latest.take() {
            let _ = frame.Close();
        }
        let _ = self.pool.Close();
    }
}

fn capture_worker(shared: &Mutex<Shared>, window_id: usize) -> Result<(), String> {
    let _apartment = Apartment::new().map_err(|e| format!("Desktop capture: {e}"))?;
    if !GraphicsCaptureSession::IsSupported().map_err(|e| format!("Desktop capture: {e}"))? {
        return Err("Windows Graphics Capture is unavailable.".to_owned());
    }
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|e| format!("Desktop capture: {e}"))?;
    let device = device.ok_or("D3D11 device is unavailable")?;
    let context = context.ok_or("D3D11 context is unavailable")?;
    let dxgi: IDXGIDevice = device.cast().map_err(|e| format!("Desktop capture: {e}"))?;
    let capture_device: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .and_then(|device| device.cast())
        .map_err(|e| format!("Desktop capture: {e}"))?;
    let mut monitor: Option<MonitorCapture> = None;
    let mut staging: Option<(ID3D11Texture2D, [u32; 2])> = None;

    loop {
        let start = Instant::now();
        let request = {
            let state = shared.lock().map_err(|e| format!("Desktop capture: {e}"))?;
            if state.stop {
                break;
            }
            state.request
        };
        // UI painting can pause while minimized; the worker must stop on its own.
        let request = request.filter(|_| !unsafe { IsIconic(HWND(window_id as *mut _)) }.as_bool());
        if let Some(geometry) = request {
            if monitor.as_ref().is_none_or(|monitor| {
                monitor.monitor != geometry.monitor || monitor.monitor_rect != geometry.monitor_rect
            }) {
                drop(monitor.take()); // Close the old monitor before opening the next.
                monitor = Some(
                    MonitorCapture::new(&capture_device, geometry)
                        .map_err(|e| format!("Desktop capture: {e}"))?,
                );
            }
            if let Some(monitor) = &mut monitor {
                let changed = monitor
                    .poll(&capture_device)
                    .map_err(|e| format!("Desktop capture: {e}"))?;
                if monitor.latest.is_none()
                    && monitor.last_arrival.elapsed() > Duration::from_secs(5)
                {
                    return Err(
                        "No desktop frames arrived; capture may be blocked by Windows.".to_owned(),
                    );
                }
                if changed && let Some(frame) = &monitor.latest {
                    let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                    let image =
                        read_frame(&device, &context, &mut staging, frame, geometry, sequence)
                            .map_err(|e| format!("Desktop capture: {e}"))?;
                    let mut state = shared.lock().map_err(|e| format!("Desktop capture: {e}"))?;
                    if state.request.is_some_and(|request| {
                        request.monitor == geometry.monitor
                            && request.monitor_rect == geometry.monitor_rect
                    }) {
                        state.frame = Some(Arc::new(image));
                    }
                }
            }
        } else {
            monitor = None;
            staging = None;

            if let Ok(mut state) = shared.lock() {
                state.frame = None;
            }
        }
        thread::sleep(FRAME_INTERVAL.saturating_sub(start.elapsed()));
    }
    Ok(())
}

fn read_frame(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    staging: &mut Option<(ID3D11Texture2D, [u32; 2])>,
    frame: &Direct3D11CaptureFrame,
    geometry: Geometry,
    sequence: u64,
) -> windows::core::Result<DesktopFrame> {
    let invalid = || {
        windows::core::Error::new(
            windows::core::HRESULT(0x80070057_u32 as i32),
            "Invalid capture crop",
        )
    };
    let [ml, mt, mr, mb] = geometry.monitor_rect;
    let (x, y, width, height) = (0, 0, (mr - ml) as u32, (mb - mt) as u32);
    if width == 0 || height == 0 {
        return Err(invalid());
    }
    let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
    // SAFETY: interfaces are queried from the owned frame; D3D validates types.
    unsafe {
        let source: ID3D11Texture2D = access.GetInterface()?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        source.GetDesc(&mut desc);
        if x + width > desc.Width
            || y + height > desc.Height
            || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
        {
            return Err(invalid());
        }
        if staging
            .as_ref()
            .is_none_or(|(_, size)| *size != [width, height])
        {
            let mut texture = None;
            device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut texture),
            )?;
            *staging = Some((texture.ok_or_else(invalid)?, [width, height]));
        }
        let (texture, _) = staging.as_ref().ok_or_else(invalid)?;
        context.CopySubresourceRegion(
            texture,
            0,
            0,
            0,
            0,
            &source,
            0,
            Some(&D3D11_BOX {
                left: x,
                top: y,
                front: 0,
                right: x + width,
                bottom: y + height,
                back: 1,
            }),
        );
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let result = if mapped.pData.is_null() || mapped.RowPitch < width * 4 {
            Err(invalid())
        } else {
            // Mapped rows are valid until Unmap; never expose the pointer to UI.
            let bytes = std::slice::from_raw_parts(
                mapped.pData.cast::<u8>(),
                mapped.RowPitch as usize * height as usize,
            );
            let size = capture_size([width, height]);
            let rgba = downsample_bgra(bytes, mapped.RowPitch as usize, [width, height], size);
            Ok(DesktopFrame {
                size,
                rgba: Arc::new(rgba),
                sequence,
                geometry,
            })
        };
        context.Unmap(texture, 0);
        result
    }
}

fn capture_size(size: [u32; 2]) -> [u32; 2] {
    let scale = 0.5_f64.min(2048.0 / f64::from(size[0].max(size[1]).max(1)));
    [
        (f64::from(size[0]) * scale).ceil().clamp(1.0, 2048.0) as u32,
        (f64::from(size[1]) * scale).ceil().clamp(1.0, 2048.0) as u32,
    ]
}

fn downsample_bgra(source: &[u8], pitch: usize, input: [u32; 2], size: [u32; 2]) -> Vec<u8> {
    let mut rgba = vec![0; (size[0] * size[1] * 4) as usize];
    for y in 0..size[1] {
        let sy = (u64::from(y) * u64::from(input[1]) / u64::from(size[1])) as usize;
        for x in 0..size[0] {
            let sx = (u64::from(x) * u64::from(input[0]) / u64::from(size[0])) as usize;
            let from = sy * pitch + sx * 4;
            let to = ((y * size[0] + x) * 4) as usize;
            rgba[to..to + 4].copy_from_slice(&[
                source[from + 2],
                source[from + 1],
                source[from],
                255,
            ]);
        }
    }
    rgba
}

#[cfg(test)]
impl DesktopFrame {
    pub(crate) fn test_pattern(size: [u32; 2]) -> Self {
        let mut rgba = Vec::with_capacity((size[0] * size[1] * 4) as usize);
        for y in 0..size[1] {
            for x in 0..size[0] {
                let grid = x % 40 < 3 || y % 40 < 3;
                rgba.extend(if grid {
                    [230, 230, 230, 255]
                } else {
                    [30, 70, 130, 255]
                });
            }
        }
        Self {
            size,
            rgba: Arc::new(rgba),
            sequence: 1,
            geometry: Geometry {
                monitor: 0,
                monitor_rect: [0, 0, size[0] as i32, size[1] as i32],
                client: [0, 0, size[0] as i32, size[1] as i32],
            },
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::w;

    #[test]
    fn capture_version_gate_covers_windows_10_and_11() {
        for (major, build) in [(6, 9600), (10, 17763), (10, 18363), (10, 19040)] {
            assert!(require_capture_version(major, build).is_err());
        }
        for (major, build) in [
            (10, 19041),
            (10, 19044),
            (10, 19045),
            (10, 22000),
            (10, 26100),
            (11, 1),
        ] {
            assert!(require_capture_version(major, build).is_ok());
        }
    }

    #[test]
    fn moving_view_reuses_monitor_pixels_and_updates_uvs() -> Result<(), String> {
        let mut frame = DesktopFrame::test_pattern([100, 100]);
        frame.geometry = Geometry {
            monitor: 7,
            monitor_rect: [-1000, -100, -900, 0],
            client: [-1000, -100, -950, -75],
        };
        assert_eq!(frame.source_uv(), [0.0, 0.0, 0.5, 0.25]);
        let mut moved = frame.geometry;
        moved.client = [-975, -50, -925, -25];
        let view = frame.reframe(moved).ok_or("Missing moved view")?;
        assert_eq!(view.source_uv(), [0.25, 0.5, 0.5, 0.25]);
        assert!(
            Arc::ptr_eq(&frame.rgba, &view.rgba),
            "Moving must not copy desktop pixels"
        );
        assert_eq!(
            frame.sequence, view.sequence,
            "Moving must not re-upload the texture"
        );
        moved.monitor = 8;
        assert!(frame.reframe(moved).is_none());
        moved.monitor = 7;
        moved.monitor_rect[2] += 100;
        assert!(
            frame.reframe(moved).is_none(),
            "Display mode changes need a new source"
        );
        Ok(())
    }

    #[test]
    fn crop_supports_negative_origins_and_rejects_spanning() {
        let mut geometry = Geometry {
            monitor: 0,
            monitor_rect: [-1920, -100, 0, 980],
            client: [-1800, 20, -800, 820],
        };
        assert_eq!(geometry.crop(), Some([120, 120, 1000, 800]));
        geometry.client[2] = 100;
        assert_eq!(geometry.crop(), None);
        geometry.client = [-1800, 20, -1800, 20];
        assert_eq!(geometry.crop(), None);
    }

    #[test]
    fn readback_conversion_respects_pitch_channels_and_scale() {
        let bytes = [
            1, 2, 3, 7, 10, 20, 30, 7, 99, 99, 99, 99, 4, 5, 6, 7, 40, 50, 60, 7, 99, 99, 99, 99,
        ];
        assert_eq!(
            downsample_bgra(&bytes, 12, [2, 2], [2, 2]),
            [3, 2, 1, 255, 30, 20, 10, 255, 6, 5, 4, 255, 60, 50, 40, 255]
        );
        assert_eq!(downsample_bgra(&bytes, 12, [2, 2], [1, 1]), [3, 2, 1, 255]);
        assert_eq!(capture_size([1280, 800]), [640, 400]);
        assert_eq!(capture_size([15360, 8640]), [2048, 1152]);
    }

    pub(super) struct TestWindow(pub(super) HWND);
    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    pub(super) fn pump_messages() {
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    pub(super) fn wait_for_frame(
        capture: &DesktopCapture,
        white: bool,
    ) -> Result<Arc<DesktopFrame>, String> {
        let color = unsafe {
            windows::Win32::Graphics::Gdi::GetSysColor(if white {
                windows::Win32::Graphics::Gdi::COLOR_WINDOW
            } else {
                windows::Win32::Graphics::Gdi::COLOR_WINDOWFRAME
            })
        };
        let expected = (color & 255) as u8;
        let start = Instant::now();
        let mut last_pixel = None;
        while start.elapsed() < Duration::from_secs(8) {
            pump_messages();
            if let Some(frame) = capture.update()? {
                let [x, y, w, h] = frame.source_uv();
                let cx = ((x + w * 0.5) * frame.size[0] as f32) as u32;
                let cy = ((y + h * 0.5) * frame.size[1] as f32) as u32;
                let center = ((cy * frame.size[0] + cx) * 4) as usize;
                let value = frame.rgba[center];
                last_pixel = Some((value, frame.sequence));
                // Allow small compositor / display color conversion differences.
                if value.abs_diff(expected) <= 8 {
                    return Ok(frame);
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(format!(
            "Timed out waiting for {} desktop pixels; last={last_pixel:?}, expected={expected}",
            if white { "white" } else { "black" }
        ))
    }

    #[test]
    #[ignore = "opens temporary test windows and requires an interactive Windows capture session"]
    fn live_capture_excludes_self_moves_resizes_and_restores()
    -> Result<(), Box<dyn std::error::Error>> {
        let _dpi = super::composition_probe::DpiGuard::new();
        // White and black STATIC rectangles provide known pixels, without saving
        // or asserting anything about the user's desktop.
        let background = TestWindow(unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("Glass capture test background"),
                WS_POPUP | WS_VISIBLE | WINDOW_STYLE(6), // SS_WHITERECT
                80,
                80,
                480,
                360,
                None,
                None,
                None,
                None,
            )?
        });
        let foreground = TestWindow(unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("Glass capture test foreground"),
                WS_POPUP | WS_VISIBLE | WINDOW_STYLE(4), // SS_BLACKRECT
                120,
                120,
                240,
                160,
                None,
                None,
                None,
                None,
            )?
        });
        unsafe {
            SetWindowPos(
                background.0,
                Some(HWND_TOPMOST),
                80,
                80,
                480,
                360,
                SWP_NOACTIVATE,
            )?;
            SetWindowPos(
                foreground.0,
                Some(HWND_TOPMOST),
                120,
                120,
                240,
                160,
                SWP_NOACTIVATE,
            )?;
        }
        pump_messages();
        let previous = previous_affinity(foreground.0)?;
        let mut capture = DesktopCapture::start_handle(foreground.0)?;
        let image = wait_for_frame(&capture, true)?;
        assert_eq!(image.geometry.client, [120, 120, 360, 280]);
        // A second capture client proves inclusion is restored, not merely that
        // the affinity flag changed. Its own window is excluded from both sessions.
        let observer = TestWindow(unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("Glass screenshot observer"),
                WS_POPUP | WS_VISIBLE | WINDOW_STYLE(4),
                120,
                120,
                240,
                160,
                None,
                None,
                None,
                None,
            )?
        });
        let mut observer_capture = DesktopCapture::start_handle(observer.0)?;
        wait_for_frame(&observer_capture, true)?;
        capture.freeze()?;
        assert!(capture.is_frozen());
        assert!(capture.worker.is_none());
        assert!(!capture.excluded);
        assert_eq!(previous_affinity(foreground.0)?, previous);
        let frozen = capture.update()?.ok_or("Missing frozen frame")?;
        assert_eq!(image.source_uv(), frozen.source_uv());
        wait_for_frame(&observer_capture, false)?;
        thread::sleep(Duration::from_millis(100));
        assert!(Arc::ptr_eq(
            &frozen,
            &capture.update()?.ok_or("Frozen frame lost")?
        ));
        capture.freeze()?; // Idempotent: do not clear the retained background.
        capture.resume()?;
        assert!(!capture.is_frozen());
        let resumed = wait_for_frame(&capture, true)?;
        assert!(
            resumed.sequence > frozen.sequence,
            "GPU cache must refresh after resume"
        );
        wait_for_frame(&observer_capture, true)?;
        observer_capture.stop()?;
        drop(observer_capture);
        drop(observer);
        unsafe {
            SetWindowPos(
                foreground.0,
                None,
                150,
                140,
                280,
                180,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )?;
        }
        let immediate = capture
            .update()?
            .ok_or("Glass disappeared during movement")?;
        assert_eq!(immediate.geometry.client, [150, 140, 430, 320]);
        let moved = wait_for_frame(&capture, true)?;
        assert_eq!(
            moved.size, image.size,
            "Movement must not reallocate the desktop texture"
        );
        assert_eq!(moved.geometry.client[..2], [150, 140]);
        // Continuous motion must keep a renderable background even before the
        // capture worker produces another desktop frame.
        for step in 0..60 {
            unsafe {
                SetWindowPos(
                    foreground.0,
                    None,
                    150 + step,
                    140,
                    280,
                    180,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )?;
            }
            pump_messages();
            let moving = capture
                .update()?
                .ok_or("Glass disappeared during continuous dragging")?;
            assert_eq!(moving.geometry.client[0], 150 + step);
            assert_eq!(moving.size, image.size);
            let [u, _, w, _] = moving.source_uv();
            let monitor_width =
                (moving.geometry.monitor_rect[2] - moving.geometry.monitor_rect[0]) as f32;
            assert!((w * monitor_width - 280.0).abs() < 0.01);
            assert!(u.is_finite());
        }

        unsafe {
            SetWindowLongPtrW(
                background.0,
                GWL_STYLE,
                (WS_POPUP | WS_VISIBLE | WINDOW_STYLE(4)).0 as isize,
            );
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(background.0), None, true);
            let _ = windows::Win32::Graphics::Gdi::UpdateWindow(background.0);
        }
        let changed = wait_for_frame(&capture, false)?;
        assert!(changed.sequence > moved.sequence);
        unsafe {
            let _ = ShowWindow(foreground.0, SW_MINIMIZE);
        }
        pump_messages();
        let paused_at = Instant::now();
        loop {
            // Do not call update(): prove the worker pauses even without UI paints.
            if capture
                .shared
                .lock()
                .map_err(|e| e.to_string())?
                .frame
                .is_none()
            {
                break;
            }
            if paused_at.elapsed() > Duration::from_secs(2) {
                return Err("Capture did not pause after minimizing".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            let _ = ShowWindow(foreground.0, SW_SHOWNOACTIVATE);
        }
        wait_for_frame(&capture, false)?;
        capture.stop()?;
        let mut restored = previous_affinity(foreground.0)?;
        assert_eq!(restored, previous);
        // Also verify RAII restoration, as used when the app exits.
        drop(DesktopCapture::start_handle(foreground.0)?);
        restored = previous_affinity(foreground.0)?;
        assert_eq!(restored, previous);
        Ok(())
    }
}

#[cfg(test)]
#[path = "composition_probe.rs"]
mod composition_probe;
