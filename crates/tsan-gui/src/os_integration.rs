use std::process::Command;

pub const PROJECT_URL: &str = "https://github.com/LunaticGhoulPiano/TS-Analyzer";

pub fn open_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Only HTTPS links are supported".into());
    }
    platform_open(url)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn platform_open(url: &str) -> Result<(), String> {
    use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    use windows::core::PCWSTR;
    let url = url.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let action = "open\0".encode_utf16().collect::<Vec<_>>();
    // SAFETY: NUL-terminated strings remain valid for the synchronous ShellExecuteW call.
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(action.as_ptr()),
            PCWSTR(url.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "Default browser could not open the link (ShellExecute code {})",
            result.0 as isize
        ))
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn platform_open(url: &str) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn platform_open(_: &str) -> Result<(), String> {
    Err(tsan_platform::OperatingSystem::current().unavailable("Default browser integration"))
}
