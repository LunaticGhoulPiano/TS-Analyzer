use std::any::Any;
use std::cell::Cell;
use std::fmt;
use std::sync::Arc;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, HGDIOBJ, RGN_DIFF, RGN_ERROR, SetWindowRgn,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, FindWindowW, GetWindowThreadProcessId, HWND_TOP, SW_HIDE, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
};
use windows::core::{PCWSTR, w};

use crate::{PlayerError, PlayerErrorKind, PlayerResult, VideoRectangle, VideoSurface};

mod gstreamer;

const SS_BLACKRECT_STYLE: WINDOW_STYLE = WINDOW_STYLE(0x0000_0004);

pub use gstreamer::{
    GstreamerAdapter, GstreamerAvailability, GstreamerDecoderIdentity, GstreamerMediaInfo,
    GstreamerPlayerBackend,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerBackendKind {
    D3d12,
    D3d11,
}

impl PlayerBackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::D3d12 => "d3d12",
            Self::D3d11 => "d3d11",
        }
    }
}

impl fmt::Display for PlayerBackendKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GpuAdapterSelection {
    #[default]
    Default,
    Index(u32),
}

impl fmt::Display for GpuAdapterSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => formatter.write_str("default"),
            Self::Index(index) => write!(formatter, "adapter {index}"),
        }
    }
}

struct Win32VideoSurface {
    native_handle: usize,
    _owner: Arc<dyn Any + Send + Sync>,
}

pub struct WindowsVideoSurface;

pub struct WindowsVideoHost {
    native_handle: usize,
    surface: VideoSurface,
    window_rectangle: Cell<Option<VideoRectangle>>,
    visible: Cell<bool>,
    clip_state: Cell<Option<(i32, i32, Option<VideoRectangle>)>>,
}

impl WindowsVideoSurface {
    pub fn from_window<T>(window: Arc<T>) -> PlayerResult<VideoSurface>
    where
        T: HasWindowHandle + Send + Sync + 'static,
    {
        let native_handle = {
            let handle = window.window_handle().map_err(|error| {
                PlayerError::new(
                    PlayerErrorKind::InvalidInput,
                    format!("failed to obtain native window handle: {error}"),
                )
            })?;
            match handle.as_raw() {
                RawWindowHandle::Win32(handle) => handle.hwnd.get() as usize,
                _ => {
                    return Err(PlayerError::new(
                        PlayerErrorKind::Unavailable,
                        "the Windows video backend requires a Win32 window",
                    ));
                }
            }
        };
        let owner: Arc<dyn Any + Send + Sync> = window;

        Ok(VideoSurface::from_platform(Win32VideoSurface {
            native_handle,
            _owner: owner,
        }))
    }
}

impl WindowsVideoHost {
    #[allow(unsafe_code)]
    pub fn new<T>(parent: Arc<T>) -> PlayerResult<Self>
    where
        T: HasWindowHandle + Send + Sync + 'static,
    {
        let parent_handle = native_window_handle(parent.as_ref())?;
        let owner: Arc<dyn Any + Send + Sync> = parent;
        Self::new_for_parent_handle(parent_handle, owner)
    }

    #[allow(unsafe_code)]
    pub fn for_process_window_title(title: &str) -> PlayerResult<Option<Self>> {
        let title_wide = title
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();

        // SAFETY: title_wide is a null-terminated UTF-16 buffer that remains alive
        // for the duration of FindWindowW. The class name is intentionally omitted.
        let parent =
            match unsafe { FindWindowW(PCWSTR::null(), PCWSTR::from_raw(title_wide.as_ptr())) } {
                Ok(parent) => parent,
                Err(_) => return Ok(None),
            };

        let mut process_id = 0;
        // SAFETY: parent was returned by FindWindowW and process_id points to valid
        // writable storage for the duration of this call.
        let _thread_id = unsafe { GetWindowThreadProcessId(parent, Some(&mut process_id)) };
        if process_id != std::process::id() {
            return Ok(None);
        }

        let owner: Arc<dyn Any + Send + Sync> = Arc::new(title.to_owned());
        Self::new_for_parent_handle(parent.0 as usize, owner).map(Some)
    }

