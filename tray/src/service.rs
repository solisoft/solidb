//! `solidb-service`: runs the SoliDB server as a Windows service.
//!
//! The server stays the console program it is. This executable is what the
//! service manager starts: it starts `solidb.exe` without a window, sends
//! its output to a log file, and when the service manager asks it to stop,
//! or the machine shuts down, asks the server to stop the way Ctrl-C does.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicPtr, Ordering};
    use std::time::{Duration, Instant};

    use windows_sys::core::PWSTR;
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
    use windows_sys::Win32::System::Services::*;
    use windows_sys::Win32::System::Threading::*;

    use solidb_tray::config::Config;
    use solidb_tray::server::{self, wide, STOP_WITHIN};
    use solidb_tray::state::{reason, Service};
    use solidb_tray::{probe, scm};

    /// Who may do what to the service. The system and the administrators
    /// keep everything; a user signed in on the machine may start, stop and
    /// query it, so that the icon needs no elevation; services may query it.
    const RIGHTS: &str = "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)\
                          (A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)\
                          (A;;CCLCSWRPWPLOCRRC;;;IU)\
                          (A;;CCLCSWLOCRRC;;;SU)";

    const USAGE: &str = "solidb-service: runs the SoliDB server as a Windows service

USAGE
    solidb-service <COMMAND>

COMMANDS
    install     Register the service (from a prompt opened as administrator).
                It starts with the machine. Settings: %ProgramData%\\SoliDB\\service.conf
    uninstall   Stop and remove the service. Data and settings are kept.
    start       Start the service
    stop        Stop the service; the server closes its database
    status      Say whether the service is installed, running and answering
