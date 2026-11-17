//! Everything that talks to Windows.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::config::Config;
use crate::probe;
use crate::state::{Process, State};

/// The notification area tells the window about the mouse with this.
const WM_TRAY: u32 = WM_APP + 1;
/// The probing thread tells the window what it found: `wparam` is 1 when
/// the server answered.
const WM_PROBED: u32 = WM_APP + 2;

const CMD_START: usize = 1;
const CMD_STOP: usize = 2;
const CMD_RESTART: usize = 3;
const CMD_CONSOLE: usize = 4;
const CMD_DATA: usize = 5;
const CMD_LOG: usize = 6;
const CMD_SETTINGS: usize = 7;
const CMD_AUTOSTART: usize = 8;
const CMD_QUIT: usize = 9;

/// How long a server asked to stop is given before it is ended.
const STOP_WITHIN: Duration = Duration::from_secs(60);
const PROBE_EVERY: Duration = Duration::from_millis(1500);

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "SoliDB";

struct Child {
    handle: HANDLE,
    pid: u32,
}

struct App {
    cfg: Config,
    home: PathBuf,
    hwnd: HWND,
    child: Option<Child>,
    state: State,
    /// Set when the server was asked to stop: when it is ended if still there.
    stop_by: Option<Instant>,
    /// Start again once the server asked to stop is gone.
    restart: bool,
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
        // The server colours its output for a terminal; its output goes to
        // a file here. Set before any thread exists, inherited by the server.
        std::env::set_var("NO_COLOR", "1");
        let (cfg, refused) = settings(&home, &beside);

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
            hwnd,
            child: None,
            state: State::Stopped,
            stop_by: None,
            restart: false,
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
        if app.cfg.start_server && app.state.can_start() {
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
        WM_COMMAND => {
            app.command(wparam & 0xFFFF);
            0
        }
        // Windows is closing the session: leave the server to it.
        WM_ENDSESSION | WM_CLOSE => {
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

    /// Starts the server without a window, its output appended to the log.
    unsafe fn start(&mut self) {
        if self.child.is_some() {
            return;
        }
        if !self.cfg.exe.is_file() {
            self.state = State::Failed(ERROR_FILE_NOT_FOUND);
            self.show(NIM_MODIFY);
            self.balloon(
                "SoliDB cannot start",
                &format!(
                    "{} was not found. Set 'exe' in the settings.",
                    self.cfg.exe.display()
                ),
            );
            return;
        }
        let _ = std::fs::create_dir_all(&self.cfg.data_dir);
        if let Some(dir) = self.cfg.log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }

        let inherit = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        let log = CreateFileW(
            wide(&self.cfg.log.to_string_lossy()).as_ptr(),
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
        let mut line = wide(&self.cfg.command_line());
        // The server reads a `.env` where it is started: start it in the
        // icon's own folder, not wherever the icon was launched from.
        let cwd = wide(&self.home.to_string_lossy());
        // A console without a window: the server needs one to be sent Ctrl-C.
        let made = CreateProcessW(
            null(),
            line.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_NO_WINDOW,
            null(),
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
            self.state = State::Failed(error);
            self.show(NIM_MODIFY);
            self.balloon("SoliDB cannot start", &format!("Windows error {error}."));
            return;
        }
        CloseHandle(pi.hThread);
        let _ = std::fs::write(self.pid_file(), pi.dwProcessId.to_string());
        self.child = Some(Child {
            handle: pi.hProcess,
            pid: pi.dwProcessId,
        });
        self.stop_by = None;
        self.state = State::Starting;
        self.show(NIM_MODIFY);
    }

    /// Asks the server to stop as Ctrl-C does, so that it closes its
    /// database. It is ended only if it has not gone by `STOP_WITHIN`.
    unsafe fn stop(&mut self) {
        let Some(child) = &self.child else { return };
        if self.stop_by.is_some() {
            return;
        }
        // Ctrl-C goes to every process of a console. This program has none;
        // it joins the server's for the time of sending, with its own
        // handling turned off so that it does not end itself.
        FreeConsole();
        if AttachConsole(child.pid) != 0 {
            SetConsoleCtrlHandler(None, 1);
            GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
            // The event is delivered on another thread: leave the console
            // only once it has had the time to be ignored here.
            std::thread::sleep(Duration::from_millis(200));
            FreeConsole();
            SetConsoleCtrlHandler(None, 0);
            self.stop_by = Some(Instant::now() + STOP_WITHIN);
        } else {
            // No console to reach it through: nothing gentler is left.
            TerminateProcess(child.handle, 1);
            self.stop_by = Some(Instant::now());
        }
        self.state = State::Stopping;
        self.show(NIM_MODIFY);
    }

    /// One look at the server: its process, and whether the port answered.
    unsafe fn look(&mut self, answering: bool) {
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
                        &format!(
                            "The server ended: {}. See the log.",
                            crate::state::reason(code)
                        ),
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
            CMD_CONSOLE => open(&self.cfg.console_url),
            CMD_DATA => open(&self.cfg.data_dir.to_string_lossy()),
            CMD_LOG => open(&self.cfg.log.to_string_lossy()),
            CMD_SETTINGS => open(&self.home.join("tray.conf").to_string_lossy()),
            CMD_AUTOSTART => set_autostart(!autostart()),
            CMD_QUIT => {
                // The server is left running: the icon takes it back when
                // it is started again.
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
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        add(CMD_START, "Start", off(self.state.can_start()));
        add(CMD_STOP, "Stop", off(self.state.can_stop()));
        add(CMD_RESTART, "Restart", off(self.state.can_stop()));
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        if !self.cfg.console_url.is_empty() {
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

/// A string as Windows takes it: UTF-16 with a closing zero.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
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
