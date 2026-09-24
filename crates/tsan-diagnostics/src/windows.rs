//! The faulting thread only publishes a fixed-size request and waits briefly.
//! The separate collector process does all file I/O and DbgHelp work.
use std::ffi::OsString;
use std::fs::{self, File};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use ::windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use ::windows::Win32::System::Diagnostics::Debug::*;
use ::windows::Win32::System::Threading::*;
use ::windows::core::{HSTRING, PWSTR};

#[repr(C)]
struct Request {
    exception: AtomicU64,
    thread: AtomicU32,
    code: AtomicU32,
}
#[repr(C)]
#[derive(Default)]
struct RemoteRequest {
    exception: u64,
    thread: u32,
    code: u32,
}
static REQUEST: Request = Request {
    exception: AtomicU64::new(0),
    thread: AtomicU32::new(0),
    code: AtomicU32::new(0),
};
static CRASHING: AtomicBool = AtomicBool::new(false);
static REQUEST_EVENT: AtomicUsize = AtomicUsize::new(0);
static DONE_EVENT: AtomicUsize = AtomicUsize::new(0);

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: This wrapper exclusively owns a successfully created/opened handle.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct Guard {
    _request: Handle,
    _done: Handle,
    _ready: Handle,
    _child: Child,
    previous: LPTOP_LEVEL_EXCEPTION_FILTER,
}
impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: Restore the process handler before releasing its event handles.
        unsafe {
            SetUnhandledExceptionFilter(self.previous);
        }
        REQUEST_EVENT.store(0, Ordering::Release);
        DONE_EVENT.store(0, Ordering::Release);
    }
}

unsafe extern "system" fn exception_filter(info: *const EXCEPTION_POINTERS) -> i32 {
    let request = REQUEST_EVENT.load(Ordering::Acquire);
    let done = DONE_EVENT.load(Ordering::Acquire);
    if request == 0 || done == 0 || info.is_null() {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    if !CRASHING.swap(true, Ordering::AcqRel) {
        REQUEST.exception.store(info as u64, Ordering::Release);
        // SAFETY: Windows supplies the exception pointers for this synchronous callback.
        unsafe {
            REQUEST
                .thread
                .store(GetCurrentThreadId(), Ordering::Release);
            let record = (*info).ExceptionRecord;
            if !record.is_null() {
                REQUEST
                    .code
                    .store((*record).ExceptionCode.0 as u32, Ordering::Release);
            }
            let _ = SetEvent(HANDLE(request as _));
        }
    }
    // SAFETY: Guard owns the manual-reset event throughout application execution.
    // Do not allocate, acquire Rust mutexes, or call DbgHelp from a corrupt process.
    unsafe {
        WaitForSingleObject(HANDLE(done as _), 15_000);
    }
    EXCEPTION_EXECUTE_HANDLER
}

fn create_event(name: &str) -> Result<Handle, String> {
    // SAFETY: The temporary UTF-16 name is valid for the call. Handles are not inheritable.
    unsafe {
        CreateEventW(None, true, false, &HSTRING::from(name))
            .map(Handle)
            .map_err(|e| e.to_string())
    }
}
fn open_event(name: &str) -> Result<Handle, String> {
    // SAFETY: The generated event name is valid for the call.
    unsafe {
        OpenEventW(
            EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
            false,
            &HSTRING::from(name),
        )
        .map(Handle)
        .map_err(|e| e.to_string())
    }
}

pub fn install(session: &Path) -> Result<Guard, String> {
    let pid = std::process::id();
    let name = session
        .file_name()
        .ok_or("Missing session name")?
        .to_string_lossy();
    let prefix = format!("Local\\TSAnalyzer-{pid}-{name}");
    let request = create_event(&format!("{prefix}-request"))?;
    let done = create_event(&format!("{prefix}-done"))?;
    let ready = create_event(&format!("{prefix}-ready"))?;
    let stderr = File::create(session.join("collector-error.txt")).map_err(|e| e.to_string())?;
    let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("--tsan-crash-helper")
        .arg(pid.to_string())
        .arg((&REQUEST as *const Request as usize).to_string())
        .arg(&prefix)
        .arg(session)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .map_err(|e| format!("Could not start crash collector: {e}"))?;
    // SAFETY: ready is owned here; the helper signals only after opening the parent and session lock.
    if unsafe { WaitForSingleObject(ready.0, 5_000) } != WAIT_OBJECT_0 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Crash collector did not become ready; see collector-error.txt".into());
    }
    REQUEST_EVENT.store(request.0.0 as usize, Ordering::Release);
    DONE_EVENT.store(done.0.0 as usize, Ordering::Release);
    // SAFETY: The callback has static lifetime and only accesses live owned event handles.
    let previous = unsafe { SetUnhandledExceptionFilter(Some(exception_filter)) };
    Ok(Guard {
        _request: request,
        _done: done,
        _ready: ready,
        _child: child,
        previous,
    })
}