";

    pub fn main() -> i32 {
        let command = std::env::args().nth(1).unwrap_or_default();
        let done = match command.as_str() {
            "install" => install(),
            "uninstall" => uninstall(),
            "start" => start(),
            "stop" => stop(),
            "status" => status(),
            // What the service manager runs; of no use from a prompt.
            "run" => run(),
            _ => {
                print!("{USAGE}");
                return if command.is_empty() { 0 } else { 2 };
            }
        };
        match done {
            Ok(()) => 0,
            Err(code) => {
                eprintln!("solidb-service: {}", scm::why(code));
                1
            }
        }
    }

    fn beside() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default()
    }

    fn install() -> Result<(), u32> {
        let exe = std::env::current_exe().map_err(|_| ERROR_FILE_NOT_FOUND)?;
        unsafe {
            let manager = scm::manager(SC_MANAGER_CREATE_SERVICE)?;
            let service = CreateServiceW(
                manager.0,
                wide(scm::NAME).as_ptr(),
                wide("SoliDB").as_ptr(),
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_AUTO_START,
                SERVICE_ERROR_NORMAL,
                wide(&format!("\"{}\" run", exe.display())).as_ptr(),
                null(),
                null_mut(),
                null(),
                null(), // the local system account
                null(),
            );
            if service.is_null() {
                return Err(GetLastError());
            }
            let service = scm::Handle(service);

            let mut text = wide("The SoliDB database server.");
            let description = SERVICE_DESCRIPTIONW {
                lpDescription: text.as_mut_ptr(),
            };
            ChangeServiceConfig2W(
                service.0,
                SERVICE_CONFIG_DESCRIPTION,
                &description as *const _ as *const c_void,
            );

            // A server that ends on its own is started again, twice a day
            // at most: a setting that cannot work must not loop for ever.
            let mut actions = [
                SC_ACTION {
                    Type: SC_ACTION_RESTART,
                    Delay: 5_000,
                },
                SC_ACTION {
                    Type: SC_ACTION_RESTART,
                    Delay: 5_000,
                },
                SC_ACTION {
                    Type: SC_ACTION_NONE,
                    Delay: 0,
                },
            ];
            let failure = SERVICE_FAILURE_ACTIONSW {
                dwResetPeriod: 86_400,
                lpRebootMsg: null_mut(),
                lpCommand: null_mut(),
                cActions: actions.len() as u32,
                lpsaActions: actions.as_mut_ptr(),
            };
            ChangeServiceConfig2W(
                service.0,
                SERVICE_CONFIG_FAILURE_ACTIONS,
                &failure as *const _ as *const c_void,
            );
            // The service ends by reporting the server's code, not by
            // crashing: say that this counts as a failure too.
            let flag = SERVICE_FAILURE_ACTIONS_FLAG {
                fFailureActionsOnNonCrashFailures: 1,
            };
            ChangeServiceConfig2W(
                service.0,
                SERVICE_CONFIG_FAILURE_ACTIONS_FLAG,
                &flag as *const _ as *const c_void,
            );

            let mut rights = null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(RIGHTS).as_ptr(),
                SDDL_REVISION_1,
                &mut rights,
                null_mut(),
            ) != 0
            {
                SetServiceObjectSecurity(service.0, DACL_SECURITY_INFORMATION, rights);
                LocalFree(rights);
            }
        }

        let home = scm::home();
        let _ = std::fs::create_dir_all(&home);
        let file = scm::settings_file();
        if !file.exists() {
            let _ = std::fs::write(&file, Config::defaults(&home, &beside()).service_template());
        }
        let (cfg, refused) = scm::settings(&beside());
        println!("The SoliDB service is installed. It starts with the machine.");
        println!("  server    {}", cfg.exe.display());
        println!("  data      {}", cfg.data_dir.display());
        println!("  port      {}", cfg.port);
        println!("  log       {}", cfg.log.display());
        println!("  settings  {}", file.display());
        for line in refused {
            println!("  not read in the settings: {line}");
        }
        if !cfg.exe.is_file() {
            println!("The server was not found there: set 'exe' in the settings before starting.");
        }
        println!("Start it now with: solidb-service start");
        Ok(())
    }

    fn uninstall() -> Result<(), u32> {
        const DELETE: u32 = 0x0001_0000;
        let service = scm::open(SERVICE_STOP | SERVICE_QUERY_STATUS | DELETE)?;
        if !matches!(scm::status(), Some(Service::Stopped(_))) {
            scm::stop()?;
            wait_for(STOP_WITHIN + Duration::from_secs(10), |s| {
                matches!(s, Service::Stopped(_))
            });
        }
        unsafe {
            if DeleteService(service.0) == 0 {
                return Err(GetLastError());
            }
        }
        println!(
            "The SoliDB service is removed. Its data and settings are kept in {}.",
            scm::home().display()
        );
        Ok(())
    }

    fn start() -> Result<(), u32> {
        scm::start()?;
        let port = scm::settings(&beside()).0.port;
        // The service says it runs as soon as the server's process exists;
        // wait for the port to say that the server is up.
        let until = Instant::now() + Duration::from_secs(60);
        loop {
            match scm::status() {
                Some(Service::Stopped(code)) => {
                    eprintln!(
                        "solidb-service: the server ended: {}. See the log.",
                        reason(code)
                    );
                    std::process::exit(1);
                }
                Some(Service::Running) if probe::answers(port) => {
                    println!("SoliDB is running on port {port}.");
                    return Ok(());
                }
                _ if Instant::now() >= until => {
                    println!(
                        "The service is started; the server does not answer on port {port} yet."
                    );
                    return Ok(());
                }
                _ => std::thread::sleep(Duration::from_millis(500)),
            }
        }
    }

    fn stop() -> Result<(), u32> {
        scm::stop()?;
        if wait_for(STOP_WITHIN + Duration::from_secs(10), |s| {
            matches!(s, Service::Stopped(_))
        }) {
            println!("SoliDB is stopped.");
        } else {
            println!("The service was asked to stop and has not stopped yet.");
        }
        Ok(())
    }

    fn status() -> Result<(), u32> {
        let Some(service) = scm::status() else {
            println!("The SoliDB service is not installed.");
            return Ok(());
        };
        let port = scm::settings(&beside()).0.port;
        match service {
            Service::Stopped(0) => println!("stopped"),
            Service::Stopped(code) => println!("stopped: the server ended, {}", reason(code)),
            Service::Starting => println!("starting"),
            Service::Stopping => println!("stopping"),
            Service::Running if probe::answers(port) => println!("running on port {port}"),
            Service::Running => println!("started; the server does not answer on port {port}"),
        }
        Ok(())
    }

    /// Waits until the service is as `wanted`; false if it never was.
    fn wait_for(within: Duration, wanted: impl Fn(Service) -> bool) -> bool {
        let until = Instant::now() + within;
        loop {
            if scm::status().is_some_and(&wanted) {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    // ---- what the service manager runs ----

    /// The handle the service reports its state through.
    static REPORT: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
    /// The event set when the service is asked to stop.
    static STOP: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

    fn run() -> Result<(), u32> {
        let mut name = wide(scm::NAME);
        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: name.as_mut_ptr(),
                lpServiceProc: Some(service_main),
            },
            SERVICE_TABLE_ENTRYW {
                lpServiceName: null_mut(),
                lpServiceProc: None,
            },
        ];
        unsafe {
            // Returns when the service has stopped.
            if StartServiceCtrlDispatcherW(table.as_ptr()) == 0 {
                let code = GetLastError();
                if code == ERROR_FAILED_SERVICE_CONTROLLER_CONNECT {
                    eprintln!(
                        "solidb-service: 'run' is what the service manager calls. Use 'start'."
                    );
                    std::process::exit(2);
                }
                return Err(code);
            }
        }
        Ok(())
    }

    unsafe fn report(state: u32, code: u32, wait_hint: u32) {
        let pending = state == SERVICE_START_PENDING || state == SERVICE_STOP_PENDING;
        let status = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: state,
            // PRESHUTDOWN rather than SHUTDOWN: a machine that shuts down
            // then gives the server minutes to close its database, not
            // seconds.
            dwControlsAccepted: if state == SERVICE_RUNNING {
                SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_PRESHUTDOWN
            } else {
                0
            },
            dwWin32ExitCode: if code == 0 {
                NO_ERROR
            } else {
                ERROR_SERVICE_SPECIFIC_ERROR
            },
            dwServiceSpecificExitCode: code,
            dwCheckPoint: pending as u32,
            dwWaitHint: wait_hint,
        };
        SetServiceStatus(REPORT.load(Ordering::SeqCst), &status);
    }

    unsafe extern "system" fn handler(
        control: u32,
        _event: u32,
        _data: *mut c_void,
        _context: *mut c_void,
    ) -> u32 {
        match control {
            SERVICE_CONTROL_STOP | SERVICE_CONTROL_PRESHUTDOWN => {
                SetEvent(STOP.load(Ordering::SeqCst));
                NO_ERROR
            }
            SERVICE_CONTROL_INTERROGATE => NO_ERROR,
            _ => ERROR_CALL_NOT_IMPLEMENTED,
        }
    }

    unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
        let report_handle =
            RegisterServiceCtrlHandlerExW(wide(scm::NAME).as_ptr(), Some(handler), null_mut());
        if report_handle.is_null() {
            return;
        }
        REPORT.store(report_handle, Ordering::SeqCst);
        let stop = CreateEventW(null(), 1, 0, null());
        STOP.store(stop, Ordering::SeqCst);
        report(SERVICE_START_PENDING, 0, 10_000);

        let (cfg, _) = scm::settings(&beside());
        let child = match server::spawn(&cfg, &scm::home()) {
            Ok(child) => child,
            Err(code) => return report(SERVICE_STOPPED, code, 0),
        };
        report(SERVICE_RUNNING, 0, 0);

        let either = [child.handle, stop];
        let first = WaitForMultipleObjects(2, either.as_ptr(), 0, INFINITE);
        let code = if first == WAIT_OBJECT_0 {
            // The server ended on its own: its code is the service's.
            let mut code = 0u32;
            GetExitCodeProcess(child.handle, &mut code);
            code
        } else {
            let limit = STOP_WITHIN.as_millis() as u32;
            report(SERVICE_STOP_PENDING, 0, limit + 10_000);
            let gone = server::ask_to_stop(&child)
                && WaitForSingleObject(child.handle, limit) == WAIT_OBJECT_0;
            if !gone {
                TerminateProcess(child.handle, 1);
                WaitForSingleObject(child.handle, 5_000);
            }
            0
        };
        CloseHandle(child.handle);
        report(SERVICE_STOPPED, code, 0);
    }
}

fn main() {
    #[cfg(windows)]
    std::process::exit(imp::main());
    #[cfg(not(windows))]
    {
        eprintln!("solidb-service runs on Windows only.");
        std::process::exit(2);
    }
}
