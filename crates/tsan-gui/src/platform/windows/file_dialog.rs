use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT,
    FOS_PATHMUSTEXIST, FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog,
    SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, HSTRING, PWSTR, w};

struct ComApartment;

impl Drop for ComApartment {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: This guard is created only after COM initialization succeeds on
        // the current dialog thread, and it is dropped on that same thread.
        unsafe {
            CoUninitialize();
        }
    }
}

struct CoTaskMemWideString(PWSTR);

impl CoTaskMemWideString {
    #[allow(unsafe_code)]
    fn to_path_buf(&self) -> PathBuf {
        // SAFETY: IFileDialog owns a valid null-terminated PWSTR until it is
        // released by CoTaskMemFree in this wrapper's Drop implementation.
        let wide = unsafe { self.0.as_wide() };
        PathBuf::from(OsString::from_wide(wide))
    }
}

impl Drop for CoTaskMemWideString {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: GetDisplayName allocates this pointer with the COM task
        // allocator, and this wrapper releases it exactly once.
        unsafe {
            CoTaskMemFree(Some(self.0.as_ptr().cast()));
        }
    }
}

pub fn open_transport_stream_dialog(
    owner: &winit::window::Window,
) -> Result<Option<Vec<PathBuf>>, String> {
    let owner_handle = owner
        .window_handle()
        .map_err(|error| format!("failed to obtain the window handle: {error}"))?;
    let RawWindowHandle::Win32(owner_handle) = owner_handle.as_raw() else {
        return Err("the Windows file dialog requires a Win32 window".to_owned());
    };
    let owner_handle = owner_handle.hwnd.get();

    show_dialog(owner_handle).map_err(|error| format!("failed to open the file dialog: {error}"))
}

pub fn save_log_dialog(owner: &winit::window::Window) -> Result<Option<PathBuf>, String> {
    let owner_handle = owner
        .window_handle()
        .map_err(|error| format!("failed to obtain the window handle: {error}"))?;
    let RawWindowHandle::Win32(owner_handle) = owner_handle.as_raw() else {
        return Err("the Windows file dialog requires a Win32 window".to_owned());
    };
    let owner_handle = owner_handle.hwnd.get();

    show_save_log_dialog(owner_handle)
        .map_err(|error| format!("failed to open the log export dialog: {error}"))
}

pub fn save_transport_stream_dialog(
    owner: &winit::window::Window,
) -> Result<Option<PathBuf>, String> {
    let owner_handle = owner
        .window_handle()
        .map_err(|error| format!("failed to obtain the window handle: {error}"))?;
    let RawWindowHandle::Win32(owner_handle) = owner_handle.as_raw() else {
        return Err("the Windows file dialog requires a Win32 window".to_owned());
    };
    let owner_handle = owner_handle.hwnd.get();

    show_save_transport_stream_dialog(owner_handle)
        .map_err(|error| format!("failed to open the recording output dialog: {error}"))
}

pub fn save_analyzed_report_dialog(
    owner: &winit::window::Window,
    format: crate::report_export::ReportFormat,
) -> Result<Option<PathBuf>, String> {
    let owner_handle = owner
        .window_handle()
        .map_err(|error| format!("failed to obtain the window handle: {error}"))?;
    let RawWindowHandle::Win32(owner_handle) = owner_handle.as_raw() else {
        return Err("the Windows file dialog requires a Win32 window".to_owned());
    };
    show_save_report_dialog(owner_handle.hwnd.get(), format)
        .map_err(|error| format!("failed to open report export dialog: {error}"))
}

fn timestamp_name() -> String {
    #[allow(unsafe_code)]
    // SAFETY: GetLocalTime fills the caller-owned SYSTEMTIME structure.
    let now = unsafe { GetLocalTime() };
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    )
}

#[allow(unsafe_code)]
fn show_dialog(owner_handle: isize) -> windows::core::Result<Option<Vec<PathBuf>>> {
    // SAFETY: The dialog is created and released on the GUI thread. Showing it
    // there lets the modal COM dialog keep pumping messages for its owner HWND.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = ComApartment;
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let filters = [
            COMDLG_FILTERSPEC {
                pszName: w!("MPEG transport stream"),
                pszSpec: w!("*.ts;*.m2ts;*.mts;*.trp"),
            },
            COMDLG_FILTERSPEC {
                pszName: w!("All files"),
                pszSpec: w!("*.*"),
            },
        ];

        dialog.SetTitle(w!("Import Transport Stream"))?;
        dialog.SetFileTypes(&filters)?;
        dialog.SetOptions(
            dialog.GetOptions()?
                | FOS_ALLOWMULTISELECT
                | FOS_FORCEFILESYSTEM
                | FOS_FILEMUSTEXIST
                | FOS_PATHMUSTEXIST,
        )?;

        if let Err(error) = dialog.Show(Some(HWND(owner_handle as *mut _))) {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error);
        }

        let items = dialog.GetResults()?;
        let count = items.GetCount()?;
        let mut paths = Vec::with_capacity(count as usize);
        for index in 0..count {
            let item = items.GetItemAt(index)?;
            let path = CoTaskMemWideString(item.GetDisplayName(SIGDN_FILESYSPATH)?);
            paths.push(path.to_path_buf());
        }
        Ok(Some(paths))
    }
}

