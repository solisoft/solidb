//! The SoliDB Windows service, as the service manager knows it: what the
//! icon and `solidb-service` ask of it.

use std::path::{Path, PathBuf};
use std::ptr::null;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Services::*;

use crate::config::Config;
use crate::server::wide;
use crate::state::Service;

/// The service's name, to the service manager and to `sc`.
pub const NAME: &str = "SoliDB";

/// The folder of the service: its settings, and by default its data and
/// log. `%ProgramData%`, since a service belongs to the machine, not to a
/// user.
pub fn home() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\ProgramData"))
        .join("SoliDB")
}

pub fn settings_file() -> PathBuf {
    home().join("service.conf")
}

/// The service's settings: `service.conf` over the defaults. `beside` is
/// the folder of the calling executable, where `solidb.exe` is looked for.
pub fn settings(beside: &Path) -> (Config, Vec<String>) {
    let defaults = Config::defaults(&home(), beside);
    match std::fs::read_to_string(settings_file()) {
        Ok(text) => {
            let (cfg, refused) = defaults.read(&text);
            (cfg.expand(|name| std::env::var(name).ok()), refused)
        }
        Err(_) => (defaults, Vec::new()),
    }
}

/// A handle that is closed when dropped.
pub struct Handle(pub SC_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseServiceHandle(self.0) };
        }
    }
}

/// Opens the service manager with `rights`. The error is Windows' code.
pub fn manager(rights: u32) -> Result<Handle, u32> {
    unsafe {
        let handle = OpenSCManagerW(null(), null(), rights);
        if handle.is_null() {
            return Err(GetLastError());
        }
        Ok(Handle(handle))
    }
}

/// Opens the service with `rights`; `ERROR_SERVICE_DOES_NOT_EXIST` when it
/// is not installed.
pub fn open(rights: u32) -> Result<Handle, u32> {
    unsafe {
        let manager = manager(SC_MANAGER_CONNECT)?;
        let handle = OpenServiceW(manager.0, wide(NAME).as_ptr(), rights);
        if handle.is_null() {
            return Err(GetLastError());
        }
        Ok(Handle(handle))
    }
}

/// What the service manager says of the service; `None` when it is not
/// installed (or cannot be asked).
pub fn status() -> Option<Service> {
    unsafe {
        let service = open(SERVICE_QUERY_STATUS).ok()?;
        let mut status: SERVICE_STATUS = std::mem::zeroed();
        if QueryServiceStatus(service.0, &mut status) == 0 {
            return None;
        }
        Some(match status.dwCurrentState {
            SERVICE_STOPPED => Service::Stopped(match status.dwWin32ExitCode {
                ERROR_SERVICE_SPECIFIC_ERROR => status.dwServiceSpecificExitCode,
                // What a service that was installed and never started
                // reports: not a failure.
                ERROR_SERVICE_NEVER_STARTED => 0,
                code => code,
            }),
            SERVICE_START_PENDING => Service::Starting,
            SERVICE_STOP_PENDING => Service::Stopping,
            _ => Service::Running,
        })
    }
}

/// Asks the service manager to start the service. Does not wait.
pub fn start() -> Result<(), u32> {
    unsafe {
        let service = open(SERVICE_START)?;
        if StartServiceW(service.0, 0, null()) == 0 {
            return match GetLastError() {
                ERROR_SERVICE_ALREADY_RUNNING => Ok(()),
                code => Err(code),
            };
        }
        Ok(())
    }
}

/// Asks the service to stop. Does not wait.
pub fn stop() -> Result<(), u32> {
    unsafe {
        let service = open(SERVICE_STOP)?;
        let mut status: SERVICE_STATUS = std::mem::zeroed();
        if ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) == 0 {
            return match GetLastError() {
                ERROR_SERVICE_NOT_ACTIVE => Ok(()),
                code => Err(code),
            };
        }
        Ok(())
    }
}

/// A Windows error code as a sentence.
pub fn why(code: u32) -> String {
    match code {
        ERROR_ACCESS_DENIED => {
            "access denied: run this from a prompt opened as administrator".to_string()
        }
        ERROR_SERVICE_DOES_NOT_EXIST => "the SoliDB service is not installed".to_string(),
        ERROR_SERVICE_EXISTS => "the SoliDB service is already installed".to_string(),
        ERROR_SERVICE_MARKED_FOR_DELETE => {
            "the SoliDB service is being removed: close the Services console and try again"
                .to_string()
        }
        ERROR_FILE_NOT_FOUND => "file not found".to_string(),
        code => format!("Windows error {code}"),
    }
}
