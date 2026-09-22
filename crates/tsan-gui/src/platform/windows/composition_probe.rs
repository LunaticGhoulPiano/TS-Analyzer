// Isolated research tests, not a production rendering backend.
// No global input, no system setting changes, and no capture affinity on the
// composition window. An observer capture exists only to measure test pixels.
use super::tests::{TestWindow, pump_messages};
use super::*;
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl,
};
use windows::UI::Composition::{CompositionEffectSourceParameter, Compositor};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL};
use windows::Win32::Graphics::Direct2D::{CLSID_D2D1DisplacementMap, CLSID_D2D12DAffineTransform};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    GRAPHICS_EFFECT_PROPERTY_MAPPING, IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{GUID, HSTRING, PCWSTR, implement, w};

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct ProbeEffect {
    id: GUID,
    name: Mutex<HSTRING>,
    properties: Vec<IPropertyValue>,
    sources: Vec<IGraphicsEffectSource>,
}
impl IGraphicsEffectSource_Impl for ProbeEffect_Impl {}
impl IGraphicsEffect_Impl for ProbeEffect_Impl {
    fn Name(&self) -> windows::core::Result<HSTRING> {
        Ok(self
            .name
            .lock()
            .map_err(|_| windows::core::Error::from(E_INVALIDARG))?
            .clone())
    }
    fn SetName(&self, name: &HSTRING) -> windows::core::Result<()> {
        *self
            .name
            .lock()
            .map_err(|_| windows::core::Error::from(E_INVALIDARG))? = name.clone();
        Ok(())
    }
}
impl IGraphicsEffectD2D1Interop_Impl for ProbeEffect_Impl {
    fn GetEffectId(&self) -> windows::core::Result<GUID> {
        Ok(self.id)
    }
    fn GetNamedPropertyMapping(
        &self,
        _name: &PCWSTR,
        _index: *mut u32,
        _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into()) // Fixed properties; no animation bindings.
    }
    fn GetPropertyCount(&self) -> windows::core::Result<u32> {
        Ok(self.properties.len() as u32)
    }
    fn GetProperty(&self, index: u32) -> windows::core::Result<IPropertyValue> {
        self.properties
            .get(index as usize)
            .cloned()
            .ok_or(E_INVALIDARG.into())
    }
    fn GetSourceCount(&self) -> windows::core::Result<u32> {
        Ok(self.sources.len() as u32)
    }
    fn GetSource(&self, index: u32) -> windows::core::Result<IGraphicsEffectSource> {
        self.sources
            .get(index as usize)
            .cloned()
            .ok_or(E_INVALIDARG.into())
    }
}
fn effect(displacement: bool, shift: f32) -> windows::core::Result<IGraphicsEffect> {
    let source: IGraphicsEffectSource =
        CompositionEffectSourceParameter::Create(&HSTRING::from("desktop"))?.cast()?;
    let (id, properties, sources) = if displacement {
        (
            CLSID_D2D1DisplacementMap,
            vec![
                PropertyValue::CreateSingle(20.0)?.cast()?,
                PropertyValue::CreateUInt32(0)?.cast()?,
                PropertyValue::CreateUInt32(1)?.cast()?,
            ],
            vec![source.clone(), source],
        )
    } else {
        (
            CLSID_D2D12DAffineTransform,
            vec![
                PropertyValue::CreateUInt32(1)?.cast()?,
                PropertyValue::CreateUInt32(1)?.cast()?,
                PropertyValue::CreateSingleArray(&[1.0, 0.0, 0.0, 1.0, shift, 0.0])?.cast()?,
                PropertyValue::CreateSingle(1.0)?.cast()?,
            ],
            vec![source],
        )
    };
    Ok(ProbeEffect {
        id,
        name: Mutex::new(HSTRING::from("probe")),
        properties,
        sources,
    }
    .into())
}

