//! `solidb-tray`: an icon in the Windows notification area that starts,
//! stops and watches a SoliDB server on this machine.
//!
//! The server stays the console program it is; this is a separate, small
//! executable. It starts `solidb.exe` without a window, sends its output to
//! a log file, asks it to stop the way Ctrl-C does, and shows by its colour
//! whether the server answers.

#![cfg_attr(windows, windows_subsystem = "windows")]
// The parts that are not Win32 are compiled, and tested, on any system.
#![cfg_attr(not(windows), allow(dead_code))]

mod config;
mod probe;
mod state;
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
