//! Everything that talks to Windows.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use solidb_tray::config::Config;
use solidb_tray::server::{self, wide, Child, STOP_WITHIN};
use solidb_tray::state::{Process, Service, State};
use solidb_tray::{console, probe, scm, state};

/// The notification area tells the window about the mouse with this.
const WM_TRAY: u32 = WM_APP + 1;
/// The probing thread tells the window what it found: `wparam` is 1 when
/// the server answered.
const WM_PROBED: u32 = WM_APP + 2;

/// A thread tells the window how the console's start went: `wparam` is one
/// of the `CONSOLE_*` below.
const WM_CONSOLE: u32 = WM_APP + 3;
/// The runtime is there: the console can be started.
const CONSOLE_FETCHED: usize = 0;
/// The console answers: the browser can be sent to it.
const CONSOLE_UP: usize = 1;
const CONSOLE_NO_DOWNLOAD: usize = 2;
const CONSOLE_WRONG_DIGEST: usize = 3;
const CONSOLE_NO_UNPACK: usize = 4;
const CONSOLE_SILENT: usize = 5;

/// How long a console that was started is given to answer.
const CONSOLE_WITHIN: Duration = Duration::from_secs(40);

const CMD_START: usize = 1;
const CMD_STOP: usize = 2;
const CMD_RESTART: usize = 3;
const CMD_CONSOLE: usize = 4;
const CMD_DATA: usize = 5;
const CMD_LOG: usize = 6;
const CMD_SETTINGS: usize = 7;
const CMD_AUTOSTART: usize = 8;
const CMD_QUIT: usize = 9;

const PROBE_EVERY: Duration = Duration::from_millis(1500);

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "SoliDB";

struct App {
    cfg: Config,
    home: PathBuf,
    /// The Windows service is installed: the icon drives it, and starts no
    /// server of its own.
    service: bool,
    hwnd: HWND,
    child: Option<Child>,
    state: State,
    /// Set when the server was asked to stop: when it is ended if still there.
    stop_by: Option<Instant>,
    /// Start again once the server asked to stop is gone.
    restart: bool,
    /// The admin console this icon started.
    console: Option<Child>,
    /// The console is being fetched or started: a second click waits.
    console_busy: bool,
    icons: Icons,
    /// The message the shell sends when the taskbar is created again.
    taskbar_created: u32,
}

struct Icons {
    stopped: HICON,
    busy: HICON,
    running: HICON,
    failed: HICON,
}