struct Queue(windows::System::DispatcherQueueController);
impl Queue {
    fn new() -> windows::core::Result<Self> {
        // SAFETY: this test owns the current thread and its dispatcher.
        Ok(Self(unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_STA,
            })?
        }))
    }
}
impl Drop for Queue {
    fn drop(&mut self) {
        if let Ok(operation) = self.0.ShutdownQueueAsync() {
            let start = Instant::now();
            while operation.Status().is_ok_and(|s| s.0 == 0)
                && start.elapsed() < Duration::from_secs(2)
            {
                pump_messages();
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

#[test]
#[ignore = "native Composition capability experiment; run explicitly"]
fn composition_effect_capabilities() -> Result<(), Box<dyn std::error::Error>> {
    let _dpi = DpiGuard::new();
    let _queue = Queue::new()?;
    println!("Probe thread DPI context: {:?}", unsafe {
        windows::Win32::UI::HiDpi::GetThreadDpiAwarenessContext()
    });
    let compositor = Compositor::new()?;
    compositor.CreateEffectFactory(&effect(false, 40.0)?)?;
    let displacement = compositor.CreateEffectFactory(&effect(true, 0.0)?);
    match displacement {
        Ok(_) => println!(
            "Composition accepted DisplacementMap; actual backdrop rendering still requires verification."
        ),
        Err(error) => println!(
            "Composition DisplacementMap rejected: {:?}; affine factory accepted.",
            error.code()
        ),
    }
    compositor.Close()?;
    Ok(())
}

fn rectangle(
    title: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    white: bool,
    composition: bool,
) -> windows::core::Result<TestWindow> {
    // SAFETY: all test windows belong to this thread and close through RAII.
    let window = TestWindow(unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE
                | WS_EX_TOOLWINDOW
                | if composition {
                    WS_EX_NOREDIRECTIONBITMAP
                } else {
                    WINDOW_EX_STYLE(0)
                },
            w!("STATIC"),
            &HSTRING::from(title),
            WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN | WINDOW_STYLE(if white { 6 } else { 4 }),
            x,
            y,
            width,
            height,
            None,
            None,
            None,
            None,
        )?
    });
    unsafe {
        SetWindowPos(
            window.0,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE,
        )?;
    }
    Ok(window)
}

fn sample_row(capture: &DesktopCapture) -> Result<Vec<u8>, String> {
    let start = Instant::now();
    let mut latest = None;
    while start.elapsed() < Duration::from_millis(500) {
        pump_messages();
        if let Some(frame) = capture.update()? {
            latest = Some(frame);
        }
        thread::sleep(Duration::from_millis(10));
    }
    let frame = latest.ok_or("Composition probe received no observation frame")?;
    let [u, v, w, h] = frame.source_uv();
    Ok([0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875]
        .map(|fraction| {
            let x = ((u + fraction * w) * frame.size[0] as f32) as usize;
            let y = ((v + h * 0.5) * frame.size[1] as f32) as usize;
            frame.rgba[(y * frame.size[0] as usize + x) * 4]
        })
        .to_vec())
}

#[test]
#[ignore = "briefly displays non-activating native probe windows and captures only test measurements"]
fn host_backdrop_affine_pixels() -> Result<(), Box<dyn std::error::Error>> {
    let _dpi = DpiGuard::new();
    let _queue = Queue::new()?;
    let background = rectangle(
        "Composition probe white background",
        80,
        80,
        600,
        400,
        true,
        false,
    )?;
    let right = TestWindow(unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE,
            w!("STATIC"),
            w!("Composition test dark half"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(4),
            260,
            0,
            340,
            400,
            Some(background.0),
            None,
            None,
            None,
        )?
    });
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(background.0);
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(right.0);
        windows::Win32::Graphics::Dwm::DwmFlush()?;
    }
    let foreground = rectangle(
        "Composition probe (no capture exclusion)",
        180,
        150,
        320,
        200,
        false,
        true,
    )?;
    let enabled = 1_i32;
    unsafe {
        DwmSetWindowAttribute(
            foreground.0,
            DWMWA_USE_HOSTBACKDROPBRUSH,
            &enabled as *const _ as *const _,
            size_of::<i32>() as u32,
        )?;
    }
    let compositor = Compositor::new()?;
    let interop: ICompositorDesktopInterop = compositor.cast()?;
    let target = unsafe { interop.CreateDesktopWindowTarget(foreground.0, true)? };
    let visual = compositor.CreateSpriteVisual()?;
    let mut size = visual.Size()?;
    size.X = 320.0;
    size.Y = 200.0;
    visual.SetSize(size)?;
    let backdrop = compositor.CreateHostBackdropBrush()?;
    visual.SetBrush(&backdrop)?;
    let root = compositor.CreateContainerVisual()?;
    root.SetSize(size)?;
    root.Children()?.InsertAtTop(&visual)?;
    let marker = compositor.CreateSpriteVisual()?;
    let mut marker_size = size;
    marker_size.X = 16.0;
    marker_size.Y = 16.0;
    marker.SetSize(marker_size)?;
    let marker_brush = compositor.CreateColorBrushWithColor(windows::UI::Color {
        A: 255,
        R: 255,
        G: 0,
        B: 0,
    })?;
    marker.SetBrush(&marker_brush)?;
    println!(
        "Marker size={:?}, opacity={:?}, root size={:?}",
        marker.Size()?,
        marker.Opacity()?,
        root.Size()?
    );
    root.Children()?.InsertAtTop(&marker)?;
    target.SetRoot(&root)?;
    // The observer is excluded solely so the measurement itself does not obscure
    // the probe. The actual composition window always retains WDA_NONE.
    let observer = rectangle(
        "Composition probe measurement",
        180,
        150,
        320,
        200,
        false,
        false,
    )?;
    unsafe {
        let _ = ShowWindow(observer.0, SW_HIDE);
    }
    let capture = DesktopCapture::start_handle(observer.0)?;
    let _initial_commit = compositor.RequestCommitAsync()?;
    println!(
        "Foreground dpi={}, thread={:?}, geom={:?}",
        unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(foreground.0) },
        unsafe { windows::Win32::UI::HiDpi::GetThreadDpiAwarenessContext() },
        window_geometry(foreground.0)?
    );
    let baseline_deadline = Instant::now() + Duration::from_secs(4);
    let identity = loop {
        pump_messages();
        let _present = compositor.RequestCommitAsync()?;
        let row = sample_row(&capture)?;
        if row[0] > 240 && row[1] > 240 && row[6] < 160 {
            break row;
        }
        if Instant::now() >= baseline_deadline {
            return Err(format!("Backdrop test pattern not yet presented: {row:?}").into());
        }
    };
    println!("Identity backdrop samples={identity:?}");
    let factory = compositor.CreateEffectFactory(&effect(false, 80.0)?)?;
    let brush = factory.CreateBrush()?;
    let transform_supported = match brush.SetSourceParameter(&HSTRING::from("desktop"), &backdrop) {
        Ok(()) => {
            visual.SetBrush(&brush)?;
            true
        }
        Err(error) => {
            assert_eq!(error.code(), E_INVALIDARG);
            println!("HostBackdrop transform rejected: {error}");
            false
        }
    };
    let _effect_commit = compositor.RequestCommitAsync()?;
    let shifted = sample_row(&capture)?;
    let image = capture.update()?.ok_or("Missing screenshot measurement")?;
    let [u, v, w, h] = image.source_uv();
    let x = ((u + w * 8.0 / 320.0) * image.size[0] as f32) as usize;
    let y = ((v + h * 8.0 / 200.0) * image.size[1] as f32) as usize;
    let offset = (y * image.size[0] as usize + x) * 4;
    let marker_pixel = &image.rgba[offset..offset + 4];
    println!(
        "Unexcluded composition marker screenshot pixel={marker_pixel:?}, size={:?}, geometry={:?}",
        image.size, image.geometry
    );
    unsafe {
        let dc = windows::Win32::Graphics::Gdi::GetDC(None);
        let color = windows::Win32::Graphics::Gdi::GetPixel(dc, 188, 158);
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, dc);
        println!("Desktop marker GetPixel={:?}", color);
    }
    if let Some(path) = std::env::var_os("LIQUID_GLASS_COMPOSITION_PREVIEW") {
        let mut pixels = Vec::new();
        for row in 0..200 {
            for column in 0..320 {
                let sx = ((u + w * column as f32 / 320.0) * image.size[0] as f32) as usize;
                let sy = ((v + h * row as f32 / 200.0) * image.size[1] as f32) as usize;
                let p = (sy * image.size[0] as usize + sx) * 4;
                pixels.extend_from_slice(&image.rgba[p..p + 4]);
            }
        }
        std::fs::write(path, pixels)?;
    }
    assert!(
        marker_pixel[0] > 200 && marker_pixel[1] < 40 && marker_pixel[2] < 40,
        "The actual composition content must appear in screenshots"
    );
    unsafe {
        SetWindowPos(
            right.0,
            None,
            300,
            0,
            300,
            400,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )?;
    }
    let animated = sample_row(&capture)?;
    println!(
        "HostBackdrop identity={identity:?}, affine(+80px)={shifted:?}, background moved={animated:?}"
    );
    assert_eq!(
        previous_affinity(foreground.0)?,
        0,
        "Composition window must remain screenshot-visible"
    );
    // Record capabilities without treating a successful HRESULT as proof of refraction.
    println!(
        "Pixel changes: affine={}, live={}",
        identity != shifted,
        shifted != animated
    );
    assert_ne!(
        shifted, animated,
        "HostBackdrop must update with the background"
    );
    if !transform_supported {
        assert!(
            identity
                .iter()
                .zip(&shifted)
                .all(|(a, b)| a.abs_diff(*b) <= 3),
            "Rejected transform must leave the settled baseline intact"
        );
    }
    drop(capture);
    target.SetRoot(None::<&windows::UI::Composition::Visual>)?;
    drop((
        brush,
        factory,
        backdrop,
        visual,
        marker,
        marker_brush,
        root,
        target,
        interop,
    ));
    compositor.Close()?;
    drop((observer, foreground, right, background));
    Ok(())
}

pub(super) struct DpiGuard(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
impl DpiGuard {
    pub(super) fn new() -> Self {
        use windows::Win32::UI::HiDpi::*;
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}
impl Drop for DpiGuard {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            unsafe {
                windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}