    #[allow(unsafe_code)]
    fn new_for_parent_handle(
        parent_handle: usize,
        owner: Arc<dyn Any + Send + Sync>,
    ) -> PlayerResult<Self> {
        // SAFETY: The parent HWND belongs to the live eframe window. The built-in
        // STATIC window class does not require registration, and Windows owns the
        // child window until its parent is destroyed.
        let child = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | SS_BLACKRECT_STYLE,
                0,
                0,
                1,
                1,
                Some(HWND(parent_handle as *mut _)),
                None,
                None,
                None,
            )
        }
        .map_err(|error| {
            PlayerError::new(
                PlayerErrorKind::Unavailable,
                format!("failed to create the video host: {error}"),
            )
        })?;
        let native_handle = child.0 as usize;
        let surface = VideoSurface::from_platform(Win32VideoSurface {
            native_handle,
            _owner: owner,
        });

        Ok(Self {
            native_handle,
            surface,
            window_rectangle: Cell::new(None),
            visible: Cell::new(false),
            clip_state: Cell::new(None),
        })
    }

    pub fn surface(&self) -> VideoSurface {
        self.surface.clone()
    }

    pub fn show(&self, rectangle: VideoRectangle) -> PlayerResult<VideoRectangle> {
        self.show_with_exclusion(rectangle, None)
    }

    #[allow(unsafe_code)]
    pub fn show_with_exclusion(
        &self,
        rectangle: VideoRectangle,
        exclusion: Option<VideoRectangle>,
    ) -> PlayerResult<VideoRectangle> {
        self.position_with_exclusion(rectangle, exclusion, true)
    }

    pub fn prepare_with_exclusion(
        &self,
        rectangle: VideoRectangle,
        exclusion: Option<VideoRectangle>,
    ) -> PlayerResult<VideoRectangle> {
        self.position_with_exclusion(rectangle, exclusion, false)
    }

    #[allow(unsafe_code)]
    fn position_with_exclusion(
        &self,
        rectangle: VideoRectangle,
        exclusion: Option<VideoRectangle>,
        visible: bool,
    ) -> PlayerResult<VideoRectangle> {
        // SAFETY: The child HWND remains owned by the live parent window. The
        // rectangle is expressed in physical client coordinates of that parent.
        if self.window_rectangle.get() != Some(rectangle) || self.visible.get() != visible {
            let visibility_flag = if visible {
                SWP_SHOWWINDOW
            } else {
                SWP_HIDEWINDOW
            };
            unsafe {
                SetWindowPos(
                    HWND(self.native_handle as *mut _),
                    Some(HWND_TOP),
                    rectangle.x(),
                    rectangle.y(),
                    rectangle.width(),
                    rectangle.height(),
                    SWP_NOACTIVATE | visibility_flag,
                )
            }
            .map_err(|error| {
                PlayerError::new(
                    PlayerErrorKind::BackendFailure,
                    format!("failed to position the embedded video host: {error}"),
                )
            })?;
            self.window_rectangle.set(Some(rectangle));
            self.visible.set(visible);
        }

        let clip_state = (rectangle.width(), rectangle.height(), exclusion);
        if self.clip_state.get() != Some(clip_state) {
            set_window_exclusion_region(
                HWND(self.native_handle as *mut _),
                rectangle.width(),
                rectangle.height(),
                exclusion,
            )?;
            self.clip_state.set(Some(clip_state));
        }

        VideoRectangle::new(0, 0, rectangle.width(), rectangle.height()).ok_or_else(|| {
            PlayerError::new(
                PlayerErrorKind::InvalidInput,
                "the embedded video host rectangle is empty",
            )
        })
    }

    #[allow(unsafe_code)]
    pub fn hide(&self) {
        // SAFETY: The child HWND remains valid while the parent window is alive.
        unsafe {
            let _ = ShowWindow(HWND(self.native_handle as *mut _), SW_HIDE);
        }
        self.window_rectangle.set(None);
        self.visible.set(false);
    }
}

#[allow(unsafe_code)]
fn set_window_exclusion_region(
    window: HWND,
    width: i32,
    height: i32,
    exclusion: Option<VideoRectangle>,
) -> PlayerResult<()> {
    let Some(exclusion) = exclusion else {
        // SAFETY: window is the live child HWND owned by WindowsVideoHost.
        if unsafe { SetWindowRgn(window, None, true) } == 0 {
            return Err(window_region_error("clear"));
        }
        return Ok(());
    };

    // SAFETY: Coordinates are physical pixels relative to the child window.
    let visible = unsafe { CreateRectRgn(0, 0, width, height) };
    let excluded = unsafe {
        CreateRectRgn(
            exclusion.x(),
            exclusion.y(),
            exclusion.x().saturating_add(exclusion.width()),
            exclusion.y().saturating_add(exclusion.height()),
        )
    };
    if visible.is_invalid() || excluded.is_invalid() {
        for region in [visible, excluded] {
            if !region.is_invalid() {
                // SAFETY: Ownership of this region has not been transferred.
                let _deleted = unsafe { DeleteObject(HGDIOBJ(region.0)) };
            }
        }
        return Err(window_region_error("create"));
    }

    // SAFETY: Both handles refer to live regions created above.
    let region_type = unsafe { CombineRgn(Some(visible), Some(visible), Some(excluded), RGN_DIFF) };
    // SAFETY: excluded is only a source region and remains owned here.
    let _excluded_deleted = unsafe { DeleteObject(HGDIOBJ(excluded.0)) };
    if region_type == RGN_ERROR {
        // SAFETY: visible has not been transferred to the window manager.
        let _visible_deleted = unsafe { DeleteObject(HGDIOBJ(visible.0)) };
        return Err(window_region_error("combine"));
    }

    // SAFETY: On success Windows takes ownership of visible.
    if unsafe { SetWindowRgn(window, Some(visible), true) } == 0 {
        // SAFETY: The failed call did not transfer ownership.
        let _visible_deleted = unsafe { DeleteObject(HGDIOBJ(visible.0)) };
        return Err(window_region_error("apply"));
    }

    Ok(())
}

fn window_region_error(action: &str) -> PlayerError {
    PlayerError::new(
        PlayerErrorKind::BackendFailure,
        format!(
            "failed to {action} the video host clipping region: {}",
            windows::core::Error::from_thread()
        ),
    )
}

fn native_window_handle<T>(window: &T) -> PlayerResult<usize>
where
    T: HasWindowHandle,
{
    let handle = window.window_handle().map_err(|error| {
        PlayerError::new(
            PlayerErrorKind::InvalidInput,
            format!("failed to obtain native window handle: {error}"),
        )
    })?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as usize),
        _ => Err(PlayerError::new(
            PlayerErrorKind::Unavailable,
            "the Windows video backend requires a Win32 window",
        )),
    }
}

pub(crate) fn native_video_handle(surface: &VideoSurface) -> PlayerResult<usize> {
    surface
        .platform_ref::<Win32VideoSurface>()
        .map(|surface| surface.native_handle)
        .ok_or_else(|| {
            PlayerError::new(
                PlayerErrorKind::InvalidInput,
                "the video surface is not a Win32 surface",
            )
        })
}
