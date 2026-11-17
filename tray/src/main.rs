//! `solidb-tray`: an icon in the Windows notification area that starts,
//! stops and watches a SoliDB server on this machine: the Windows service
//! when it is installed (`solidb-service install`), a server of its own in
//! the user's session otherwise.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::run();
    #[cfg(not(windows))]
    {
        eprintln!("solidb-tray runs on Windows only.");
        std::process::exit(2);
    }
}
