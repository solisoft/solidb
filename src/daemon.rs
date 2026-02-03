use std::fs::File;
use std::io;
use std::os::unix::io::AsRawFd;

#[derive(Debug)]
pub struct DaemonizeError(String);

impl std::fmt::Display for DaemonizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DaemonizeError {}

pub struct Daemonize {
    pid_file: Option<String>,
    working_directory: Option<String>,
    stdout: Option<File>,
    stderr: Option<File>,
}

impl Daemonize {
    pub fn new() -> Self {
        Self {
            pid_file: None,
            working_directory: None,
            stdout: None,
            stderr: None,
        }
    }

    pub fn pid_file(mut self, path: &str) -> Self {
        self.pid_file = Some(path.to_string());
        self
    }

    pub fn working_directory(mut self, path: &str) -> Self {
        self.working_directory = Some(path.to_string());
        self
    }

    pub fn stdout(mut self, file: File) -> Self {
        self.stdout = Some(file);
        self
    }

    pub fn stderr(mut self, file: File) -> Self {
        self.stderr = Some(file);
        self
    }

    pub fn start(self) -> Result<(), DaemonizeError> {
        unsafe {
            // First fork - detach from terminal
            match libc::fork() {
                -1 => {
                    return Err(DaemonizeError(format!(
                        "First fork failed: {}",
                        io::Error::last_os_error()
                    )))
                }
                0 => {
                    // Child process continues
                }
                _ => {
                    // Parent process exits
                    std::process::exit(0);
                }
            }

            // Create new session
            if libc::setsid() == -1 {
                return Err(DaemonizeError(format!(
                    "setsid failed: {}",
                    io::Error::last_os_error()
                )));
            }

            // Second fork to prevent reacquiring terminal
            match libc::fork() {
                -1 => {
                    return Err(DaemonizeError(format!(
                        "Second fork failed: {}",
                        io::Error::last_os_error()
                    )))
                }
                0 => {
                    // Grandchild process continues
                }
                _ => {
                    // Child process exits
                    std::process::exit(0);
                }
            }

            // Set umask
            libc::umask(0o022);

            // Change working directory
            if let Some(ref dir) = self.working_directory {
                let dir_cstring = std::ffi::CString::new(dir.as_bytes())
                    .map_err(|e| DaemonizeError(format!("Invalid working directory: {}", e)))?;
                if libc::chdir(dir_cstring.as_ptr()) == -1 {
                    return Err(DaemonizeError(format!(
                        "chdir failed: {}",
                        io::Error::last_os_error()
                    )));
                }
            }

            // Redirect stdout
            if let Some(ref file) = self.stdout {
                let fd = file.as_raw_fd();
                if libc::dup2(fd, libc::STDOUT_FILENO) == -1 {
                    return Err(DaemonizeError(format!(
                        "dup2 stdout failed: {}",
                        io::Error::last_os_error()
                    )));
                }
            }

            // Redirect stderr
            if let Some(ref file) = self.stderr {
                let fd = file.as_raw_fd();
                if libc::dup2(fd, libc::STDERR_FILENO) == -1 {
                    return Err(DaemonizeError(format!(
                        "dup2 stderr failed: {}",
                        io::Error::last_os_error()
                    )));
                }
            }

            // Write PID file
            if let Some(ref pid_file) = self.pid_file {
                let pid = std::process::id();
                std::fs::write(pid_file, format!("{}\n", pid))
                    .map_err(|e| DaemonizeError(format!("Failed to write PID file: {}", e)))?;
            }
        }

        Ok(())
    }
}

impl Default for Daemonize {
    fn default() -> Self {
        Self::new()
    }
}