pub fn run() {
    unsafe {
        // One icon per session: a second start only brings the first one's menu up.
        let mutex = CreateMutexW(null(), 0, wide("Local\\SoliDB.Tray").as_ptr());
        if !mutex.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
            let other = FindWindowW(wide(CLASS).as_ptr(), null());
            if !other.is_null() {
                PostMessageW(other, WM_TRAY, 0, WM_RBUTTONUP as LPARAM);
            }
            return;
        }

        let beside = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        // A `tray.conf` beside the executable makes that folder the icon's
        // own: a copy on a USB key or in a test folder keeps to itself.
        let home = if beside.join("tray.conf").is_file() {
            beside.clone()
        } else {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| beside.clone())
                .join("SoliDB")
        };
        let _ = std::fs::create_dir_all(&home);
        let (mut cfg, mut refused) = settings(&home, &beside);
        let service = scm::status().is_some();
        if service {
            // The server is the service's: where it runs is the service's
            // to say. The icon keeps only what is its own (the console).
            let (theirs, more) = scm::settings(&beside);
            cfg = Config {
                console_url: cfg.console_url,
                console_port: cfg.console_port,
                soli: cfg.soli,
                console_app: cfg.console_app,
                ..theirs
            };
            refused = more;
        }

        let instance = GetModuleHandleW(null());
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        // A window that is never shown: it is only there to receive messages.
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("SoliDB").as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }

        let app = Box::into_raw(Box::new(App {
            cfg,
            home,
            service,
            hwnd,
            child: None,
            state: State::Stopped,
            stop_by: None,
            restart: false,
            console: None,
            console_busy: false,
            icons: Icons {
                stopped: disc(0x8A8F98),
                busy: disc(0xE0A21B),
                running: disc(0x2FA84F),
                failed: disc(0xD2402F),
            },
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
        let app = &mut *app;

        app.adopt();
        app.show(NIM_ADD);
        if !refused.is_empty() {
            app.balloon(
                "SoliDB settings",
                &format!("Not read in tray.conf: {}", refused.join("; ")),
            );
        }

        // One look before deciding to start: the port may already be served.
        let answering = probe::answers(app.cfg.port);
        app.look(answering);
        if !app.service && app.cfg.start_server && app.state.can_start() {
            app.start();
        }

        let port = app.cfg.port;
        let target = hwnd as isize;
        std::thread::spawn(move || loop {
            let answering = probe::answers(port);
            if PostMessageW(target as HWND, WM_PROBED, answering as WPARAM, 0) == 0 {
                return; // the window is gone
            }
            std::thread::sleep(PROBE_EVERY);
        });

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        CloseHandle(mutex);
    }
}

const CLASS: &str = "SoliDBTrayWindow";

/// Reads `tray.conf`, writing it with the defaults the first time.
fn settings(home: &Path, beside: &Path) -> (Config, Vec<String>) {
    let defaults = Config::defaults(home, beside);
    let path = home.join("tray.conf");
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let (cfg, refused) = defaults.read(&text);
            (cfg.expand(|name| std::env::var(name).ok()), refused)
        }
        Err(_) => {
            let _ = std::fs::write(&path, defaults.template());
            (defaults, Vec::new())
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if app.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let app = &mut *app;
    match msg {
        WM_TRAY => {
            let event = lparam as u32 & 0xFFFF;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                app.menu();
            }
            0
        }
        WM_PROBED => {
            app.look(wparam != 0);
            0
        }
        WM_CONSOLE => {
            app.console_went(wparam);
            0
        }
        WM_COMMAND => {
            app.command(wparam & 0xFFFF);
            0
        }
        // Windows is closing the session: leave the server to it.
        WM_ENDSESSION | WM_CLOSE => {
            app.end_console();
            app.show(NIM_DELETE);
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        m if m == app.taskbar_created => {
            // Explorer started again and forgot every icon.
            app.show(NIM_ADD);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

impl App {
    fn pid_file(&self) -> PathBuf {
        self.home.join("solidb.pid")
    }

    /// Takes back a server this icon started before it was closed: its pid
    /// was left in a file, and is only believed if that process is a
    /// `solidb.exe`.
    unsafe fn adopt(&mut self) {
        if self.service {
            return;
        }
        let Some(pid) = std::fs::read_to_string(self.pid_file())
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
        else {
            return;
        };
        let handle = OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        );
        if handle.is_null() {
            let _ = std::fs::remove_file(self.pid_file());
            return;
        }
        let mut name = [0u16; 1024];
        let mut len = name.len() as u32;
        let known = QueryFullProcessImageNameW(handle, 0, name.as_mut_ptr(), &mut len) != 0;
        let image = String::from_utf16_lossy(&name[..len as usize]).to_lowercase();
        let ours = known
            && Path::new(&image).file_name().and_then(|n| n.to_str()) == Some("solidb.exe")
            && WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        if ours {
            self.child = Some(Child { handle, pid });
        } else {
            CloseHandle(handle);
            let _ = std::fs::remove_file(self.pid_file());
        }
    }

    /// Starts the server: the service when there is one, else a process of
    /// the icon's own, without a window, its output appended to the log.
    unsafe fn start(&mut self) {
        if self.service {
            match scm::start() {
                Ok(()) => self.state = State::Starting,
                Err(code) => self.balloon("SoliDB cannot start", &scm::why(code)),
            }
            self.show(NIM_MODIFY);
            return;
        }
        if self.child.is_some() {
            return;
        }
        match server::spawn(&self.cfg, &self.home) {
            Ok(child) => {
                let _ = std::fs::write(self.pid_file(), child.pid.to_string());
                self.child = Some(child);
                self.stop_by = None;
                self.state = State::Starting;
            }
            Err(code) => {
                self.state = State::Failed(code);
                let why = if code == ERROR_FILE_NOT_FOUND {
                    format!(
                        "{} was not found. Set 'exe' in the settings.",
                        self.cfg.exe.display()
                    )
                } else {
                    format!("Windows error {code}.")
                };
                self.balloon("SoliDB cannot start", &why);
            }
        }
        self.show(NIM_MODIFY);
    }

    /// Asks the server to stop as Ctrl-C does, so that it closes its
    /// database: through the service when there is one. A process of the
    /// icon's own is ended only if it has not gone by `STOP_WITHIN`.
    unsafe fn stop(&mut self) {
        if self.service {
            match scm::stop() {
                Ok(()) => self.state = State::Stopping,
                Err(code) => {
                    self.restart = false;
                    self.balloon("SoliDB cannot stop", &scm::why(code));
                }
            }
            self.show(NIM_MODIFY);
            return;
        }
        let Some(child) = &self.child else { return };
        if self.stop_by.is_some() {
            return;
        }
        if server::ask_to_stop(child) {
            self.stop_by = Some(Instant::now() + STOP_WITHIN);
        } else {
            // No console to reach it through: nothing gentler is left.
            TerminateProcess(child.handle, 1);
            self.stop_by = Some(Instant::now());
        }
        self.state = State::Stopping;
        self.show(NIM_MODIFY);
    }

    /// One look at the service: what the service manager says of it, and
    /// whether the port answered.
    unsafe fn look_at_service(&mut self, answering: bool) {
        let before = self.state;
        // A service removed while the icon runs reads as stopped.
        let service = scm::status().unwrap_or(Service::Stopped(0));
        self.state = before.of_service(service, answering);
        if let (State::Failed(code), false) = (self.state, matches!(before, State::Failed(_))) {
            self.restart = false;
            self.balloon(
                "SoliDB stopped",
                &format!("The service ended: {}. See the log.", state::reason(code)),
            );
        }
        if self.state == State::Stopped && std::mem::take(&mut self.restart) {
            self.start();
        }
        if self.state != before {
            self.show(NIM_MODIFY);
        }
    }

    /// One look at the server: its process, and whether the port answered.
    unsafe fn look(&mut self, answering: bool) {
        if self.service {
            return self.look_at_service(answering);
        }
        let process = match &self.child {
            None => Process::None,
            Some(child) => {
                if WaitForSingleObject(child.handle, 0) == WAIT_TIMEOUT {
                    Process::Alive
                } else {
                    let mut code = 0u32;
                    GetExitCodeProcess(child.handle, &mut code);
                    Process::Exited(code)
                }
            }
        };
        let before = self.state;
        self.state = before.after(process, answering, self.stop_by.is_some());

        match process {
            Process::Exited(_) => {
                if let Some(child) = self.child.take() {
                    CloseHandle(child.handle);
                }
                let _ = std::fs::remove_file(self.pid_file());
                self.stop_by = None;
                if let State::Failed(code) = self.state {
                    self.balloon(
                        "SoliDB stopped",
                        &format!("The server ended: {}. See the log.", state::reason(code)),
                    );
                }
                if std::mem::take(&mut self.restart) && self.state == State::Stopped {
                    self.start();
                }
            }
            Process::Alive => {
                if let (Some(by), Some(child)) = (self.stop_by, &self.child) {
                    if Instant::now() >= by {
                        TerminateProcess(child.handle, 1);
                    }
                }
            }
            Process::None => {}
        }
        if self.state != before {
            self.show(NIM_MODIFY);
        }
    }

    fn console_log(&self) -> PathBuf {
        self.home.join("console.log")
    }

    /// "Open admin console": the address the settings give, else the
    /// console of this machine, started if it is not there, on a runtime
    /// fetched if there is none, after asking.
    unsafe fn open_console(&mut self) {
        if !self.cfg.console_url.is_empty() {
            return open(&self.cfg.console_url);
        }
        if self.console_busy {
            return;
        }
        // Started before, by this icon or one before it: nothing to start.
        if probe::answers_at(self.cfg.console_port(), "/health") {
            return open(&console::url(&self.cfg));
        }
        if !console::is_app(&self.cfg.console_app) {
            return self.balloon(
                "SoliDB admin console",
                &format!(
                    "No admin application in {}. Set 'console_app' or 'console_url' in the settings.",
                    self.cfg.console_app.display()
                ),
            );
        }
        if !console::needs_download(&self.cfg.soli) {
            return self.start_console();
        }
        let question = format!(
            "The admin console runs on the Soli runtime, which is not on this machine yet.\n\n\
             Download Soli {} ({} MB) from github.com/solisoft/soli_lang?\n\n\
             It is kept in {}.",
            console::SOLI_VERSION,
            console::SOLI_MEGABYTES,
            self.cfg.soli.parent().unwrap_or(&self.home).display()
        );
        let answer = MessageBoxW(
            self.hwnd,
            wide(&question).as_ptr(),
            wide("SoliDB admin console").as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND | MB_TOPMOST,
        );
        if answer != IDYES {
            return;
        }
        self.console_busy = true;
        let (soli, log, target) = (
            self.cfg.soli.clone(),
            self.console_log(),
            self.hwnd as isize,
        );
        std::thread::spawn(move || {
            let went = fetch(&soli, &log);
            PostMessageW(target as HWND, WM_CONSOLE, went, 0);
        });
    }

    /// Starts the console on a copy of the application in the icon's
    /// folder, then waits for it to answer on another thread.
    unsafe fn start_console(&mut self) {
        self.console_busy = false;
        if let Some(old) = self.console.take() {
            // One that ended, or never answered: it must not hold the port.
            TerminateProcess(old.handle, 1);
            CloseHandle(old.handle);
        }
        let copy = self.home.join("console");
        if let Err(error) = console::refresh(&self.cfg.console_app, &copy) {
            return self.balloon(
                "SoliDB admin console",
                &format!(
                    "Could not copy the application to {}: {error}.",
                    copy.display()
                ),
            );
        }
        let environment = console::environment(std::env::vars_os(), &console::settings(&self.cfg));
        let line = console::command_line(&self.cfg, &copy);
        match server::start(&line, &copy, &self.console_log(), Some(&environment)) {
            Ok(child) => self.console = Some(child),
            Err(code) => {
                return self.balloon(
                    "SoliDB admin console",
                    &format!(
                        "{} could not be started: Windows error {code}.",
                        self.cfg.soli.display()
                    ),
                );
            }
        }
        self.console_busy = true;
        let (port, target) = (self.cfg.console_port(), self.hwnd as isize);
        std::thread::spawn(move || {
            let until = Instant::now() + CONSOLE_WITHIN;
            let mut went = CONSOLE_SILENT;
            while Instant::now() < until {
                if probe::answers_at(port, "/health") {
                    went = CONSOLE_UP;
                    break;
                }
                std::thread::sleep(Duration::from_millis(400));
            }
            PostMessageW(target as HWND, WM_CONSOLE, went, 0);
        });
    }

    /// What a thread reported of the console's start.
    unsafe fn console_went(&mut self, went: usize) {
        self.console_busy = false;
        let log = self.console_log();
        let why = match went {
            CONSOLE_FETCHED => return self.start_console(),
            CONSOLE_UP => return open(&console::url(&self.cfg)),
            CONSOLE_NO_DOWNLOAD => format!(
                "Soli could not be downloaded (see {}). Without a connection, put soli.exe in {}.",
                log.display(),
                self.cfg.soli.parent().unwrap_or(&self.home).display()
            ),
            CONSOLE_WRONG_DIGEST => {
                "The file downloaded is not the Soli release expected: it was deleted, and nothing of it was run."
                    .to_string()
            }
            CONSOLE_NO_UNPACK => format!("Soli could not be unpacked (see {}).", log.display()),
            _ => format!(
                "The console did not answer on port {} (see {}).",
                self.cfg.console_port(),
                log.display()
            ),
        };
        self.balloon("SoliDB admin console", &why);
    }

    unsafe fn end_console(&mut self) {
        if let Some(child) = self.console.take() {
            TerminateProcess(child.handle, 0);
            CloseHandle(child.handle);
        }
    }

    unsafe fn command(&mut self, id: usize) {
        match id {
            CMD_START => self.start(),
            CMD_STOP => {
                self.restart = false;
                self.stop();
            }
            CMD_RESTART => {
                self.restart = true;
                self.stop();
            }
            CMD_CONSOLE => self.open_console(),
            CMD_DATA => open(&self.cfg.data_dir.to_string_lossy()),
            CMD_LOG => open(&self.cfg.log.to_string_lossy()),
            CMD_SETTINGS if self.service => open(&scm::settings_file().to_string_lossy()),
            CMD_SETTINGS => open(&self.home.join("tray.conf").to_string_lossy()),
            CMD_AUTOSTART => set_autostart(!autostart()),
            CMD_QUIT => {
                // The server is left running: the icon takes it back when
                // it is started again. The console is the icon's alone
                // and keeps nothing: it ends with it.
                self.end_console();
                self.show(NIM_DELETE);
                DestroyWindow(self.hwnd);
            }
            _ => {}
        }
    }

    unsafe fn menu(&mut self) {
        let menu = CreatePopupMenu();
        let add = |id: usize, text: &str, flags: u32| {
            AppendMenuW(menu, MF_STRING | flags, id, wide(text).as_ptr());
        };
        let off = |on: bool| if on { 0 } else { MF_GRAYED };

        add(0, &self.state.label(self.cfg.port), MF_GRAYED);
        if self.service {
            add(0, "Runs as a Windows service", MF_GRAYED);
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        add(CMD_START, "Start", off(self.state.can_start()));
        add(CMD_STOP, "Stop", off(self.state.can_stop()));
        add(CMD_RESTART, "Restart", off(self.state.can_stop()));
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        if self.console_busy {
            add(0, "Admin console is starting...", MF_GRAYED);
        } else {
            add(CMD_CONSOLE, "Open admin console", 0);
        }
        add(
            CMD_DATA,
            "Open data folder",
            off(self.cfg.data_dir.is_dir()),
        );
        add(CMD_LOG, "Open log", off(self.cfg.log.is_file()));
        add(CMD_SETTINGS, "Settings", 0);
        add(
            CMD_AUTOSTART,
            "Start with Windows",
            if autostart() { MF_CHECKED } else { 0 },
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        add(
            0,
            concat!("solidb-tray ", env!("CARGO_PKG_VERSION")),
            MF_GRAYED,
        );
        add(CMD_QUIT, "Quit", 0);

        let mut at = POINT { x: 0, y: 0 };
        GetCursorPos(&mut at);
        // Without this the menu does not close when the user clicks elsewhere.
        SetForegroundWindow(self.hwnd);
        TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            at.x,
            at.y,
            0,
            self.hwnd,
            null(),
        );
        PostMessageW(self.hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }

    fn icon(&self) -> HICON {
        match self.state {
            State::Stopped | State::Elsewhere => self.icons.stopped,
            State::Starting | State::Stopping | State::Silent => self.icons.busy,
            State::Running => self.icons.running,
            State::Failed(_) => self.icons.failed,
        }
    }

    unsafe fn data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = std::mem::zeroed();
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = 1;
        data
    }

    /// Adds, updates or removes the icon, with the state as its tooltip.
    unsafe fn show(&self, how: NOTIFY_ICON_MESSAGE) {
        let mut data = self.data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = self.icon();
        fill(&mut data.szTip, &self.state.label(self.cfg.port));
        Shell_NotifyIconW(how, &data);
    }

    unsafe fn balloon(&self, title: &str, text: &str) {
        let mut data = self.data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_WARNING;
        fill(&mut data.szInfoTitle, title);
        fill(&mut data.szInfo, text);
        Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// Fetches the Soli runtime to `soli`: downloads the release's archive,
/// holds it to its digest, and only then unpacks it. Gives a `CONSOLE_*`.
unsafe fn fetch(soli: &Path, log: &Path) -> usize {
    let Some(dir) = soli.parent() else {
        return CONSOLE_NO_DOWNLOAD;
    };
    let _ = std::fs::create_dir_all(dir);
    // The tools of Windows itself, by their full path: not whatever a
    // folder of the PATH calls curl or tar.
    let system = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
        .join("System32");
    let archive = dir.join("soli.tar.gz.part");
    let _ = std::fs::remove_file(&archive);
    if server::run(&console::download_line(&system, &archive), dir, log) != Ok(0) {
        let _ = std::fs::remove_file(&archive);
        return CONSOLE_NO_DOWNLOAD;
    }
    if !matches!(
        console::has_digest(&archive, console::SOLI_SHA256),
        Ok(true)
    ) {
        let _ = std::fs::remove_file(&archive);
        return CONSOLE_WRONG_DIGEST;
    }
    let _ = std::fs::remove_file(soli);
    let unpacked = server::run(&console::unpack_line(&system, &archive, dir), dir, log);
    let _ = std::fs::remove_file(&archive);
    if unpacked != Ok(0) || !soli.is_file() || console::mark_downloaded(soli).is_err() {
        return CONSOLE_NO_UNPACK;
    }
    CONSOLE_FETCHED
}

/// Opens a folder, a file or an address with whatever Windows opens it with.
unsafe fn open(what: &str) {
    ShellExecuteW(
        null_mut(),
        wide("open").as_ptr(),
        wide(what).as_ptr(),
        null(),
        null(),
        SW_SHOWNORMAL,
    );
}

/// Whether the icon is started with the user's session.
unsafe fn autostart() -> bool {
    RegGetValueW(
        HKEY_CURRENT_USER,
        wide(RUN_KEY).as_ptr(),
        wide(RUN_VALUE).as_ptr(),
        RRF_RT_REG_SZ,
        null_mut(),
        null_mut(),
        null_mut(),
    ) == ERROR_SUCCESS
}

unsafe fn set_autostart(on: bool) {
    if !on {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(RUN_VALUE).as_ptr(),
        );
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let value = wide(&format!("\"{}\"", exe.display()));
    RegSetKeyValueW(
        HKEY_CURRENT_USER,
        wide(RUN_KEY).as_ptr(),
        wide(RUN_VALUE).as_ptr(),
        REG_SZ,
        value.as_ptr() as *const c_void,
        (value.len() * 2) as u32,
    );
}

/// A disc of one colour (`0xRRGGBB`) on a transparent ground, drawn here so
/// that the program carries no resource file.
unsafe fn disc(rgb: u32) -> HICON {
    const SIZE: i32 = 32;
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    let centre = (SIZE as f32 - 1.0) / 2.0;
    let radius = SIZE as f32 / 2.0 - 2.0;
    let mut pixels = vec![0u32; (SIZE * SIZE) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = (x as f32 - centre, y as f32 - centre);
            let distance = (dx * dx + dy * dy).sqrt();
            // One pixel of soft edge; a lighter upper half gives it some volume.
            let cover = (radius + 0.5 - distance).clamp(0.0, 1.0);
            if cover == 0.0 {
                continue;
            }
            let light = 1.0 + 0.25 * (-dy / radius).clamp(-1.0, 1.0);
            let channel = |c: u32| ((c as f32 * light).min(255.0) * cover) as u32;
            let alpha = (cover * 255.0) as u32;
            // Stored as BGRA with the colour already multiplied by alpha.
            pixels[(y * SIZE + x) as usize] =
                alpha << 24 | channel(r) << 16 | channel(g) << 8 | channel(b);
        }
    }
    let colour = CreateBitmap(SIZE, SIZE, 1, 32, pixels.as_ptr() as *const c_void);
    let mask_bits = vec![0u8; (SIZE * SIZE / 8) as usize];
    let mask = CreateBitmap(SIZE, SIZE, 1, 1, mask_bits.as_ptr() as *const c_void);
    let info = ICONINFO {
        fIcon: 1,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: colour,
    };
    let icon = CreateIconIndirect(&info);
    DeleteObject(colour);
    DeleteObject(mask);
    icon
}

/// Copies `text` into a fixed field, cut to fit, always zero-terminated.
fn fill(field: &mut [u16], text: &str) {
    let room = field.len() - 1;
    let mut at = 0;
    for unit in text.encode_utf16().take(room) {
        field[at] = unit;
        at += 1;
    }
    field[at] = 0;
}
