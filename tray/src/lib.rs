//! What `solidb-tray` (the notification-area icon) and `solidb-service`
//! (the Windows service) share. The server stays the console program it
//! is; these start `solidb.exe` without a window, send its output to a log
//! file and ask it to stop the way Ctrl-C does.

// The parts that are not Win32 are compiled, and tested, on any system.
#![cfg_attr(not(windows), allow(dead_code))]

pub mod config;
pub mod console;
pub mod probe;
pub mod state;

#[cfg(windows)]
pub mod scm;
#[cfg(windows)]
pub mod server;