#[allow(unsafe_code)]
fn show_save_report_dialog(
    owner_handle: isize,
    format: crate::report_export::ReportFormat,
) -> windows::core::Result<Option<PathBuf>> {
    // SAFETY: The modal dialog and its COM apartment live on this GUI thread.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = ComApartment;
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        let filter = match format {
            crate::report_export::ReportFormat::Cbor => COMDLG_FILTERSPEC {
                pszName: w!("CBOR data"),
                pszSpec: w!("*.cbor"),
            },
            crate::report_export::ReportFormat::Xlsx => COMDLG_FILTERSPEC {
                pszName: w!("Excel workbook"),
                pszSpec: w!("*.xlsx"),
            },
            crate::report_export::ReportFormat::Latex => COMDLG_FILTERSPEC {
                pszName: w!("LaTeX source"),
                pszSpec: w!("*.tex"),
            },
            crate::report_export::ReportFormat::Pdf => COMDLG_FILTERSPEC {
                pszName: w!("PDF report"),
                pszSpec: w!("*.pdf"),
            },
        };
        dialog.SetTitle(w!("Export Analyzed Report"))?;
        dialog.SetFileTypes(&[filter])?;
        dialog.SetDefaultExtension(&HSTRING::from(format.extension()))?;
        dialog.SetFileName(&HSTRING::from(format!(
            "ts-analyzer_report_{}.{}",
            timestamp_name(),
            format.extension()
        )))?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT)?;
        if let Err(error) = dialog.Show(Some(HWND(owner_handle as *mut _))) {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error);
        }
        let item = dialog.GetResult()?;
        let path = CoTaskMemWideString(item.GetDisplayName(SIGDN_FILESYSPATH)?);
        Ok(Some(path.to_path_buf()))
    }
}

#[allow(unsafe_code)]
fn show_save_log_dialog(owner_handle: isize) -> windows::core::Result<Option<PathBuf>> {
    // SAFETY: The dialog is created and released on the GUI thread. Showing it
    // there lets the modal COM dialog keep pumping messages for its owner HWND.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = ComApartment;
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        let filters = [
            COMDLG_FILTERSPEC {
                pszName: w!("Log files"),
                pszSpec: w!("*.log"),
            },
            COMDLG_FILTERSPEC {
                pszName: w!("Text files"),
                pszSpec: w!("*.txt"),
            },
        ];

        dialog.SetTitle(w!("Dump TS Analyzer Log"))?;
        dialog.SetFileTypes(&filters)?;
        dialog.SetDefaultExtension(w!("log"))?;
        dialog.SetFileName(&HSTRING::from(format!(
            "ts-analyzer_log_{}.log",
            timestamp_name()
        )))?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT)?;

        if let Err(error) = dialog.Show(Some(HWND(owner_handle as *mut _))) {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error);
        }

        let item = dialog.GetResult()?;
        let path = CoTaskMemWideString(item.GetDisplayName(SIGDN_FILESYSPATH)?);
        Ok(Some(path.to_path_buf()))
    }
}

#[allow(unsafe_code)]
fn show_save_transport_stream_dialog(
    owner_handle: isize,
) -> windows::core::Result<Option<PathBuf>> {
    // SAFETY: The dialog is created and released on the GUI thread. Showing it
    // there lets the modal COM dialog keep pumping messages for its owner HWND.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = ComApartment;
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        let filters = [COMDLG_FILTERSPEC {
            pszName: w!("MPEG transport stream"),
            pszSpec: w!("*.ts"),
        }];

        dialog.SetTitle(w!("Select Recording Output File"))?;
        dialog.SetFileTypes(&filters)?;
        dialog.SetDefaultExtension(w!("ts"))?;
        dialog.SetFileName(w!("recording.ts"))?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT)?;

        if let Err(error) = dialog.Show(Some(HWND(owner_handle as *mut _))) {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error);
        }

        let item = dialog.GetResult()?;
        let path = CoTaskMemWideString(item.GetDisplayName(SIGDN_FILESYSPATH)?);
        Ok(Some(path.to_path_buf()))
    }
}
