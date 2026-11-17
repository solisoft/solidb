//! Starting the server without a window and asking it to stop: what the
//! icon and the service both do.

use std::ffi::c_void;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::Duration;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::Threading::*;

use crate::config::Config;

/// How long a server asked to stop is given before it is ended.
pub const STOP_WITHIN: Duration = Duration::from_secs(60);

pub struct Child {
    pub handle: HANDLE,
    pub pid: u32,
}

/// Starts the server without a window, its output appended to the log, in
/// `home`: the server reads a `.env` where it is started, so it is started
/// in a folder of its own, not wherever its starter happens to be. The
/// error is Windows' code.
///
/// # Safety
/// Calls Win32; the returned handle is the caller's to close.
pub unsafe fn spawn(cfg: &Config, home: &Path) -> Result<Child, u32> {
    if !cfg.exe.is_file() {
        return Err(ERROR_FILE_NOT_FOUND);
    }
    // The server colours its output for a terminal; it goes to a file here.
    std::env::set_var("NO_COLOR", "1");
    let _ = std::fs::create_dir_all(&cfg.data_dir);
    start(&cfg.command_line(), home, &cfg.log, None)
}

/// Starts `line` without a window, in `cwd`, its output appended to `log`.
/// `environment` is a block as `CreateProcessW` takes one; without it the
/// process gets the caller's. The error is Windows' code.
///
/// # Safety
/// Calls Win32; the returned handle is the caller's to close.
pub unsafe fn start(
    line: &str,
    cwd: &Path,
    log: &Path,
    environment: Option<&[u16]>,
) -> Result<Child, u32> {
    if let Some(dir) = log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    let inherit = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let log = CreateFileW(
        wide(&log.to_string_lossy()).as_ptr(),
        FILE_APPEND_DATA,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        &inherit,
        OPEN_ALWAYS,
        FILE_ATTRIBUTE_NORMAL,
        null_mut(),
    );
    let nul = CreateFileW(
        wide("NUL").as_ptr(),
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        &inherit,
        OPEN_EXISTING,
        0,
        null_mut(),
    );

    let mut si: STARTUPINFOW = std::mem::zeroed();
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = nul;
    si.hStdOutput = log;
    si.hStdError = log;
    let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
    // CreateProcessW may write into the command line it is given.
    let mut line = wide(line);
    let cwd = wide(&cwd.to_string_lossy());
    // A console without a window: the server needs one to be sent Ctrl-C.
    let made = CreateProcessW(
        null(),
        line.as_mut_ptr(),
        null(),
        null(),
        1,
        CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
        environment.map_or(null(), |block| block.as_ptr() as *const c_void),
        cwd.as_ptr(),
        &si,
        &mut pi,
    );
    let error = GetLastError();
    if log != INVALID_HANDLE_VALUE {
        CloseHandle(log);
    }
    if nul != INVALID_HANDLE_VALUE {
        CloseHandle(nul);
    }
    if made == 0 {
        return Err(error);
    }
    CloseHandle(pi.hThread);
    Ok(Child {
        handle: pi.hProcess,
        pid: pi.dwProcessId,
    })
}

/// Runs `line` to its end, without a window, and gives its exit code.
///
/// # Safety
/// Calls Win32.
pub unsafe fn run(line: &str, cwd: &Path, log: &Path) -> Result<u32, u32> {
    let child = start(line, cwd, log, None)?;
    WaitForSingleObject(child.handle, INFINITE);
    let mut code = 0u32;
    GetExitCodeProcess(child.handle, &mut code);
    CloseHandle(child.handle);
    Ok(code)
}

/// Asks the server to stop as Ctrl-C does, so that it closes its database.
/// False when it could not be reached that way.
///
/// # Safety
/// Calls Win32. The calling process leaves any console it had.
pub unsafe fn ask_to_stop(child: &Child) -> bool {
    // Ctrl-C goes to every process of a console. The caller has none of its
    // own to keep; it joins the server's for the time of sending, with its
    // own handling turned off so that it does not end itself.
    FreeConsole();
    if AttachConsole(child.pid) == 0 {
        return false;
    }
    SetConsoleCtrlHandler(None, 1);
    GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
    // The event is delivered on another thread: leave the console only
    // once it has had the time to be ignored here.
    std::thread::sleep(Duration::from_millis(200));
    FreeConsole();
    SetConsoleCtrlHandler(None, 0);
    true
}

/// A string as Windows takes it: UTF-16 with a closing zero.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