fn confirm_executable(process: HANDLE) -> Result<(), String> {
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    // SAFETY: The process handle is valid; the output buffer has the advertised capacity.
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
        .map_err(|e| e.to_string())?;
    }
    use std::os::windows::ffi::OsStringExt;
    let target = std::path::PathBuf::from(OsString::from_wide(&buffer[..length as usize]));
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    if fs::canonicalize(target).map_err(|e| e.to_string())?
        != fs::canonicalize(current).map_err(|e| e.to_string())?
    {
        return Err("Crash collector may only monitor the same executable".into());
    }
    Ok(())
}

pub fn run_helper(args: &[OsString]) -> Result<(), String> {
    if args.len() != 4 {
        return Err("Invalid crash collector arguments".into());
    }
    let pid = args[0]
        .to_str()
        .ok_or("Invalid process ID")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let address = args[1]
        .to_str()
        .ok_or("Invalid request address")?
        .parse::<usize>()
        .map_err(|e| e.to_string())?;
    if pid == std::process::id() || address == 0 {
        return Err("Invalid crash collector target".into());
    }
    let prefix = args[2].to_str().ok_or("Invalid event name")?;
    let session = Path::new(&args[3]);
    if !session.is_absolute()
        || fs::read_to_string(session.join("identity.txt")).map_err(|e| e.to_string())?
            != super::SESSION_MARKER
    {
        return Err("Invalid diagnostic session".into());
    }
    let _lock = super::session_lock(session).map_err(|e| e.to_string())?;
    let request = open_event(&format!("{prefix}-request"))?;
    let done = open_event(&format!("{prefix}-done"))?;
    let ready = open_event(&format!("{prefix}-ready"))?;
    // SAFETY: Request only read/query/duplicate/synchronize rights for the launching application.
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_DUP_HANDLE | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    }
    .map(Handle)
    .map_err(|e| e.to_string())?;
    confirm_executable(process.0)?;
    fs::write(
        session.join("native-crash.txt"),
        "Collector ready; waiting for an unhandled native exception.\n",
    )
    .map_err(|e| e.to_string())?;
    // SAFETY: All events and the parent process handle remain owned throughout this wait.
    unsafe {
        SetEvent(ready.0).map_err(|e| e.to_string())?;
    }
    let result = unsafe { WaitForMultipleObjects(&[request.0, process.0], false, INFINITE) };
    if result == WAIT_OBJECT_0 {
        let dumped = write_dump(process.0, pid, address, session);
        if let Err(error) = &dumped {
            let _ = fs::write(
                session.join("native-crash.txt"),
                format!("Dump failed: {error}\n"),
            );
        }
        // Release the crashing process even if disk access or DbgHelp failed.
        unsafe {
            let _ = SetEvent(done.0);
        }
        dumped
    } else if result.0 == WAIT_OBJECT_0.0 + 1 {
        let mut code = 0;
        // SAFETY: A signaled process handle retains its exit code.
        unsafe {
            GetExitCodeProcess(process.0, &mut code).map_err(|e| e.to_string())?;
        }
        let state = if code == 0 {
            "normal"
        } else {
            "nonzero exit; no native exception reached the collector"
        };
        fs::write(
            session.join("process-exit.txt"),
            format!("pid={pid}\nexit_code=0x{code:08X}\nstate={state}\n"),
        )
        .map_err(|e| e.to_string())
    } else {
        Err(format!("Crash collector wait failed: {}", result.0))
    }
}

fn write_dump(process: HANDLE, pid: u32, address: usize, session: &Path) -> Result<(), String> {
    let mut request = RemoteRequest::default();
    let mut bytes = 0;
    // SAFETY: ReadProcessMemory validates remote memory. The local output is a sized POD record.
    unsafe {
        ReadProcessMemory(
            process,
            address as _,
            (&mut request as *mut RemoteRequest).cast(),
            size_of::<RemoteRequest>(),
            Some(&mut bytes),
        )
        .map_err(|e| e.to_string())?;
    }
    if bytes != size_of::<RemoteRequest>() || request.exception == 0 || request.thread == 0 {
        return Err("Incomplete native exception request".into());
    }
    let file = File::create(session.join("crash.dmp")).map_err(|e| e.to_string())?;
    let exception = MINIDUMP_EXCEPTION_INFORMATION {
        ThreadId: request.thread,
        ExceptionPointers: request.exception as *mut EXCEPTION_POINTERS,
        ClientPointers: true.into(),
    };
    // SAFETY: Exception pointers refer to the waiting parent (ClientPointers=TRUE).
    // This is the sole DbgHelp caller in the collector process. No full heap dump is requested.
    unsafe {
        MiniDumpWriteDump(
            process,
            pid,
            HANDLE(file.as_raw_handle()),
            MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules,
            Some(&exception),
            None,
            None,
        )
        .map_err(|e| e.to_string())?;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    fs::write(
        session.join("native-crash.txt"),
        format!(
            "Dump complete\npid={pid}\nthread_id={}\nexception_code=0x{:08X}\ndump=crash.dmp\n",
            request.thread, request.code
        ),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
